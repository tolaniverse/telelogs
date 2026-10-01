use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
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

    let docker = DockerSource::connect()?;
    let listener = TcpListener::bind(args.listen).await?;
    tracing::info!(
        listen = %args.listen,
        auth = if token.is_some() { "token" } else { "none" },
        tls = tls.is_some(),
        "telelog-server started"
    );
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    telelog_server::serve(listener, Config { token, tls }, docker, shutdown).await
}
