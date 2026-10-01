//! Tails container logs through the Docker Engine API.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::query_parameters::{EventsOptions, ListContainersOptionsBuilder, LogsOptionsBuilder};
use futures::stream::{self, BoxStream, SelectAll, StreamExt};
use telelog_core::{Level, LogRecord, SourceKind, Stream, Target};

#[derive(Clone)]
pub struct DockerSource {
    docker: Docker,
}

impl DockerSource {
    /// Connects using `DOCKER_HOST` or the platform's default socket.
    pub fn connect() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults().context("connecting to Docker")?;
        Ok(DockerSource { docker })
    }

    pub async fn list_targets(&self, all: bool) -> Result<Vec<Target>> {
        let options = ListContainersOptionsBuilder::default().all(all).build();
        let containers = self
            .docker
            .list_containers(Some(options))
            .await
            .context("listing containers")?;

        Ok(containers
            .into_iter()
            .filter_map(|c| {
                let id = c.id?;
                let name = c
                    .names
                    .and_then(|n| n.into_iter().next())
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_else(|| id.chars().take(12).collect());
                let labels = target_labels(&id, c.image.as_deref(), c.labels.unwrap_or_default());
                Some(Target {
                    id,
                    name,
                    source: SourceKind::Docker,
                    state: c.state.map(|s| s.to_string()).unwrap_or_default(),
                    labels,
                })
            })
            .collect())
    }

    /// Streams `backlog` historical lines, then follows the container until it stops.
    pub fn tail(&self, target: &Target, backlog: u32) -> BoxStream<'static, Result<LogRecord>> {
        self.follow_logs(target, LogsOptionsBuilder::default().tail(&backlog.to_string()))
    }

    /// Follows a container from `since` (Unix seconds), for containers that just started.
    fn tail_since(&self, target: &Target, since: i64) -> BoxStream<'static, Result<LogRecord>> {
        let since = i32::try_from(since).unwrap_or(i32::MAX);
        self.follow_logs(target, LogsOptionsBuilder::default().since(since))
    }

    fn follow_logs(&self, target: &Target, options: LogsOptionsBuilder) -> BoxStream<'static, Result<LogRecord>> {
        let options = options.follow(true).stdout(true).stderr(true).timestamps(true).build();
        let origin = target.name.clone();
        let labels = target.labels.clone();

        self.docker
            .logs(&target.id, Some(options))
            .flat_map(move |chunk| {
                let records: Vec<Result<LogRecord>> = match chunk {
                    Ok(output) => parse_output(output, &origin, &labels).into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(anyhow::Error::new(e).context(format!("tailing {origin}")))],
                };
                stream::iter(records)
            })
            .boxed()
    }

    /// Merges the tails of several targets into one stream.
    pub fn tail_many(&self, targets: &[Target], backlog: u32) -> BoxStream<'static, Result<LogRecord>> {
        stream::select_all(targets.iter().map(|t| self.tail(t, backlog))).boxed()
    }
}

impl DockerSource {
    /// Tails every running container and keeps adding containers as they start.
    pub async fn tail_live(&self, backlog: u32) -> Result<BoxStream<'static, Result<LogRecord>>> {
        // Subscribe before listing so a container that starts in between isn't missed.
        let filters = HashMap::from([
            ("type".to_string(), vec!["container".to_string()]),
            ("event".to_string(), vec!["start".to_string()]),
        ]);
        let events = self
            .docker
            .events(Some(EventsOptions {
                filters: Some(filters),
                ..Default::default()
            }))
            .map(|event| {
                let event = event.context("watching Docker events")?;
                Ok(event
                    .actor
                    .and_then(started_target)
                    .map(|target| (target, event.time.unwrap_or_else(unix_now))))
            })
            .boxed();

        let running = self.list_targets(false).await?;
        let mut live = LiveTail {
            docker: self.clone(),
            events: Some(events),
            tails: SelectAll::new(),
            listed: running.iter().map(|t| t.id.clone()).collect(),
            listed_at: unix_now(),
        };
        for target in &running {
            live.tails.push(self.tail(target, backlog));
        }
        Ok(live.boxed())
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Builds a target from a container `start` event. Event attributes carry the container's
/// name, image and labels.
fn started_target(actor: bollard::models::EventActor) -> Option<Target> {
    let id = actor.id?;
    let mut attributes = actor.attributes.unwrap_or_default();
    let name = attributes
        .remove("name")
        .unwrap_or_else(|| id.chars().take(12).collect());
    let image = attributes.remove("image");
    let labels = target_labels(&id, image.as_deref(), attributes);
    Some(Target {
        id,
        name,
        source: SourceKind::Docker,
        state: "running".into(),
        labels,
    })
}

/// A container that started, and when (Unix seconds).
type StartEvents = BoxStream<'static, Result<Option<(Target, i64)>>>;

/// Merges the logs of all tailed containers, adding a new tail whenever a container starts.
struct LiveTail {
    docker: DockerSource,
    events: Option<StartEvents>,
    tails: SelectAll<BoxStream<'static, Result<LogRecord>>>,
    /// Containers tailed from the initial listing, and when that listing happened. A start
    /// event for one of them at or before `listed_at` is the same start, already covered.
    listed: HashSet<String>,
    listed_at: i64,
}

impl futures::Stream for LiveTail {
    type Item = Result<LogRecord>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        while let Some(events) = this.events.as_mut() {
            match events.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(Some((target, since))))) => {
                    if since <= this.listed_at && this.listed.contains(&target.id) {
                        continue;
                    }
                    tracing::info!(container = %target.name, "container started, tailing");
                    this.tails.push(this.docker.tail_since(&target, since));
                }
                Poll::Ready(Some(Ok(None))) => {}
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => this.events = None,
                Poll::Pending => break,
            }
        }
        match this.tails.poll_next_unpin(cx) {
            // No containers right now; keep waiting for start events.
            Poll::Ready(None) if this.events.is_some() => Poll::Pending,
            other => other,
        }
    }
}

