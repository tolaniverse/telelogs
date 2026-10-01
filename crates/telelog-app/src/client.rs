//! Talks to telelog-server on a dedicated tokio runtime and forwards events to the UI.
//!
//! GPUI runs its own executor, and tonic needs tokio, so the two meet over a channel.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use telelog_core::{LogRecord, Target};
use telelog_proto::{LogServiceClient, v1};
use tokio::sync::Notify;

/// Lines of history to request per target when (re)connecting.
pub const BACKLOG: u32 = 500;
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub enum ClientEvent {
    Connecting,
    Connected { targets: Vec<Target> },
    Record(LogRecord),
    Disconnected { reason: String },
}

/// Lets the UI cut the reconnect delay short.
#[derive(Clone)]
pub struct Client {
    retry: Arc<Notify>,
}

impl Client {
    pub fn retry_now(&self) {
        self.retry.notify_one();
    }
}

/// Tails every running target on `server`, reconnecting until the receiver is dropped.
pub fn spawn(runtime: &tokio::runtime::Handle, server: String) -> (Client, UnboundedReceiver<ClientEvent>) {
    let (tx, rx) = unbounded();
    let retry = Arc::new(Notify::new());
    let client = Client { retry: retry.clone() };
    runtime.spawn(async move {
        while !tx.is_closed() {
            let _ = tx.unbounded_send(ClientEvent::Connecting);
            let reason = match tail_once(&server, &tx).await {
                Ok(()) => "stream ended".to_string(),
                Err(e) => describe(&e),
            };
            if tx.unbounded_send(ClientEvent::Disconnected { reason }).is_err() {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                _ = retry.notified() => {}
            }
        }
    });
    (client, rx)
}

/// The innermost cause reads best in the UI ("connection refused" rather than a transport chain).
fn describe(e: &anyhow::Error) -> String {
    let root = e.root_cause().to_string();
    let lower = root.to_lowercase();
    if lower.contains("connection refused") {
        "connection refused".into()
    } else {
        root
    }
}

async fn tail_once(server: &str, tx: &UnboundedSender<ClientEvent>) -> anyhow::Result<()> {
    let mut client = LogServiceClient::connect(server.to_string()).await?;

    let targets: Vec<Target> = client
        .list_targets(v1::ListTargetsRequest { all: true })
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
