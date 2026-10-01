use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use telelog_server::archive::Archive;
use telelog_server::{Config, TlsFiles, auth};
use telelog_sources::DockerSource;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Address the gRPC API listens on.
    #[arg(long, env = "TELELOG_LISTEN", default_value = "127.0.0.1:7070")]
    listen: SocketAddr,

    /// Bearer token clients must present. Required when listening beyond localhost.
    #[arg(long, env = "TELELOG_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// PEM certificate chain for TLS. Use with --tls-key.
    #[arg(long, env = "TELELOG_TLS_CERT", requires = "tls_key")]
    tls_cert: Option<PathBuf>,

    /// PEM private key for TLS. Use with --tls-cert.
    #[arg(long, env = "TELELOG_TLS_KEY", requires = "tls_cert")]
    tls_key: Option<PathBuf>,

    /// Listen beyond localhost without a token (only behind a proxy that authenticates).
    #[arg(long)]
    allow_unauthenticated: bool,

    /// Bucket to archive every log line to: s3://bucket/prefix (also R2/MinIO via AWS_ENDPOINT),
    /// gs://bucket/prefix, or file:///path. Credentials come from the provider's usual env vars.
    #[arg(long, env = "TELELOG_ARCHIVE_URL")]
    archive_url: Option<String>,

    /// Seconds between archive flushes (1 to 3600).
    #[arg(long, env = "TELELOG_ARCHIVE_FLUSH_SECS", default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
    archive_flush_secs: u64,

    /// Days to keep archived logs; 0 keeps them forever.
    #[arg(long, env = "TELELOG_RETENTION_DAYS", default_value_t = 90)]
    retention_days: u32,
}

#[derive(Subcommand)]
enum Command {
    /// Print a new random token for --token / TELELOG_TOKEN.
    GenToken,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    if let Some(Command::GenToken) = args.command {
        println!("{}", auth::generate_token()?);
        return Ok(());
    }

    let token = args.token.filter(|t| !t.is_empty());
    if token.as_ref().is_some_and(|t| t.len() < 16) {
        bail!("token is too short; use at least 16 characters (`telelog-server gen-token` makes one)");
    }
    let tls = args
        .tls_cert
        .zip(args.tls_key)
        .map(|(cert, key)| TlsFiles { cert, key });
    if let Some(warning) =
        auth::check_exposure(args.listen, token.is_some(), tls.is_some(), args.allow_unauthenticated)?
    {
        tracing::warn!("{warning}");
    }

    let archive = match &args.archive_url {
        Some(url) => {
            let mut archive = Archive::open(url)?;
            archive.flush_every = Duration::from_secs(args.archive_flush_secs);
            archive.retention_days = args.retention_days;
            Some(Arc::new(archive))
        }
        None => None,
    };

    let docker = DockerSource::connect()?;
    let listener = TcpListener::bind(args.listen).await?;
    tracing::info!(
        listen = %args.listen,
        auth = if token.is_some() { "token" } else { "none" },
        tls = tls.is_some(),
        archive = archive.as_ref().map_or("off".to_string(), |a| a.location.clone()),
        "telelog-server started"
    );
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    telelog_server::serve(listener, Config { token, tls, archive }, docker, shutdown).await
}
