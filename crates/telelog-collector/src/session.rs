//! The collector's one connection to the gateway: attach, answer the calls it forwards, keep it
//! alive, and reconnect with backoff when it drops.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use telelog_proto::auth::BearerToken;
use telelog_proto::cloud::v1::collector_frame::Frame;
use telelog_proto::cloud::v1::collector_service_client::CollectorServiceClient;
use telelog_proto::cloud::v1::{Hello, Ping, Pong, gateway_frame};
use telelog_sources::Sources;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

use crate::{Enrollment, frame};

pub struct Collector {
    pub enrollment: Enrollment,
    pub sources: Sources,
    /// PEM CA certificate to trust for the gateway, for a development gateway with a
    /// self-signed certificate.
    pub ca_pem: Option<Vec<u8>>,
    pub timing: Timing,
}

/// How often to ping and how long to wait before reconnecting. Tests shorten them.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub ping_every: Duration,
    /// Pings in a row without a Pong before the connection counts as dead.
    pub missed_pings: u32,
    pub first_retry: Duration,
    pub max_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            ping_every: Duration::from_secs(20),
            missed_pings: 3,
            first_retry: Duration::from_secs(1),
            max_retry: Duration::from_secs(30),
        }
    }
}

/// Frames waiting to go out. A call waits when it's full, so a slow connection slows reading
/// the sources instead of growing memory.
const OUTBOX: usize = 256;

/// Stays attached to the gateway until `shutdown` resolves. Returns an error only when the
/// gateway refuses the collector's key, which no retry can fix.
pub async fn run(collector: Collector, shutdown: impl Future<Output = ()>) -> Result<()> {
    let boot_id = random_hex(16)?;
    let timing = collector.timing;
    let mut shutdown = std::pin::pin!(shutdown);
    let mut retry = timing.first_retry;
    loop {
        let started = Instant::now();
        let result = tokio::select! {
            result = attach(&collector, &boot_id) => result,
            _ = &mut shutdown => return Ok(()),
        };
        match result {
            Ok(()) => tracing::info!("the gateway closed the connection"),
            Err(e) if refused(&e) => {
                return Err(e.context("the gateway refused this collector's key; enroll it again"));
            }
            // The gateway's own message says what's wrong; tonic's code name only adds noise.
            Err(e) => match e.downcast_ref::<Status>() {
                Some(status) => tracing::warn!("gateway connection: {}", status.message()),
                None => tracing::warn!("gateway connection: {e:#}"),
            },
        }
        // A connection that stayed up for a while was healthy, so start the backoff over.
        if started.elapsed() > timing.max_retry {
            retry = timing.first_retry;
        }
        tokio::select! {
            _ = tokio::time::sleep(jitter(retry)) => {}
            _ = &mut shutdown => return Ok(()),
        }
        retry = (retry * 2).min(timing.max_retry);
    }
}

fn refused(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Status>()
        .is_some_and(|status| matches!(status.code(), Code::Unauthenticated | Code::PermissionDenied))
}

/// One connection, until it drops.
async fn attach(collector: &Collector, boot_id: &str) -> Result<()> {
    let enrollment = &collector.enrollment;
    let channel = telelog_proto::auth::channel(&enrollment.gateway, collector.ca_pem.as_deref()).await?;
    let mut client = CollectorServiceClient::with_interceptor(channel, BearerToken::new(Some(&enrollment.key))?);
    let (out, outbox) = mpsc::channel(OUTBOX);
    out.send(frame(Frame::Hello(Hello {
        version: crate::VERSION.to_string(),
        host: crate::host_name(),
        boot_id: boot_id.to_string(),
        key_id: String::new(),
    })))
    .await?;
    let mut inbound = client.attach(ReceiverStream::new(outbox)).await?.into_inner();
    tracing::info!(gateway = %enrollment.gateway, collector = %enrollment.collector_id, "attached");

    // Calls in progress. Dropping the set when the connection ends aborts them all.
    let mut tasks = JoinSet::new();
    let mut calls: HashMap<u64, AbortHandle> = HashMap::new();
    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + collector.timing.ping_every,
        collector.timing.ping_every,
    );
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let (mut unanswered, mut nonce) = (0, 0);
    loop {
        tokio::select! {
            message = inbound.message() => {
                let Some(received) = message? else {
                    return Ok(());
                };
                match received.frame {
                    Some(gateway_frame::Frame::Config(config)) => {
                        tracing::info!(store = config.store, "configured by the gateway");
                        if config.store {
                            tracing::warn!("the gateway asked for storage, which this collector doesn't send yet");
                        }
                    }
                    Some(gateway_frame::Frame::Open(open)) => {
                        let (sources, out, call) = (collector.sources.clone(), out.clone(), open.call);
                        let handle = tasks.spawn(async move {
                            crate::calls::answer(&sources, open, &out).await;
                            call
                        });
                        calls.insert(call, handle);
                    }
                    Some(gateway_frame::Frame::Close(close)) => {
                        if let Some(handle) = calls.remove(&close.call) {
                            handle.abort();
                        }
                    }
                    Some(gateway_frame::Frame::Ping(Ping { nonce })) => {
                        out.send(frame(Frame::Pong(Pong { nonce }))).await?;
                    }
                    Some(gateway_frame::Frame::Pong(_)) => unanswered = 0,
                    Some(gateway_frame::Frame::Ack(_)) | None => {}
                }
            }
            Some(Ok(call)) = tasks.join_next() => {
                calls.remove(&call);
            }
            _ = ping.tick() => {
                if unanswered >= collector.timing.missed_pings {
                    bail!("the gateway stopped answering pings");
                }
                unanswered += 1;
                nonce += 1;
                out.send(frame(Frame::Ping(Ping { nonce }))).await?;
            }
        }
    }
}

/// Between half and all of `wait`, so collectors that lost the gateway together don't all come
/// back at the same moment.
fn jitter(wait: Duration) -> Duration {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    let fraction = f64::from(u32::from_le_bytes(bytes)) / f64::from(u32::MAX);
    wait.mul_f64(0.5 + 0.5 * fraction)
}

fn random_hex(len: usize) -> Result<String> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("reading system randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_stays_between_half_and_all() {
        for _ in 0..100 {
            let wait = jitter(Duration::from_secs(10));
            assert!(
                wait >= Duration::from_secs(5) && wait <= Duration::from_secs(10),
                "{wait:?}"
            );
        }
    }
}
