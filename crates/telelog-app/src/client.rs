//! Talks to telelog-server on a dedicated tokio runtime and forwards events to the UI.
//!
//! GPUI runs its own executor, and tonic needs tokio, so the two meet over a channel.

use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::StreamExt;
use telelog_core::{LogRecord, Target};
use telelog_proto::{LogServiceClient, v1};

/// Lines of history to request per target when (re)connecting.
const BACKLOG: u32 = 500;
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub enum ClientEvent {
    Connected { targets: Vec<Target> },
    Record(LogRecord),
    Disconnected { reason: String },
}

/// Starts tailing every running target on `server`, reconnecting until the receiver is dropped.
pub fn spawn_tail(runtime: &tokio::runtime::Handle, server: String) -> UnboundedReceiver<ClientEvent> {
    let (tx, rx) = unbounded();
    runtime.spawn(async move {
        while !tx.is_closed() {
            let reason = match tail_once(&server, &tx).await {
                Ok(()) => "stream ended".to_string(),
                Err(e) => format!("{e:#}"),
            };
            if tx.unbounded_send(ClientEvent::Disconnected { reason }).is_err() {
                break;
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
    rx
}

async fn tail_once(server: &str, tx: &UnboundedSender<ClientEvent>) -> anyhow::Result<()> {
    let mut client = LogServiceClient::connect(server.to_string()).await?;

    let targets: Vec<Target> = client
        .list_targets(v1::ListTargetsRequest { all: false })
        .await?
        .into_inner()
        .targets
        .into_iter()
        .map(Into::into)
        .collect();
    tx.unbounded_send(ClientEvent::Connected { targets })?;

    let mut stream = client
        .tail(v1::TailRequest { target_ids: Vec::new(), backlog: BACKLOG })
        .await?
        .into_inner();
    while let Some(record) = stream.next().await {
        tx.unbounded_send(ClientEvent::Record(record?.into()))?;
    }
    Ok(())
}
