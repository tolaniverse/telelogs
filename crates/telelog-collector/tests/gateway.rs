//! The collector against a fake gateway running in-process: enrolling, answering calls,
//! keep-alive and reconnecting.

use std::time::Duration;

use prost::Message;
use telelog_collector::{Collector, Enrollment, Timing};
use telelog_proto::cloud::v1::collector_service_server::{CollectorService, CollectorServiceServer};
use telelog_proto::cloud::v1::{
    Close, CollectorFrame, EnrollRequest, EnrollResponse, GatewayFrame, Hello, Method, Open, Ping, Pong,
    collector_frame, gateway_frame,
};
use telelog_proto::v1;
use telelog_sources::Sources;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Code, Request, Response, Status, Streaming};

const TOKEN: &str = "tk_good";
const KEY: &str = "collector-key-0123456789";
const WAIT: Duration = Duration::from_secs(5);

/// One collector connection, handed to the test to drive.
struct Attached {
    inbound: Streaming<CollectorFrame>,
    outbound: mpsc::Sender<Result<GatewayFrame, Status>>,
}

impl Attached {
    async fn next(&mut self) -> collector_frame::Frame {
        tokio::time::timeout(WAIT, self.inbound.message())
            .await
            .expect("the collector sent nothing")
            .expect("the stream failed")
            .expect("the collector hung up")
            .frame
            .expect("an empty frame")
    }

    async fn hello(&mut self) -> Hello {
        match self.next().await {
            collector_frame::Frame::Hello(hello) => hello,
            other => panic!("expected Hello first, got {other:?}"),
        }
    }

    async fn send(&self, frame: gateway_frame::Frame) {
        self.outbound
            .send(Ok(GatewayFrame { frame: Some(frame) }))
            .await
            .expect("the connection is gone");
    }

    async fn open(&self, call: u64, method: Method, payload: Vec<u8>) {
        self.send(gateway_frame::Frame::Open(Open {
            call,
            method: method.into(),
            payload,
        }))
        .await;
    }
}

struct FakeGateway {
    attached: mpsc::UnboundedSender<Attached>,
}

#[tonic::async_trait]
impl CollectorService for FakeGateway {
    async fn enroll(&self, request: Request<EnrollRequest>) -> Result<Response<EnrollResponse>, Status> {
        let request = request.into_inner();
        if request.token != TOKEN {
            return Err(Status::permission_denied("unknown token"));
        }
        assert!(!request.host.is_empty());
        Ok(Response::new(EnrollResponse {
            collector_id: "col_1".into(),
            workspace_id: "ws_1".into(),
            key: KEY.into(),
        }))
    }

    type AttachStream = ReceiverStream<Result<GatewayFrame, Status>>;

