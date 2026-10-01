//! The log model shared by every Telelogs component.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Where a log line came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl Level {
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

/// Case-insensitive substring filter applied to records.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    needle: String,
    pub min_level: Option<Level>,
}

impl Filter {
    pub fn new(text: &str) -> Self {
        Filter { needle: text.to_lowercase(), min_level: None }
    }

    pub fn is_empty(&self) -> bool {
        self.needle.is_empty() && self.min_level.is_none()
    }

    pub fn matches(&self, record: &LogRecord) -> bool {
        if let Some(min) = self.min_level
            && record.level < min
        {
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
    fn filter_respects_min_level() {
        let mut f = Filter::new("");
        f.min_level = Some(Level::Warn);
        assert!(f.matches(&record("a", "ERROR boom")));
        assert!(!f.matches(&record("a", "INFO ok")));
    }
}
