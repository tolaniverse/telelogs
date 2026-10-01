//! Tails container logs through the Docker Engine API.

use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::query_parameters::{ListContainersOptionsBuilder, LogsOptionsBuilder};
use futures::stream::{self, BoxStream, StreamExt};
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
        let options = LogsOptionsBuilder::default()
            .follow(true)
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .tail(&backlog.to_string())
            .build();
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
