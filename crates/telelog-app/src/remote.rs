//! One connected server, reached directly (telelog-server) or through Telelog Cloud's relay (a
//! collector). Both answer the same LogService calls; through the relay each call travels as
//! bytes the gateway passes on without decoding.

use futures::stream::{self, BoxStream, StreamExt};
use prost::Message;
use telelog_proto::auth::BearerToken;
use telelog_proto::cloud::v1::relay_service_client::RelayServiceClient;
use telelog_proto::cloud::v1::{CallRequest, Collector, ListCollectorsRequest, LogRecords, Method};
use telelog_proto::v1;
use tonic::Status;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

use crate::client::Connection;
#[cfg(test)]
use crate::client::Token;

type Relay = RelayServiceClient<InterceptedService<Channel, BearerToken>>;
type Replies<T> = BoxStream<'static, Result<T, Status>>;

pub enum Remote {
    Direct(telelog_proto::auth::Client),
    Relay { client: Relay, collector: String },
}

pub async fn connect(connection: &Connection) -> anyhow::Result<Remote> {
    // Read on every connect, so a renewed certificate is picked up without a restart.
    let ca_pem = match &connection.ca_cert {
        Some(path) => Some(std::fs::read(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?),
        None => None,
    };
    let token = resolve(connection).await?;
    let token = token.as_deref();
    Ok(match &connection.collector {
        None => Remote::Direct(telelog_proto::auth::connect(&connection.server, token, ca_pem.as_deref()).await?),
        Some(collector) => Remote::Relay {
            client: relay_client(&connection.server, token, ca_pem.as_deref()).await?,
            collector: collector.clone(),
        },
    })
}

/// The connection's token, read on a blocking thread in case the Keychain asks the user.
async fn resolve(connection: &Connection) -> anyhow::Result<Option<String>> {
    let token = connection.token.clone();
    Ok(tokio::task::spawn_blocking(move || token.resolve()).await?)
}

async fn relay_client(gateway: &str, token: Option<&str>, ca_pem: Option<&[u8]>) -> anyhow::Result<Relay> {
    let channel = telelog_proto::auth::channel(gateway, ca_pem).await?;
    Ok(RelayServiceClient::with_interceptor(channel, BearerToken::new(token)?))
}

/// The collectors in the signed-in workspace. `connection` is the gateway, without a collector.
pub async fn list_collectors(connection: &Connection) -> anyhow::Result<Vec<Collector>> {
    let ca_pem = connection.ca_cert.as_ref().map(std::fs::read).transpose()?;
    let token = resolve(connection).await?;
    let mut client = relay_client(&connection.server, token.as_deref(), ca_pem.as_deref()).await?;
    Ok(client
        .list_collectors(ListCollectorsRequest {})
        .await?
        .into_inner()
        .collectors)
}

impl Remote {
    pub async fn list_targets(&mut self, all: bool) -> Result<Vec<v1::Target>, Status> {
        let request = v1::ListTargetsRequest { all };
        match self {
            Remote::Direct(client) => Ok(client.list_targets(request).await?.into_inner().targets),
            Remote::Relay { client, collector } => {
                let mut replies = relay(client, collector, Method::ListTargets, &request).await?;
                let reply = replies
                    .next()
                    .await
                    .ok_or_else(|| Status::unavailable("the collector sent no answer"))??;
                Ok(decode::<v1::ListTargetsResponse>(&reply)?.targets)
            }
        }
    }

    pub async fn watch_targets(&mut self) -> Result<Replies<v1::Target>, Status> {
        let request = v1::WatchTargetsRequest {};
        match self {
            Remote::Direct(client) => Ok(client.watch_targets(request).await?.into_inner().boxed()),
            Remote::Relay { client, collector } => Ok(relay(client, collector, Method::WatchTargets, &request)
                .await?
                .map(|reply| reply.and_then(|payload| decode(&payload)))
                .boxed()),
        }
    }

    pub async fn tail(&mut self, request: v1::TailRequest) -> Result<Replies<v1::LogRecord>, Status> {
        match self {
            Remote::Direct(client) => Ok(client.tail(request).await?.into_inner().boxed()),
            // Each reply carries a batch of lines; the app takes them one by one.
            Remote::Relay { client, collector } => Ok(relay(client, collector, Method::Tail, &request)
                .await?
                .flat_map(|reply| {
                    let lines: Vec<_> = match reply.and_then(|payload| decode::<LogRecords>(&payload)) {
                        Ok(batch) => batch.records.into_iter().map(Ok).collect(),
                        Err(status) => vec![Err(status)],
                    };
                    stream::iter(lines)
                })
                .boxed()),
        }
    }

    pub async fn query(&mut self, request: v1::QueryRequest) -> Result<Replies<v1::LogRecord>, Status> {
        match self {
            Remote::Direct(client) => Ok(client.query(request).await?.into_inner().boxed()),
            Remote::Relay { .. } => Err(Status::failed_precondition("Telelog Cloud doesn't store logs yet")),
        }
    }

    /// Through the relay there is no bucket yet, which reads as archiving off.
    pub async fn get_storage(&mut self) -> Result<v1::StorageInfo, Status> {
        match self {
            Remote::Direct(client) => Ok(client.get_storage(v1::GetStorageRequest {}).await?.into_inner()),
            Remote::Relay { .. } => Ok(v1::StorageInfo::default()),
        }
    }
}

async fn relay(
    client: &mut Relay,
    collector: &str,
    method: Method,
    request: &impl Message,
) -> Result<Replies<Vec<u8>>, Status> {
    let replies = client
        .call(CallRequest {
            collector_id: collector.to_string(),
            method: method.into(),
            payload: request.encode_to_vec(),
        })
        .await?
        .into_inner();
    Ok(replies.map(|reply| reply.map(|reply| reply.payload)).boxed())
}

fn decode<M: Message + Default>(payload: &[u8]) -> Result<M, Status> {
    M::decode(payload).map_err(|e| Status::internal(format!("unreadable answer from the collector: {e}")))
}

#[cfg(test)]
mod tests {
    use telelog_proto::cloud::v1::relay_service_server::{RelayService, RelayServiceServer};
    use telelog_proto::cloud::v1::{CallReply, ListCollectorsResponse};
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Code, Request, Response};

    use super::*;

    /// Answers like a gateway with one collector that has one target and four lines.
    struct FakeRelay;

    fn reply(message: &impl Message) -> Result<CallReply, Status> {
        Ok(CallReply {
            payload: message.encode_to_vec(),
        })
    }

    #[tonic::async_trait]
    impl RelayService for FakeRelay {
        async fn list_collectors(
            &self,
            request: Request<ListCollectorsRequest>,
        ) -> Result<Response<ListCollectorsResponse>, Status> {
            assert_eq!(request.metadata().get("authorization").unwrap(), "Bearer tlu_test");
            Ok(Response::new(ListCollectorsResponse {
                collectors: vec![Collector {
                    id: "col_1".into(),
                    name: "build-box".into(),
                    online: true,
                    ..Default::default()
                }],
            }))
        }

        type CallStream = BoxStream<'static, Result<CallReply, Status>>;

        async fn call(&self, request: Request<CallRequest>) -> Result<Response<Self::CallStream>, Status> {
            let request = request.into_inner();
            assert_eq!(request.collector_id, "col_1");
            let line = |body: &str| v1::LogRecord {
                origin: "api".into(),
                body: body.into(),
                ..Default::default()
            };
            let replies = match request.method() {
                Method::ListTargets => {
                    v1::ListTargetsRequest::decode(request.payload.as_slice()).unwrap();
                    vec![reply(&v1::ListTargetsResponse {
                        targets: vec![v1::Target {
                            id: "a".into(),
                            name: "api".into(),
                            ..Default::default()
                        }],
                    })]
                }
                Method::Tail => vec![
                    reply(&LogRecords {
                        records: vec![line("1"), line("2")],
                    }),
                    reply(&LogRecords {
                        records: vec![line("3"), line("4")],
                    }),
                    Err(Status::unavailable("the collector disconnected")),
                ],
                _ => return Err(Status::unavailable("the collector is offline")),
            };
            Ok(Response::new(stream::iter(replies).boxed()))
        }
    }

    async fn relay() -> Connection {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(RelayServiceServer::new(FakeRelay))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        Connection {
            server,
            token: Token::Given("tlu_test".into()),
            ca_cert: None,
            collector: Some("col_1".into()),
        }
    }

    #[tokio::test]
    async fn answers_log_service_calls_through_the_relay() {
        let connection = relay().await;
        let mut remote = connect(&connection).await.unwrap();
        assert!(matches!(remote, Remote::Relay { .. }));

        let targets = remote.list_targets(true).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "api");

        // Batches arrive as single lines, then the stream's error.
        let tail = remote.tail(v1::TailRequest::default()).await.unwrap();
        let items: Vec<_> = tail.collect().await;
        let bodies: Vec<_> = items
            .iter()
            .filter_map(|i| i.as_ref().ok())
            .map(|r| r.body.as_str())
            .collect();
        assert_eq!(bodies, ["1", "2", "3", "4"]);
        assert_eq!(items.last().unwrap().as_ref().unwrap_err().code(), Code::Unavailable);

        let status = remote.watch_targets().await.err().unwrap();
        assert_eq!(status.message(), "the collector is offline");

        // No cloud storage yet: Query says so and storage reads as off.
        let status = remote.query(v1::QueryRequest::default()).await.err().unwrap();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert!(!remote.get_storage().await.unwrap().enabled);
    }

    #[tokio::test]
    async fn lists_the_workspaces_collectors() {
        let mut connection = relay().await;
        connection.collector = None;
        let collectors = list_collectors(&connection).await.unwrap();
        assert_eq!(collectors.len(), 1);
        assert_eq!(collectors[0].name, "build-box");
    }
}
