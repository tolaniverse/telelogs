use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use telelog_server::archive::Archive;
use telelog_server::{Config, TlsFiles, auth};
use telelog_sources::{DockerSource, KubeSource, Sources};
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

    /// Don't read Docker containers (for a server that only reads Kubernetes).
    #[arg(long, env = "TELELOG_NO_DOCKER")]
    no_docker: bool,

    /// Read pod logs from Kubernetes: the kubeconfig's current context, or the pod's service
    /// account when running in a cluster.
    #[arg(long, env = "TELELOG_KUBERNETES")]
    kubernetes: bool,

    /// Kubeconfig context to use instead of the current one. Implies --kubernetes.
    #[arg(long, env = "TELELOG_KUBE_CONTEXT")]
    kube_context: Option<String>,

    /// Namespaces to read; repeat the flag or separate with commas. All namespaces by default.
    #[arg(long = "namespace", env = "TELELOG_NAMESPACES", value_delimiter = ',')]
    namespaces: Vec<String>,

    /// Only read pods matching this label selector, e.g. `app=api` or `tier in (web,api)`.
    #[arg(long, env = "TELELOG_KUBE_SELECTOR")]
    selector: Option<String>,
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
    // The Kubernetes client, object_store and tonic each enable a rustls crypto backend; pick
    // one before any of them opens a connection.
    let _ = rustls::crypto::ring::default_provider().install_default();
    if let Some(Command::GenToken) = args.command {
        println!("{}", auth::generate_token()?);
        return Ok(());
    }

    let sources = connect_sources(&args).await?;
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
    telelog_server::serve(listener, Config { token, tls, archive }, sources, shutdown).await
}

/// Docker unless `--no-docker`, plus Kubernetes when asked for. With Kubernetes on, a missing
/// Docker daemon is only a warning, so the same flags work on a laptop and inside a cluster.
async fn connect_sources(args: &Args) -> Result<Sources> {
    let kubernetes = args.kubernetes || args.kube_context.is_some();
    let mut sources = Sources::default();
    if !args.no_docker {
        match DockerSource::connect() {
            // Pods that Kubernetes runs on this same daemon are read by the Kubernetes source.
            Ok(docker) if kubernetes => sources.docker = Some(docker.without_kubernetes()),
            Ok(docker) => sources.docker = Some(docker),
            Err(e) if kubernetes => tracing::warn!("not reading Docker: {e:#}"),
            Err(e) => return Err(e),
        }
    }
    if kubernetes {
        let kube = KubeSource::connect(
            args.kube_context.as_deref(),
            args.namespaces.clone(),
            args.selector.clone(),
        )
        .await?;
        tracing::info!(
            cluster = %kube.cluster,
            namespaces = %if args.namespaces.is_empty() { "all".to_string() } else { args.namespaces.join(",") },
            "reading Kubernetes"
        );
        sources.kubernetes = Some(kube);
    } else if !args.namespaces.is_empty() || args.selector.is_some() {
        bail!("--namespace and --selector need --kubernetes");
    }
    if sources.is_empty() {
        bail!("no log sources: drop --no-docker or add --kubernetes");
    }
    Ok(sources)
}
