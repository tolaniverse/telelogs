//! Talks to telelog-server on a dedicated tokio runtime and forwards events to the UI.
//!
//! GPUI runs its own executor, and tonic needs tokio, so the two meet over a channel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

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
    /// Answer to [`Client::query_archive`] number `request`.
    Archive {
        request: u64,
        result: Result<Vec<LogRecord>, String>,
    },
    /// Answer to [`Client::fetch_storage`].
    Storage(Result<v1::StorageInfo, String>),
}

/// Most archived lines fetched for one time range.
pub const ARCHIVE_LIMIT: u32 = 20_000;

/// Handle for one-off requests and for cutting the reconnect delay short.
#[derive(Clone)]
pub struct Client {
    retry: Arc<Notify>,
    connection: Connection,
    runtime: tokio::runtime::Handle,
    tx: UnboundedSender<ClientEvent>,
}

impl Client {
    pub fn retry_now(&self) {
        self.retry.notify_one();
    }

    /// Asks the server's archive for lines in `[from, to]`; the answer arrives as
    /// [`ClientEvent::Archive`] with the same `request` number.
    /// Archived lines in `from..=to`, except each origin's lines from its `skip_from` time on,
    /// which are already in memory.
    pub fn query_archive(
        &self,
        request: u64,
        from: Option<SystemTime>,
        to: SystemTime,
        skip_from: HashMap<String, SystemTime>,
    ) {
        let (connection, tx) = (self.connection.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let result = async {
                let mut client = connect(&connection).await?;
                let mut stream = client
                    .query(v1::QueryRequest {
                        from: from.map(telelog_proto::to_timestamp),
                        to: Some(telelog_proto::to_timestamp(to)),
                        limit: ARCHIVE_LIMIT,
                        skip_from: skip_from
                            .into_iter()
                            .map(|(origin, t)| (origin, telelog_proto::to_timestamp(t)))
                            .collect(),
                        ..Default::default()
                    })
                    .await?
                    .into_inner();
                let mut records = Vec::new();
                while let Some(record) = stream.next().await {
                    records.push(record?.into());
                }
                anyhow::Ok(records)
            }
            .await
            .map_err(|e| describe_request(&e));
            let _ = tx.unbounded_send(ClientEvent::Archive { request, result });
        });
    }

    /// Fetches bucket stats; the answer arrives as [`ClientEvent::Storage`].
    pub fn fetch_storage(&self) {
        let (connection, tx) = (self.connection.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let result = async {
                let mut client = connect(&connection).await?;
                anyhow::Ok(client.get_storage(v1::GetStorageRequest {}).await?.into_inner())
            }
            .await
            .map_err(|e| describe_request(&e));
            let _ = tx.unbounded_send(ClientEvent::Storage(result));
        });
    }
}

/// For one-off requests, the server's own message reads best ("no archive bucket configured").
fn describe_request(e: &anyhow::Error) -> String {
    match e.downcast_ref::<tonic::Status>() {
        Some(status) if status.code() == tonic::Code::Unimplemented => {
            "this server is too old to support this; update telelog-server".into()
        }
        Some(status) if status.code() != tonic::Code::Unauthenticated => status.message().to_string(),
        _ => describe(e),
    }
}

async fn connect(connection: &Connection) -> anyhow::Result<telelog_proto::auth::Client> {
    telelog_proto::auth::connect(
        &connection.server,
        connection.token.as_deref(),
        connection.ca_pem.as_deref(),
    )
    .await
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
    let client = Client {
        retry: retry.clone(),
        connection: connection.clone(),
        runtime: runtime.clone(),
        tx: tx.clone(),
    };
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
    let mut client = connect(connection).await?;

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