/// The few labels worth showing on every line. Raw container labels are often dozens of
/// build annotations, so only the Compose project and service are carried over.
fn target_labels(id: &str, image: Option<&str>, container: HashMap<String, String>) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert("container_id".to_string(), id.chars().take(12).collect());
    if let Some(image) = image {
        labels.insert("image".to_string(), image.to_string());
    }
    for (key, short) in [
        ("com.docker.compose.project", "compose_project"),
        ("com.docker.compose.service", "compose_service"),
    ] {
        if let Some(value) = container.get(key) {
            labels.insert(short.to_string(), value.clone());
        }
    }
    labels
}

fn parse_output(output: LogOutput, origin: &str, labels: &BTreeMap<String, String>) -> Vec<LogRecord> {
    let (stream, bytes) = match output {
        LogOutput::StdErr { message } => (Stream::Stderr, message),
        LogOutput::StdOut { message } | LogOutput::Console { message } => (Stream::Stdout, message),
        LogOutput::StdIn { .. } => return Vec::new(),
    };

    // A TTY container's chunk can hold several lines, each with its own timestamp.
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (timestamp, body) = split_timestamp(line);
            LogRecord {
                timestamp,
                source: SourceKind::Docker,
                origin: origin.to_string(),
                stream,
                level: Level::sniff(body),
                body: body.to_string(),
                labels: labels.clone(),
            }
        })
        .collect()
}

/// Docker prefixes each line with an RFC 3339 timestamp when `timestamps=true`.
fn split_timestamp(line: &str) -> (SystemTime, &str) {
    if let Some((ts, rest)) = line.split_once(' ')
        && let Ok(time) = humantime::parse_rfc3339(ts)
    {
        return (time, rest);
    }
    (SystemTime::now(), line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn builds_target_from_start_event() {
        let actor = bollard::models::EventActor {
            id: Some("0123456789abcdef".into()),
            attributes: Some(HashMap::from([
                ("name".to_string(), "api".to_string()),
                ("image".to_string(), "nginx:1.27".to_string()),
                ("com.docker.compose.project".to_string(), "shop".to_string()),
            ])),
        };
        let target = started_target(actor).unwrap();
        assert_eq!(target.name, "api");
        assert_eq!(target.state, "running");
        assert_eq!(target.labels["image"], "nginx:1.27");
        assert_eq!(target.labels["compose_project"], "shop");
    }

    #[test]
    fn keeps_only_useful_labels() {
        let container = HashMap::from([
            ("com.docker.compose.service".to_string(), "api".to_string()),
            ("org.opencontainers.image.revision".to_string(), "abc".to_string()),
        ]);
        let labels = target_labels("0123456789abcdef", Some("alpine"), container);
        assert_eq!(labels["container_id"], "0123456789ab");
        assert_eq!(labels["image"], "alpine");
        assert_eq!(labels["compose_service"], "api");
        assert_eq!(labels.len(), 3);
    }

    #[test]
    fn splits_docker_timestamp_prefix() {
        let (ts, body) = split_timestamp("2026-10-01T12:00:00.123456789Z ERROR boom");
        assert_eq!(body, "ERROR boom");
        let secs = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(secs.subsec_nanos(), 123_456_789);
    }

    #[test]
    fn keeps_line_without_timestamp() {
        let (_, body) = split_timestamp("no timestamp here");
        assert_eq!(body, "no timestamp here");
    }

    #[test]
    fn parses_multi_line_tty_chunk() {
        let output = LogOutput::Console {
            message: "2026-10-01T12:00:00Z a\r\n2026-10-01T12:00:01Z WARN b\r\n".into(),
        };
        let records = parse_output(output, "web", &BTreeMap::new());
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].body, "WARN b");
        assert_eq!(records[1].level, Level::Warn);
        assert_eq!(
            records[1].timestamp.duration_since(records[0].timestamp).unwrap(),
            Duration::from_secs(1)
        );
    }
}
