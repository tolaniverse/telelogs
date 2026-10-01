use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use telelog_proto::LogServiceServer;
use telelog_sources::DockerSource;
use tracing_subscriber::EnvFilter;

mod service;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Address the gRPC API listens on.
    #[arg(long, env = "TELELOG_LISTEN", default_value = "127.0.0.1:7070")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let docker = DockerSource::connect()?;
    let service = service::Logs::new(docker);

    tracing::info!(listen = %args.listen, "telelog-server started");
    tonic::transport::Server::builder()
        .add_service(LogServiceServer::new(service))
        .serve_with_shutdown(args.listen, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
