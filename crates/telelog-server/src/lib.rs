//! telelog-server as a library, so integration tests can run it in-process.

pub mod auth;
mod service;

use std::future::Future;
use std::path::PathBuf;

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
}

/// Serves the gRPC API on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    config: Config,
    docker: DockerSource,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let mut builder = Server::builder();
    if let Some(tls) = &config.tls {
        let cert = std::fs::read(&tls.cert).with_context(|| format!("reading {}", tls.cert.display()))?;
        let key = std::fs::read(&tls.key).with_context(|| format!("reading {}", tls.key.display()))?;
        builder = builder
            .tls_config(ServerTlsConfig::new().identity(Identity::from_pem(cert, key)))
            .context("configuring TLS")?;
    }
    let service = LogServiceServer::with_interceptor(service::Logs::new(docker), auth::TokenAuth::new(config.token));
    builder
        .add_service(service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await?;
    Ok(())
}