    async fn attach(
        &self,
        request: Request<Streaming<CollectorFrame>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let authorization = request.metadata().get("authorization").and_then(|v| v.to_str().ok());
        if authorization != Some(&format!("Bearer {KEY}")) {
            return Err(Status::unauthenticated("unknown collector key"));
        }
        let (outbound, rx) = mpsc::channel(16);
        let _ = self.attached.send(Attached {
            inbound: request.into_inner(),
            outbound,
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Starts a fake gateway; returns its URL and the connections collectors make to it.
async fn gateway() -> (String, mpsc::UnboundedReceiver<Attached>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (attached, connections) = mpsc::unbounded_channel();
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(CollectorServiceServer::new(FakeGateway { attached }))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    (url, connections)
}

/// A collector with no sources, attached to `url`, until the returned sender is dropped.
fn start(url: &str, key: &str, timing: Timing) -> (tokio::task::JoinHandle<anyhow::Result<()>>, mpsc::Sender<()>) {
    let collector = Collector {
        enrollment: Enrollment {
            gateway: url.to_string(),
            collector_id: "col_1".into(),
            workspace_id: "ws_1".into(),
            key: key.to_string(),
        },
        sources: Sources::default(),
        ca_pem: None,
        timing,
    };
    let (stop, mut stopped) = mpsc::channel::<()>(1);
    let task = tokio::spawn(telelog_collector::run(collector, async move {
        stopped.recv().await;
    }));
    (task, stop)
}

/// Fast enough for tests; pings effectively off unless a test turns them on.
fn quick() -> Timing {
    Timing {
        ping_every: Duration::from_secs(3600),
        missed_pings: 3,
        first_retry: Duration::from_millis(20),
        max_retry: Duration::from_millis(100),
    }
}

async fn next_connection(connections: &mut mpsc::UnboundedReceiver<Attached>) -> Attached {
    tokio::time::timeout(WAIT, connections.recv())
        .await
        .expect("the collector didn't attach")
        .unwrap()
}

#[tokio::test]
async fn enrolls_and_keeps_the_key_private() {
    let (url, _connections) = gateway().await;
    let enrollment = telelog_collector::enroll(&format!("{url}/"), TOKEN, None)
        .await
        .unwrap();
    assert_eq!(enrollment.gateway, url);
    assert_eq!(enrollment.collector_id, "col_1");
    assert_eq!(enrollment.key, KEY);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state").join("collector.json");
    enrollment.save(&path).unwrap();
    assert_eq!(Enrollment::load(&path).unwrap(), enrollment);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let refused = telelog_collector::enroll(&url, "tk_used", None).await.unwrap_err();
    assert!(
        refused.to_string().contains("refused the enrollment token"),
        "{refused:#}"
    );
}

#[tokio::test]
async fn answers_forwarded_calls() {
    let (url, mut connections) = gateway().await;
    let (_task, _stop) = start(&url, KEY, quick());
    let mut conn = next_connection(&mut connections).await;
    let hello = conn.hello().await;
    assert_eq!(hello.boot_id.len(), 32);
    assert!(hello.key_id.is_empty());

    // ListTargets: one Reply, then a clean Close.
    conn.open(
        1,
        Method::ListTargets,
        v1::ListTargetsRequest { all: true }.encode_to_vec(),
    )
    .await;
    match conn.next().await {
        collector_frame::Frame::Reply(reply) => {
            assert_eq!(reply.call, 1);
            let response = v1::ListTargetsResponse::decode(reply.payload.as_slice()).unwrap();
            assert!(response.targets.is_empty());
        }
        other => panic!("expected a Reply, got {other:?}"),
    }
    assert!(matches!(
        conn.next().await,
        collector_frame::Frame::Close(Close { call: 1, code: 0, .. })
    ));

    // A method this collector doesn't know closes the call with UNIMPLEMENTED.
    conn.send(gateway_frame::Frame::Open(Open {
        call: 2,
        method: 99,
        payload: Vec::new(),
    }))
    .await;
    match conn.next().await {
        collector_frame::Frame::Close(close) => {
            assert_eq!(close.call, 2);
            assert_eq!(close.code, Code::Unimplemented as i32);
        }
        other => panic!("expected a Close, got {other:?}"),
    }

    // A call the gateway closes stops without another word; the next frame answers the ping.
    conn.open(3, Method::WatchTargets, v1::WatchTargetsRequest {}.encode_to_vec())
        .await;
    conn.send(gateway_frame::Frame::Close(Close {
        call: 3,
        ..Default::default()
    }))
    .await;
    conn.send(gateway_frame::Frame::Ping(Ping { nonce: 6 })).await;
    assert!(matches!(
        conn.next().await,
        collector_frame::Frame::Pong(Pong { nonce: 6 })
    ));
}

#[tokio::test]
async fn reconnects_when_the_gateway_hangs_up() {
    let (url, mut connections) = gateway().await;
    let (_task, _stop) = start(&url, KEY, quick());
    let mut first = next_connection(&mut connections).await;
    let boot_id = first.hello().await.boot_id;
    drop(first.outbound);

    let mut second = next_connection(&mut connections).await;
    assert_eq!(second.hello().await.boot_id, boot_id, "the same run keeps its boot id");
}

#[tokio::test]
async fn reconnects_when_pings_go_unanswered() {
    let (url, mut connections) = gateway().await;
    let timing = Timing {
        ping_every: Duration::from_millis(20),
        missed_pings: 2,
        ..quick()
    };
    let (_task, _stop) = start(&url, KEY, timing);
    let mut first = next_connection(&mut connections).await;
    first.hello().await;
    // Never answer: after two pings the collector gives up on this connection.
    for _ in 0..2 {
        assert!(matches!(first.next().await, collector_frame::Frame::Ping(_)));
    }
    let mut second = next_connection(&mut connections).await;
    second.hello().await;
}

#[tokio::test]
async fn stops_when_the_key_is_refused() {
    let (url, _connections) = gateway().await;
    let (task, _stop) = start(&url, "revoked-key-0123456789", quick());
    let error = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("enroll it again"), "{error:#}");
}

#[tokio::test]
async fn stops_on_shutdown() {
    let (url, mut connections) = gateway().await;
    let (task, stop) = start(&url, KEY, quick());
    next_connection(&mut connections).await.hello().await;
    drop(stop);
    tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
}

/// Needs Docker with a container writing logs, e.g.
/// `docker run -d --rm --name telelog-demo alpine sh -c 'while true; do echo tick; sleep 0.2; done'`.
#[tokio::test]
#[ignore = "needs a Docker daemon with a running container"]
async fn tails_docker_through_the_gateway() {
    let (url, mut connections) = gateway().await;
    let collector = Collector {
        enrollment: Enrollment {
            gateway: url,
            collector_id: "col_1".into(),
            workspace_id: "ws_1".into(),
            key: KEY.into(),
        },
        sources: Sources {
            docker: Some(telelog_sources::DockerSource::connect().unwrap()),
            kubernetes: None,
        },
        ca_pem: None,
        timing: quick(),
    };
    tokio::spawn(telelog_collector::run(collector, std::future::pending()));
    let mut conn = next_connection(&mut connections).await;
    conn.hello().await;

    conn.open(
        1,
        Method::Tail,
        v1::TailRequest {
            target_ids: Vec::new(),
            backlog: 5,
        }
        .encode_to_vec(),
    )
    .await;
    let collector_frame::Frame::Reply(reply) = conn.next().await else {
        panic!("expected log lines");
    };
    let records = telelog_proto::cloud::v1::LogRecords::decode(reply.payload.as_slice()).unwrap();
    assert!(!records.records.is_empty());
    assert!(
        records
            .records
            .iter()
            .all(|r| !r.origin.is_empty() && r.timestamp.is_some())
    );
}
