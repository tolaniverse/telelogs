//! Client-side connection setup shared by the app and tests: optional TLS and a bearer token.

use anyhow::{Context as _, Result};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Status};

use crate::LogServiceClient;

/// Adds `authorization: Bearer <token>` to every request when a token is set.
#[derive(Clone)]
pub struct BearerToken(Option<MetadataValue<Ascii>>);

impl BearerToken {
    pub fn new(token: Option<&str>) -> Result<Self> {
        let value = token
            .map(|t| {
                let mut value = format!("Bearer {t}").parse::<MetadataValue<Ascii>>()?;
                // Keeps the token out of Debug output and HTTP/2 header compression tables.
                value.set_sensitive(true);
                Ok::<_, tonic::metadata::errors::InvalidMetadataValue>(value)
            })
            .transpose()
            .context("token contains characters that can't be sent in a header")?;
        Ok(BearerToken(value))
    }
}

impl Interceptor for BearerToken {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(value) = &self.0 {
            request.metadata_mut().insert("authorization", value.clone());
        }
        Ok(request)
    }
}

pub type Client = LogServiceClient<InterceptedService<Channel, BearerToken>>;

/// Connects to `endpoint` (`http://` or `https://`). For `https`, the system roots are trusted,
/// plus `ca_pem` when given (for self-signed server certificates).
pub async fn connect(endpoint: &str, token: Option<&str>, ca_pem: Option<&[u8]>) -> Result<Client> {
    let mut builder = Endpoint::from_shared(endpoint.to_string()).context("invalid server address")?;
    if endpoint.starts_with("https://") {
        let mut tls = ClientTlsConfig::new().with_native_roots();
        if let Some(pem) = ca_pem {
            tls = tls.ca_certificate(Certificate::from_pem(pem));
        }
        builder = builder.tls_config(tls).context("configuring TLS")?;
    } else if ca_pem.is_some() {
        anyhow::bail!("a CA certificate was given but the server address is not https://");
    }
    let channel = builder.connect().await?;
    Ok(LogServiceClient::with_interceptor(channel, BearerToken::new(token)?))
}
