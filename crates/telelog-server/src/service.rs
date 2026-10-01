use std::pin::Pin;

use futures::{Stream, StreamExt};
use telelog_proto::{LogService, v1};
use telelog_sources::DockerSource;
use tonic::{Request, Response, Status};

/// Max backlog lines per target, so a client can't request unbounded history.
const MAX_BACKLOG: u32 = 10_000;

pub struct Logs {
    docker: DockerSource,
}

impl Logs {
    pub fn new(docker: DockerSource) -> Self {
        Logs { docker }
    }
}

type RecordStream = Pin<Box<dyn Stream<Item = Result<v1::LogRecord, Status>> + Send>>;

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
            self.docker.tail_live(backlog).await.map_err(|e| Status::unavailable(format!("{e:#}")))?
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
}
