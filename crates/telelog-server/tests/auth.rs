//! End-to-end checks of token auth and TLS against a real in-process server.
//!
//! The server is given an unreachable Docker daemon, so an authorized `ListTargets` fails with
//! `Unavailable`. What matters here is whether the request gets past authentication.

use std::path::Path;

use telelog_proto::auth::connect;
use telelog_proto::v1::ListTargetsRequest;
use telelog_server::{Config, TlsFiles};
use telelog_sources::DockerSource;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const TOKEN: &str = "test-token-0123456789abcdef";

struct Running {
    port: u16,
    _stop: oneshot::Sender<()>,
}

async fn start(config: Config) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = oneshot::channel::<()>();
    // An address nothing listens on: these tests only care whether requests pass auth, and
    // must behave the same on machines without Docker.
    let docker = DockerSource::connect_http("tcp://127.0.0.1:9").unwrap();
    tokio::spawn(async move {
        telelog_server::serve(listener, config, docker, async {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    });
    Running { port, _stop: stop }
}

/// The gRPC status code of a ListTargets call, or `Ok` when it succeeds.
async fn list_targets(endpoint: &str, token: Option<&str>, ca: Option<&[u8]>) -> tonic::Code {
    let mut client = connect(endpoint, token, ca).await.expect("transport connects");
    match client.list_targets(ListTargetsRequest { all: false }).await {
        Ok(_) => tonic::Code::Ok,
        Err(status) => status.code(),
    }
}

fn passed_auth(code: tonic::Code) -> bool {
    matches!(code, tonic::Code::Ok | tonic::Code::Unavailable)
}

#[tokio::test]
async fn plaintext_server_requires_the_token() {
    let server = start(Config {
        token: Some(TOKEN.into()),
        tls: None,
        archive: None,
    })
    .await;
    let endpoint = format!("http://127.0.0.1:{}", server.port);

    assert_eq!(list_targets(&endpoint, None, None).await, tonic::Code::Unauthenticated);
    assert_eq!(
        list_targets(&endpoint, Some("wrong-token-0000000000"), None).await,
        tonic::Code::Unauthenticated
    );
    assert!(passed_auth(list_targets(&endpoint, Some(TOKEN), None).await));
}

#[tokio::test]
async fn server_without_token_is_open() {
    let server = start(Config {
        token: None,
        tls: None,
        archive: None,
    })
    .await;
    let endpoint = format!("http://127.0.0.1:{}", server.port);
    assert!(passed_auth(list_targets(&endpoint, None, None).await));
}

fn write_self_signed(dir: &Path) -> (TlsFiles, Vec<u8>) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.cert.pem();
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, cert.signing_key.serialize_pem()).unwrap();
    (
        TlsFiles {
            cert: cert_path,
            key: key_path,
        },
        cert_pem.into_bytes(),
    )
}

#[tokio::test]
async fn tls_server_with_token() {
    let dir = std::env::temp_dir().join(format!("telelog-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (tls, ca) = write_self_signed(&dir);
    let server = start(Config {
        token: Some(TOKEN.into()),
        tls: Some(tls),
        archive: None,
    })
    .await;
    let endpoint = format!("https://localhost:{}", server.port);

    // Trusting the self-signed CA: auth is enforced over TLS.
    assert_eq!(
        list_targets(&endpoint, None, Some(&ca)).await,
        tonic::Code::Unauthenticated
    );
    assert!(passed_auth(list_targets(&endpoint, Some(TOKEN), Some(&ca)).await));

    // Without the CA the certificate isn't trusted, so the connection is refused outright.
    let Err(untrusted) = connect(&endpoint, Some(TOKEN), None).await else {
        panic!("untrusted certificate was accepted");
    };
    assert!(format!("{untrusted:#}").contains("UnknownIssuer"), "{untrusted:#}");

    // Plain HTTP against the TLS port never gets a successful answer.
    let plain = format!("http://localhost:{}", server.port);
    if let Ok(mut client) = connect(&plain, Some(TOKEN), None).await {
        assert!(client.list_targets(ListTargetsRequest { all: false }).await.is_err());
    }

    let _ = std::fs::remove_dir_all(&dir);
}
