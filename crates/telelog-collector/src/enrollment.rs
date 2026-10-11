//! Enrolling: a one-time token from the web console is traded for this collector's id and key,
//! which a state file keeps for every later start.

use std::path::Path;

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};
use telelog_proto::cloud::v1::EnrollRequest;
use telelog_proto::cloud::v1::collector_service_client::CollectorServiceClient;
use tonic::Code;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enrollment {
    /// Telelog Cloud's gateway, e.g. `https://gateway.example.com`.
    pub gateway: String,
    pub collector_id: String,
    pub workspace_id: String,
    /// The collector's credential for attaching; as secret as a password.
    pub key: String,
}

impl Enrollment {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
    }

    /// Writes the file readable by its owner only, through a temporary file so a crash can't
    /// leave half of it behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("tmp");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            std::io::Write::write_all(&mut options.open(&tmp)?, &serde_json::to_vec_pretty(self)?)?;
            std::fs::rename(tmp, path)
        };
        write().with_context(|| format!("writing {}", path.display()))
    }
}

/// Trades the one-time `token` for this collector's id and key.
pub async fn enroll(gateway: &str, token: &str, ca_pem: Option<&[u8]>) -> Result<Enrollment> {
    let gateway = gateway.trim_end_matches('/');
    let channel = telelog_proto::auth::channel(gateway, ca_pem)
        .await
        .with_context(|| format!("connecting to {gateway}"))?;
    let response = CollectorServiceClient::new(channel)
        .enroll(EnrollRequest {
            token: token.to_string(),
            host: crate::host_name(),
            version: crate::VERSION.to_string(),
        })
        .await
        .map_err(|status| match status.code() {
            Code::Unauthenticated | Code::PermissionDenied | Code::NotFound => anyhow!(
                "the gateway refused the enrollment token ({}); tokens work once, so make a new one in the web console",
                status.message()
            ),
            _ => anyhow!(status).context("enrolling"),
        })?
        .into_inner();
    Ok(Enrollment {
        gateway: gateway.to_string(),
        collector_id: response.collector_id,
        workspace_id: response.workspace_id,
        key: response.key,
    })
}
