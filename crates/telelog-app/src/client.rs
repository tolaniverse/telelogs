//! Talks to telelog-server, or to a collector through Telelog Cloud, on a dedicated tokio
//! runtime and forwards events to the UI.
//!
//! GPUI runs its own executor, and tonic needs tokio, so the two meet over a channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use telelog_core::{LogRecord, Target};
use telelog_proto::cloud::v1::Collector;
use telelog_proto::v1;
use tokio::sync::{Notify, watch};

use crate::remote;

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

/// Handle for one-off requests and for cutting the reconnect delay short. Dropping the last
/// handle disconnects.
#[derive(Clone)]
pub struct Client {
    retry: Arc<Notify>,
    /// Never sent on; the tail loop stops when it's dropped.
    _alive: Arc<watch::Sender<()>>,
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
                let mut remote = remote::connect(&connection).await?;
                let mut stream = remote
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
                    .await?;
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
                let mut remote = remote::connect(&connection).await?;
                anyhow::Ok(remote.get_storage().await?)
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

/// Where and how to reach a server.
#[derive(Clone)]
pub struct Connection {
    /// telelog-server, or Telelog Cloud's gateway when `collector` is set.
    pub server: String,
    pub token: Token,
    /// PEM CA certificate to trust for an `https://` server with a self-signed certificate.
    pub ca_cert: Option<PathBuf>,
    /// A collector to reach through the gateway's relay.
    pub collector: Option<String>,
}

/// A token, or where in the Keychain to find one.
///
/// A Keychain read can stop to ask the user for permission, so it happens when connecting, off
/// the UI thread, and once per connection: a reconnect doesn't ask again.
#[derive(Clone)]
pub enum Token {
    None,
    Given(String),
    Keychain {
        id: String,
        read: Arc<std::sync::OnceLock<Option<String>>>,
    },
}

impl Token {
    pub fn given(token: Option<String>) -> Self {
        token.map_or(Token::None, Token::Given)
    }

    pub fn keychain(id: &str) -> Self {
        Token::Keychain {
            id: id.to_string(),
            read: Arc::default(),
        }
    }

    /// The token as typed or given on the command line; None for one in the Keychain.
    pub fn as_given(&self) -> Option<&str> {
        match self {
            Token::Given(token) => Some(token),
            Token::None | Token::Keychain { .. } => None,
        }
    }

    /// Reads it, blocking on the Keychain the first time; call it off the UI thread.
    pub fn resolve(&self) -> Option<String> {
        match self {
            Token::None => None,
            Token::Given(token) => Some(token.clone()),
            Token::Keychain { id, read } => read.get_or_init(|| crate::servers::token(id)).clone(),
        }
    }
}

/// Longest the dialogs wait for a test connection.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `request` once on the runtime with a time limit; the answer arrives on the receiver.
fn once<T: Send + 'static>(
    runtime: &tokio::runtime::Handle,
    request: impl Future<Output = anyhow::Result<T>> + Send + 'static,
) -> oneshot::Receiver<Result<T, String>> {
    let (tx, rx) = oneshot::channel();
    runtime.spawn(async move {
        let result = tokio::time::timeout(PROBE_TIMEOUT, request)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("no answer after {} seconds", PROBE_TIMEOUT.as_secs())))
            .map_err(|e| describe(&e));
        let _ = tx.send(result);
    });
    rx
}

/// Connects once and counts the server's targets, to test a server before it's saved.
pub fn probe(runtime: &tokio::runtime::Handle, connection: Connection) -> oneshot::Receiver<Result<usize, String>> {
    once(runtime, async move {
        let mut remote = remote::connect(&connection).await?;
        anyhow::Ok(remote.list_targets(true).await?.len())
    })
}

/// Signs in to Telelog Cloud once and counts the workspace's collectors, to test the sign-in.
pub fn probe_cloud(
    runtime: &tokio::runtime::Handle,
    connection: Connection,
) -> oneshot::Receiver<Result<usize, String>> {
    once(runtime, async move {
        anyhow::Ok(remote::list_collectors(&connection).await?.len())
    })
}

/// The collectors in the Telelog Cloud workspace `connection` signs in to.
pub fn collectors(
    runtime: &tokio::runtime::Handle,
    connection: Connection,
) -> oneshot::Receiver<Result<Vec<Collector>, String>> {
    once(runtime, async move { remote::list_collectors(&connection).await })
}

/// Tails every running target on the server, reconnecting until the receiver or the last
/// [`Client`] handle is dropped.
pub fn spawn(runtime: &tokio::runtime::Handle, connection: Connection) -> (Client, UnboundedReceiver<ClientEvent>) {
    let (tx, rx) = unbounded();
    let retry = Arc::new(Notify::new());
    let (alive, mut dropped) = watch::channel(());
    let client = Client {
        retry: retry.clone(),
        _alive: Arc::new(alive),
        connection: connection.clone(),
        runtime: runtime.clone(),
        tx: tx.clone(),
    };
    runtime.spawn(async move {
        while !tx.is_closed() {
            let _ = tx.unbounded_send(ClientEvent::Connecting);
            let reason = tokio::select! {
                result = tail_once(&connection, &tx) => match result {
                    Ok(()) => "stream ended".to_string(),
                    Err(e) => describe(&e),
                },
                // Nothing is ever sent, so this finishes only when the last handle is dropped.
                _ = dropped.changed() => break,
            };
            if tx.unbounded_send(ClientEvent::Disconnected { reason }).is_err() {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                _ = retry.notified() => {}
                _ = dropped.changed() => break,
            }
        }
    });
    (client, rx)
}

/// The innermost cause reads best in the UI ("connection refused" rather than a transport chain).
fn describe(e: &anyhow::Error) -> String {
    // A status carries the other side's own words ("the collector is offline").
    if let Some(status) = e.downcast_ref::<tonic::Status>()
        && !status.message().is_empty()
    {
        return status.message().to_string();
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
    let mut remote = remote::connect(connection).await?;

    // Subscribe to state changes before listing, so none fall in the gap. Replaying an event
    // that the listing already reflects is harmless: each one carries the final state.
    // Servers older than WatchTargets answer Unimplemented; keep tailing without live state.
    let changes = match remote.watch_targets().await {
        Ok(changes) => changes
            .map(|target| target.map(|t| ClientEvent::TargetChanged(t.into())))
            .boxed(),
        Err(status) if status.code() == tonic::Code::Unimplemented => futures::stream::pending().boxed(),
        Err(status) => return Err(status.into()),
    };

    let targets: Vec<Target> = remote.list_targets(true).await?.into_iter().map(Into::into).collect();
    tx.unbounded_send(ClientEvent::Connected { targets })?;

    let records = remote
        .tail(v1::TailRequest {
            target_ids: Vec::new(),
            backlog: BACKLOG,
        })
        .await?
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
        assert_eq!(describe(&error), "invalid or missing token");
    }
}
