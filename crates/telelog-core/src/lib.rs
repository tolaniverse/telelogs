//! The log model shared by every Telelogs component.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Where a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    Docker,
    Kubernetes,
    Vm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Unknown,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl SourceKind {
    /// The inverse of [`SourceKind::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "docker" => Some(SourceKind::Docker),
            "kubernetes" => Some(SourceKind::Kubernetes),
            "vm" => Some(SourceKind::Vm),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Docker => "docker",
            SourceKind::Kubernetes => "kubernetes",
            SourceKind::Vm => "vm",
        }
    }
}

impl Stream {
    /// The inverse of [`Stream::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stdout" => Some(Stream::Stdout),
            "stderr" => Some(Stream::Stderr),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        }
    }
}

impl Level {
    /// The inverse of [`Level::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unknown" => Some(Level::Unknown),
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Unknown => "unknown",
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    /// Best-effort level detection from an unstructured line.
    pub fn sniff(line: &str) -> Level {
        let head = &line[..line.len().min(64)];
        let upper = head.to_ascii_uppercase();
        if upper.contains("ERROR") || upper.contains("FATAL") || upper.contains("PANIC") {
            Level::Error
        } else if upper.contains("WARN") {
            Level::Warn
        } else if upper.contains("INFO") {
            Level::Info
        } else if upper.contains("DEBUG") {
            Level::Debug
        } else if upper.contains("TRACE") {
            Level::Trace
        } else {
            Level::Unknown
        }
    }
}

/// One log line plus its metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    pub timestamp: SystemTime,
    pub source: SourceKind,
    /// Human-readable origin, e.g. a container name or `namespace/pod/container`.
    pub origin: String,
    pub stream: Stream,
    pub level: Level,
    pub body: String,
    pub labels: BTreeMap<String, String>,
}

/// A log-producing target that can be tailed (a container, a pod, a VM file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub source: SourceKind,
    pub state: String,
    pub labels: BTreeMap<String, String>,
}

/// Case-insensitive substring filter applied to records, optionally limited to a time range.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    needle: String,
    pub min_level: Option<Level>,
    /// Inclusive lower bound on `LogRecord::timestamp`.
    pub from: Option<SystemTime>,
    /// Inclusive upper bound on `LogRecord::timestamp`.
    pub to: Option<SystemTime>,
}

impl Filter {
    pub fn new(text: &str) -> Self {
        Filter {
            needle: text.to_lowercase(),
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.needle.is_empty() && self.min_level.is_none() && self.from.is_none() && self.to.is_none()
    }

    pub fn matches(&self, record: &LogRecord) -> bool {
        if let Some(min) = self.min_level
            && record.level < min
        {
            return false;
        }
        if self.from.is_some_and(|from| record.timestamp < from) || self.to.is_some_and(|to| record.timestamp > to) {
            return false;
        }
        self.needle.is_empty()
            || record.body.to_lowercase().contains(&self.needle)
            || record.origin.to_lowercase().contains(&self.needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(origin: &str, body: &str) -> LogRecord {
        LogRecord {
            timestamp: SystemTime::UNIX_EPOCH,
            source: SourceKind::Docker,
            origin: origin.into(),
            stream: Stream::Stdout,
            level: Level::sniff(body),
            body: body.into(),
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn sniffs_levels() {
        assert_eq!(Level::sniff("2026-10-01 ERROR db down"), Level::Error);
        assert_eq!(Level::sniff("[warn] slow query"), Level::Warn);
        assert_eq!(Level::sniff("hello"), Level::Unknown);
    }

    #[test]
    fn filter_matches_body_and_origin_case_insensitively() {
        let r = record("api-1", "GET /Health 200");
        assert!(Filter::new("health").matches(&r));
        assert!(Filter::new("API").matches(&r));
        assert!(!Filter::new("postgres").matches(&r));
        assert!(Filter::new("").matches(&r));
    }

    #[test]
    fn enum_names_round_trip() {
        for kind in [SourceKind::Docker, SourceKind::Kubernetes, SourceKind::Vm] {
            assert_eq!(SourceKind::parse(kind.as_str()), Some(kind));
        }
        for stream in [Stream::Stdout, Stream::Stderr] {
            assert_eq!(Stream::parse(stream.as_str()), Some(stream));
        }
        for level in [
            Level::Unknown,
            Level::Trace,
            Level::Debug,
            Level::Info,
            Level::Warn,
            Level::Error,
        ] {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
        assert_eq!(Level::parse("nope"), None);
    }

    #[test]
    fn filter_respects_time_range() {
        use std::time::Duration;
        let at = |secs| {
            let mut r = record("a", "x");
            r.timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
            r
        };
        let mut f = Filter::new("");
        f.from = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        f.to = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(20));
        assert!(!f.is_empty());
        assert!(!f.matches(&at(9)));
        assert!(f.matches(&at(10)));
        assert!(f.matches(&at(20)));
        assert!(!f.matches(&at(21)));
    }

    #[test]
    fn filter_respects_min_level() {
        let mut f = Filter::new("");
        f.min_level = Some(Level::Warn);
        assert!(f.matches(&record("a", "ERROR boom")));
        assert!(!f.matches(&record("a", "INFO ok")));
    }
}
