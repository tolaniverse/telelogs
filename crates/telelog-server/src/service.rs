use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use telelog_core::Filter;
use telelog_proto::{LogService, from_timestamp, to_timestamp, v1};
use telelog_sources::DockerSource;
use tonic::{Request, Response, Status};

use crate::archive::Archive;

/// Max backlog lines per target, so a client can't request unbounded history.
const MAX_BACKLOG: u32 = 10_000;
const DEFAULT_QUERY_LIMIT: u32 = 5_000;
const MAX_QUERY_LIMIT: u32 = 50_000;

pub struct Logs {
    docker: DockerSource,
    archive: Option<Arc<Archive>>,
}

impl Logs {
    pub fn new(docker: DockerSource, archive: Option<Arc<Archive>>) -> Self {
        Logs { docker, archive }
    }
}

fn query_filter(request: &v1::QueryRequest) -> Filter {
    let mut filter = Filter::new(&request.text);
    let level = v1::Level::try_from(request.min_level).unwrap_or(v1::Level::Unknown);
    if level != v1::Level::Unknown {
        filter.min_level = Some(level.into());
    }
    filter.from = request.from.map(|t| from_timestamp(Some(t)));
    filter.to = request.to.map(|t| from_timestamp(Some(t)));
    filter.skip_from = request
        .skip_from
        .iter()
        .map(|(origin, t)| (origin.clone(), from_timestamp(Some(*t))))
        .collect();
    filter
}

type RecordStream = Pin<Box<dyn Stream<Item = Result<v1::LogRecord, Status>> + Send>>;
type TargetStream = Pin<Box<dyn Stream<Item = Result<v1::Target, Status>> + Send>>;

#[tonic::async_trait]
impl LogService for Logs {
    async fn list_targets(
        &self,
        request: Request<v1::ListTargetsRequest>,
    ) -> Result<Response<v1::ListTargetsResponse>, Status> {
        let targets = self
            .docker
            .list_targets(request.into_inner().all)
            .await
            .map_err(|e| Status::unavailable(format!("{e:#}")))?;
        Ok(Response::new(v1::ListTargetsResponse {
            targets: targets.into_iter().map(Into::into).collect(),
        }))
    }

    type TailStream = RecordStream;

    async fn tail(&self, request: Request<v1::TailRequest>) -> Result<Response<RecordStream>, Status> {
        let request = request.into_inner();
        let backlog = request.backlog.min(MAX_BACKLOG);

        // No explicit targets means "everything": follow new containers as they start too.
        let records = if request.target_ids.is_empty() {
            tracing::info!("live tail started");
            self.docker
                .tail_live(telelog_sources::LiveStart::Backlog(backlog))
                .await
                .map_err(|e| Status::unavailable(format!("{e:#}")))?
        } else {
            let targets: Vec<_> = self
                .docker
                .list_targets(false)
                .await
                .map_err(|e| Status::unavailable(format!("{e:#}")))?
                .into_iter()
                .filter(|t| request.target_ids.contains(&t.id))
                .collect();
            if targets.is_empty() {
                return Err(Status::not_found("no running targets match the request"));
            }
            tracing::info!(count = targets.len(), "tail started");
            self.docker.tail_many(&targets, backlog)
        };

        let stream = records.map(|record| {
            record
                .map(v1::LogRecord::from)
                .map_err(|e| Status::internal(format!("{e:#}")))
        });
        Ok(Response::new(Box::pin(stream)))
    }

    type WatchTargetsStream = TargetStream;

    async fn watch_targets(
        &self,
        _request: Request<v1::WatchTargetsRequest>,
    ) -> Result<Response<TargetStream>, Status> {
        let stream = self.docker.watch_targets().map(|target| {
            target
                .map(v1::Target::from)
                .map_err(|e| Status::unavailable(format!("{e:#}")))
        });
        Ok(Response::new(Box::pin(stream)))
    }

    type QueryStream = RecordStream;

    async fn query(&self, request: Request<v1::QueryRequest>) -> Result<Response<RecordStream>, Status> {
        let Some(archive) = &self.archive else {
            return Err(Status::failed_precondition(
                "this server has no archive bucket configured",
            ));
        };
        let request = request.into_inner();
        let limit = match request.limit {
            0 => DEFAULT_QUERY_LIMIT,
            n => n.min(MAX_QUERY_LIMIT),
        };
        let records = archive
            .query(&query_filter(&request), limit as usize)
            .await
            .map_err(|e| Status::unavailable(format!("{e:#}")))?;
        let stream = futures::stream::iter(records.into_iter().map(|r| Ok(v1::LogRecord::from(r))));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn get_storage(&self, _request: Request<v1::GetStorageRequest>) -> Result<Response<v1::StorageInfo>, Status> {
        let Some(archive) = &self.archive else {
            return Ok(Response::new(v1::StorageInfo::default()));
        };
        let stats = archive.stats().await;
        let status = archive.status();
        let (stats, stats_error) = match stats {
            Ok(stats) => (stats, None),
            Err(e) => (Default::default(), Some(format!("{e:#}"))),
        };
        Ok(Response::new(v1::StorageInfo {
            enabled: true,
            location: archive.location.clone(),
            provider: archive.provider.into(),
            objects: stats.objects,
            bytes: stats.bytes,
            truncated: stats.truncated,
            oldest: stats.oldest.map(to_timestamp),
            last_flush: status.last_flush.map(to_timestamp),
            last_error: stats_error.or(status.last_error).unwrap_or_default(),
            lines_archived: status.lines_archived,
            retention_days: archive.retention_days,
            flush_seconds: archive.flush_every.as_secs() as u32,
        }))
    }
}
