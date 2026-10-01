//! Bearer-token authentication and the rules for when the server may run without it.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Result, bail};
use tonic::service::Interceptor;
use tonic::{Request, Status};

/// Rejects requests whose `authorization` header doesn't carry the expected bearer token.
#[derive(Clone)]
pub struct TokenAuth {
    expected: Option<Arc<str>>,
}

impl TokenAuth {
    pub fn new(token: Option<String>) -> Self {
        TokenAuth {
            expected: token.map(Into::into),
        }
    }
}

impl Interceptor for TokenAuth {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(expected) = &self.expected else {
            return Ok(request);
        };
        let presented = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        match presented {
            Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => Ok(request),
            _ => Err(Status::unauthenticated("invalid or missing token")),
        }
    }
}

/// Compares without short-circuiting, so response timing doesn't reveal how much matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A random 256-bit token, hex encoded.
pub fn generate_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("reading system randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Refuses configurations that would expose logs to the network without a token, and returns
/// a warning for ones that are allowed but risky.
pub fn check_exposure(
    listen: SocketAddr,
    has_token: bool,
    has_tls: bool,
    allow_unauthenticated: bool,
) -> Result<Option<&'static str>> {
    let local = listen.ip().is_loopback();
    if !local && !has_token && !allow_unauthenticated {
        bail!(
            "refusing to listen on {listen} without a token: anyone who can reach it could read your logs. \
             Set --token (generate one with `telelog-server gen-token`), listen on 127.0.0.1, \
             or pass --allow-unauthenticated if a proxy in front handles auth."
        );
    }
    Ok(match (local, has_token, has_tls) {
        (false, false, _) => Some("running without authentication on a non-local address"),
        (false, true, false) => {
            Some("token is sent in plain text; enable --tls-cert/--tls-key or terminate TLS in a proxy")
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(header: Option<&str>) -> Request<()> {
        let mut request = Request::new(());
        if let Some(header) = header {
            request.metadata_mut().insert("authorization", header.parse().unwrap());
        }
        request
    }

    #[test]
    fn accepts_only_the_right_token() {
        let mut auth = TokenAuth::new(Some("s3cret".into()));
        assert!(auth.call(request(Some("Bearer s3cret"))).is_ok());
        for bad in [
            None,
            Some("Bearer wrong"),
            Some("Bearer s3cre"),
            Some("s3cret"),
            Some("Basic s3cret"),
        ] {
            let status = auth.call(request(bad)).unwrap_err();
            assert_eq!(status.code(), tonic::Code::Unauthenticated, "{bad:?}");
        }
    }

    #[test]
    fn no_token_configured_allows_everything() {
        assert!(TokenAuth::new(None).call(request(None)).is_ok());
    }

    #[test]
    fn generated_tokens_are_long_and_distinct() {
        let (a, b) = (generate_token().unwrap(), generate_token().unwrap());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn exposure_rules() {
        let local: SocketAddr = "127.0.0.1:7070".parse().unwrap();
        let public: SocketAddr = "0.0.0.0:7070".parse().unwrap();
        assert_eq!(check_exposure(local, false, false, false).unwrap(), None);
        assert!(check_exposure(public, false, false, false).is_err());
        assert!(check_exposure(public, false, false, true).unwrap().is_some());
        assert!(check_exposure(public, true, false, false).unwrap().is_some());
        assert_eq!(check_exposure(public, true, true, false).unwrap(), None);
    }
}
