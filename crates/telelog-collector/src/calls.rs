//! Answers one app call the gateway forwarded: Reply frames from the sources, then a Close.

use futures::StreamExt;
use prost::Message;
use telelog_proto::cloud::v1::collector_frame::Frame;
use telelog_proto::cloud::v1::{Close, CollectorFrame, LogRecords, Method, Open, Reply};
use telelog_proto::v1;
use telelog_sources::Sources;
use tokio::sync::mpsc::Sender;
use tonic::Status;

use crate::frame;

/// Most log lines in one Reply. Lines that are ready go out together; none wait to fill one.
const BATCH: usize = 256;

pub(crate) async fn answer(sources: &Sources, open: Open, out: &Sender<CollectorFrame>) {
    let (code, message) = match respond(sources, &open, out).await {
        Ok(()) => (0, String::new()),
        Err(status) => (status.code() as i32, status.message().to_string()),
    };
    let close = Close {
        call: open.call,
        code,
        message,
    };
    let _ = out.send(frame(Frame::Close(close))).await;
}

async fn respond(sources: &Sources, open: &Open, out: &Sender<CollectorFrame>) -> Result<(), Status> {
    let reply = |payload: Vec<u8>| {
        frame(Frame::Reply(Reply {
            call: open.call,
            payload,
        }))
    };
    match open.method() {
        Method::ListTargets => {
            let request: v1::ListTargetsRequest = decode(&open.payload)?;
            let targets = sources.list_targets(request.all).await.map_err(unavailable)?;
            let response = v1::ListTargetsResponse {
                targets: targets.into_iter().map(Into::into).collect(),
            };
            send(out, reply(response.encode_to_vec())).await
        }
        Method::WatchTargets => {
            let _: v1::WatchTargetsRequest = decode(&open.payload)?;
            let mut changes = sources.watch_targets();
            while let Some(target) = changes.next().await {
                let target = v1::Target::from(target.map_err(unavailable)?);
                send(out, reply(target.encode_to_vec())).await?;
            }
            Ok(())
        }
        Method::Tail => {
            let request: v1::TailRequest = decode(&open.payload)?;
            let records = sources
                .tail(&request.target_ids, request.backlog)
                .await
                .map_err(unavailable)?
                .ok_or_else(|| Status::not_found("no running targets match the request"))?;
            let mut batches = records.ready_chunks(BATCH);
            while let Some(batch) = batches.next().await {
                let records = batch
                    .into_iter()
                    .map(|record| record.map(v1::LogRecord::from))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| Status::internal(format!("{e:#}")))?;
                send(out, reply(LogRecords { records }.encode_to_vec())).await?;
            }
            Ok(())
        }
        Method::Unspecified => Err(Status::unimplemented(format!(
            "this collector doesn't answer method {}; update it",
            open.method
        ))),
    }
}

fn decode<M: Message + Default>(payload: &[u8]) -> Result<M, Status> {
    M::decode(payload).map_err(|e| Status::invalid_argument(format!("decoding the request: {e}")))
}

fn unavailable(e: anyhow::Error) -> Status {
    Status::unavailable(format!("{e:#}"))
}

async fn send(out: &Sender<CollectorFrame>, frame: CollectorFrame) -> Result<(), Status> {
    out.send(frame)
        .await
        .map_err(|_| Status::cancelled("the gateway connection closed"))
}
