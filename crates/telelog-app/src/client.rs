//! Talks to telelog-server on a dedicated tokio runtime and forwards events to the UI.
//!
//! GPUI runs its own executor, and tonic needs tokio, so the two meet over a channel.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use telelog_core::{LogRecord, Target};
use telelog_proto::v1;
use tokio::sync::Notify;

/// Lines of history to request per target when (re)connecting.
pub const BACKLOG: u32 = 500;
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub enum ClientEvent {
    Connecting,
    Connected {
        targets: Vec<Target>,
    },
    Record(LogRecord),
    /// A target started, stopped or was removed; `state` holds its new state.
    TargetChanged(Target),
    Disconnected {
        reason: String,
    },
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

/// Where and how to reach telelog-server.
#[derive(Clone)]
pub struct Connection {
    pub server: String,
    pub token: Option<String>,
    /// PEM CA certificate to trust for an `https://` server with a self-signed certificate.
    pub ca_pem: Option<Vec<u8>>,
}

/// Tails every running target on the server, reconnecting until the receiver is dropped.
pub fn spawn(runtime: &tokio::runtime::Handle, connection: Connection) -> (Client, UnboundedReceiver<ClientEvent>) {
    let (tx, rx) = unbounded();
    let retry = Arc::new(Notify::new());
    let client = Client { retry: retry.clone() };
    runtime.spawn(async move {
        while !tx.is_closed() {
            let _ = tx.unbounded_send(ClientEvent::Connecting);
            let reason = match tail_once(&connection, &tx).await {
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
    if let Some(status) = e.downcast_ref::<tonic::Status>()
        && status.code() == tonic::Code::Unauthenticated
    {
        return "invalid or missing token (set TELELOG_TOKEN)".into();
    }
    let root = e.root_cause().to_string();
    let lower = root.to_lowercase();
    if lower.contains("connection refused") {
        "connection refused".into()
    } else {
        root
    }
}

async fn tail_once(connection: &Connection, tx: &UnboundedSender<ClientEvent>) -> anyhow::Result<()> {
    let mut client = telelog_proto::auth::connect(
        &connection.server,
        connection.token.as_deref(),
        connection.ca_pem.as_deref(),
    )
    .await?;

    // Subscribe to state changes before listing, so none fall in the gap. Replaying an event
    // that the listing already reflects is harmless: each one carries the final state.
    // Servers older than WatchTargets answer Unimplemented; keep tailing without live state.
    let changes = match client.watch_targets(v1::WatchTargetsRequest {}).await {
        Ok(response) => response
            .into_inner()
            .map(|target| target.map(|t| ClientEvent::TargetChanged(t.into())))
            .boxed(),
        Err(status) if status.code() == tonic::Code::Unimplemented => futures::stream::pending().boxed(),
        Err(status) => return Err(status.into()),
    };

    let targets: Vec<Target> = client
        .list_targets(v1::ListTargetsRequest { all: true })
        .await?
        .into_inner()
        .targets
        .into_iter()
        .map(Into::into)
        .collect();
    tx.unbounded_send(ClientEvent::Connected { targets })?;

    let records = client
        .tail(v1::TailRequest {
            target_ids: Vec::new(),
            backlog: BACKLOG,
        })
        .await?
        .into_inner()
        .map(|record| record.map(|r| ClientEvent::Record(r.into())));

    // The log stream ending means the connection is over, whatever the watch stream is doing.
    let mut events = std::pin::pin!(futures::stream::select(
        records.map(Some).chain(futures::stream::once(async { None })),
        changes.map(Some),
    ));
    while let Some(Some(event)) = events.next().await {
        tx.unbounded_send(event?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn explains_auth_failures() {
        let error = anyhow::Error::from(tonic::Status::unauthenticated("invalid or missing token"));
        assert_eq!(describe(&error), "invalid or missing token (set TELELOG_TOKEN)");
    }
}
