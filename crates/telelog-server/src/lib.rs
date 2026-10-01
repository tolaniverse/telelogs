//! telelog-server as a library, so integration tests can run it in-process.

pub mod archive;
pub mod auth;
mod service;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use telelog_proto::LogServiceServer;
use telelog_sources::DockerSource;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};

pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

pub struct Config {
    pub token: Option<String>,
    pub tls: Option<TlsFiles>,
    /// When set, every container's output is archived to this bucket and can be queried.
    pub archive: Option<Arc<archive::Archive>>,
}

/// Serves the gRPC API on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    docker: DockerSource,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    // object_store and tonic enable different rustls crypto backends; pick one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = Server::builder();
    if let Some(tls) = &config.tls {
        let cert = std::fs::read(&tls.cert).with_context(|| format!("reading {}", tls.cert.display()))?;
        let key = std::fs::read(&tls.key).with_context(|| format!("reading {}", tls.key.display()))?;
        builder = builder
            .tls_config(ServerTlsConfig::new().identity(Identity::from_pem(cert, key)))
            .context("configuring TLS")?;
    }
    // Archiving runs for as long as the server does, whether or not any app is connected.
    let background: Vec<_> = match &config.archive {
        Some(archive) => vec![
            tokio::spawn(archive::ingest::run(archive.clone(), docker.clone())),
            tokio::spawn(archive::ingest::sweep_forever(archive.clone())),
        ],
        None => Vec::new(),
    };
    let service = LogServiceServer::with_interceptor(
        service::Logs::new(docker, config.archive.clone()),
        auth::TokenAuth::new(config.token),
    );
    builder
        .add_service(service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await?;
    for task in background {
        task.abort();
    }
    Ok(())
}
