//! Signing in to Telelog Cloud from the browser: the app asks the website for a code, the person
//! approves it there, and the app picks up its token and the gateway's address.

use std::time::Duration;

use serde::Deserialize;
use tokio::sync::oneshot;

/// The Telelog Cloud website. `TELELOG_CLOUD_URL` points the app at another one (development).
pub fn site() -> String {
    std::env::var("TELELOG_CLOUD_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "https://telelogs-web.olatundeomotolani23.workers.dev".into())
        .trim_end_matches('/')
        .to_string()
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Deserialize)]
pub struct Started {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri_complete: String,
    /// Seconds between polls.
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Pending,
    SignedIn { token: String, gateway: String },
    Denied,
    Expired,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("telelogs/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Runs a blocking request off the UI thread.
fn blocking<T: Send + 'static>(
    runtime: &tokio::runtime::Handle,
    request: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> oneshot::Receiver<Result<T, String>> {
    let (tx, rx) = oneshot::channel();
    runtime.spawn_blocking(move || {
        let _ = tx.send(request());
    });
    rx
}

fn unreachable(site: &str, e: ureq::Error) -> String {
    format!("couldn't reach {site}: {e}")
}

/// Asks for a code to show and approve in the browser.
pub fn start(runtime: &tokio::runtime::Handle, site: String) -> oneshot::Receiver<Result<Started, String>> {
    blocking(runtime, move || {
        let mut response = agent()
            .post(format!("{site}/api/device/start"))
            .send_json(serde_json::json!({ "device": device_name() }))
            .map_err(|e| unreachable(&site, e))?;
        if !response.status().is_success() {
            return Err(format!("Telelog Cloud answered {}", response.status()));
        }
        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("unexpected answer from Telelog Cloud: {e}"))
    })
}

/// Whether the code has been approved yet.
pub fn poll(
    runtime: &tokio::runtime::Handle,
    site: String,
    device_code: String,
) -> oneshot::Receiver<Result<Poll, String>> {
    blocking(runtime, move || {
        let mut response = agent()
            .post(format!("{site}/api/device/token"))
            .send_json(serde_json::json!({ "device_code": device_code }))
            .map_err(|e| unreachable(&site, e))?;
        let ok = response.status().is_success();
        let body: Answer = response
            .body_mut()
            .read_json()
            .map_err(|e| format!("unexpected answer from Telelog Cloud: {e}"))?;
        answer(ok, body)
    })
}

#[derive(Debug, Default, Deserialize)]
struct Answer {
    token: Option<String>,
    gateway: Option<String>,
    error: Option<String>,
}

fn answer(ok: bool, body: Answer) -> Result<Poll, String> {
    if ok {
        let token = body.token.ok_or("Telelog Cloud sent no token")?;
        let gateway = body.gateway.ok_or("Telelog Cloud has no gateway set up yet")?;
        return Ok(Poll::SignedIn { token, gateway });
    }
    Ok(match body.error.as_deref() {
        Some("authorization_pending") => Poll::Pending,
        Some("access_denied") => Poll::Denied,
        _ => Poll::Expired,
    })
}

/// What the dashboard lists this sign-in as.
fn device_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|name| name.trim().trim_end_matches(".local").to_string())
        .filter(|name| !name.is_empty())
        .map_or_else(|| "Telelogs app".into(), |name| format!("Telelogs on {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(json: &str) -> Answer {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_the_websites_answers() {
        assert_eq!(
            answer(false, body(r#"{"error":"authorization_pending"}"#)),
            Ok(Poll::Pending)
        );
        assert_eq!(answer(false, body(r#"{"error":"access_denied"}"#)), Ok(Poll::Denied));
        assert_eq!(answer(false, body(r#"{"error":"expired_token"}"#)), Ok(Poll::Expired));
        assert_eq!(
            answer(
                true,
                body(r#"{"token":"tlu_1","gateway":"https://gw.test","workspace":{"id":"ws_1","name":"x"}}"#)
            ),
            Ok(Poll::SignedIn {
                token: "tlu_1".into(),
                gateway: "https://gw.test".into()
            })
        );
        assert!(answer(true, body(r#"{"token":"tlu_1","gateway":null}"#)).is_err());
    }
}
