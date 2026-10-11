use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use telelog_collector::{Collector, Enrollment, Timing};
use telelog_sources::{Options, Sources};
use tracing_subscriber::EnvFilter;

/// Reads Docker and Kubernetes logs and serves them to Telelog Cloud over one outbound
/// connection; nothing on this machine listens for connections.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// One-time enrollment token from the web console. Enrolls this collector, saves its key in
    /// the state file, then runs. Later starts need neither this nor --gateway.
    #[arg(long, env = "TELELOG_ENROLL_TOKEN", hide_env_values = true, requires = "gateway")]
    enroll: Option<String>,

    /// Telelog Cloud's gateway, for enrolling.
    #[arg(long, env = "TELELOG_GATEWAY")]
    gateway: Option<String>,

    /// Where the collector keeps its id and key [default: the user's data directory].
    #[arg(long, env = "TELELOG_COLLECTOR_STATE")]
    state: Option<PathBuf>,

    /// PEM CA certificate to trust for the gateway, for a development gateway with a self-signed
    /// certificate.
    #[arg(long, env = "TELELOG_GATEWAY_CA_CERT")]
    ca_cert: Option<PathBuf>,

    /// Don't read Docker containers (for a collector that only reads Kubernetes).
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    // The Kubernetes client and tonic each enable a rustls crypto backend; pick one before
    // either opens a connection.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let state = match args.state {
        Some(path) => path,
        None => dirs::data_local_dir()
            .context("no data directory for the state file; pass --state")?
            .join("telelog-collector")
            .join("collector.json"),
    };
    let ca_pem = args
        .ca_cert
        .map(|path| std::fs::read(&path).with_context(|| format!("reading {}", path.display())))
        .transpose()?;

    let enrollment = match (args.enroll, args.gateway) {
        (Some(token), Some(gateway)) => {
            let enrollment = telelog_collector::enroll(&gateway, &token, ca_pem.as_deref()).await?;
            enrollment.save(&state)?;
            tracing::info!(collector = %enrollment.collector_id, state = %state.display(), "enrolled");
            enrollment
        }
        _ => Enrollment::load(&state).with_context(|| {
            format!(
                "this collector isn't enrolled yet (no {}): run it once with --enroll <token> --gateway <url>",
                state.display()
            )
        })?,
    };

    let sources = Sources::connect(&Options {
        no_docker: args.no_docker,
        kubernetes: args.kubernetes,
        kube_context: args.kube_context,
        namespaces: args.namespaces,
        selector: args.selector,
    })
    .await?;

    let collector = Collector {
        enrollment,
        sources,
        ca_pem,
        timing: Timing::default(),
    };
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    telelog_collector::run(collector, shutdown).await
}
