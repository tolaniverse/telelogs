//! Generated gRPC types plus conversions to and from `telelog-core`.

use std::time::{Duration, SystemTime};

use telelog_core as core;

pub mod auth;

pub mod v1 {
    tonic::include_proto!("telelog.v1");
}

pub use v1::log_service_client::LogServiceClient;
pub use v1::log_service_server::{LogService, LogServiceServer};

impl From<core::SourceKind> for v1::SourceKind {
    fn from(s: core::SourceKind) -> Self {
        match s {
            core::SourceKind::Docker => v1::SourceKind::Docker,
            core::SourceKind::Kubernetes => v1::SourceKind::Kubernetes,
            core::SourceKind::Vm => v1::SourceKind::Vm,
        }
    }
}

impl From<v1::SourceKind> for core::SourceKind {
    fn from(s: v1::SourceKind) -> Self {
        match s {
            v1::SourceKind::Kubernetes => core::SourceKind::Kubernetes,
            v1::SourceKind::Vm => core::SourceKind::Vm,
            v1::SourceKind::Docker | v1::SourceKind::Unspecified => core::SourceKind::Docker,
        }
    }
}

impl From<core::Stream> for v1::Stream {
    fn from(s: core::Stream) -> Self {
        match s {
            core::Stream::Stdout => v1::Stream::Stdout,
            core::Stream::Stderr => v1::Stream::Stderr,
        }
    }
}

impl From<v1::Stream> for core::Stream {
    fn from(s: v1::Stream) -> Self {
        match s {
            v1::Stream::Stderr => core::Stream::Stderr,
            v1::Stream::Stdout | v1::Stream::Unspecified => core::Stream::Stdout,
        }
    }
}

impl From<core::Level> for v1::Level {
    fn from(l: core::Level) -> Self {
        match l {
            core::Level::Unknown => v1::Level::Unknown,
            core::Level::Trace => v1::Level::Trace,
            core::Level::Debug => v1::Level::Debug,
            core::Level::Info => v1::Level::Info,
            core::Level::Warn => v1::Level::Warn,
            core::Level::Error => v1::Level::Error,
        }
    }
}

impl From<v1::Level> for core::Level {
    fn from(l: v1::Level) -> Self {
        match l {
            v1::Level::Unknown => core::Level::Unknown,
            v1::Level::Trace => core::Level::Trace,
            v1::Level::Debug => core::Level::Debug,
            v1::Level::Info => core::Level::Info,
            v1::Level::Warn => core::Level::Warn,
            v1::Level::Error => core::Level::Error,
        }
    }
}

fn to_timestamp(t: SystemTime) -> prost_types::Timestamp {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    prost_types::Timestamp {
        seconds: d.as_secs() as i64,
        nanos: d.subsec_nanos() as i32,
    }
}

fn from_timestamp(t: Option<prost_types::Timestamp>) -> SystemTime {
    t.map(|t| SystemTime::UNIX_EPOCH + Duration::new(t.seconds.max(0) as u64, t.nanos.max(0) as u32))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

impl From<core::LogRecord> for v1::LogRecord {
    fn from(r: core::LogRecord) -> Self {
        v1::LogRecord {
            timestamp: Some(to_timestamp(r.timestamp)),
            source: v1::SourceKind::from(r.source) as i32,
            origin: r.origin,
            stream: v1::Stream::from(r.stream) as i32,
            level: v1::Level::from(r.level) as i32,
            body: r.body,
            labels: r.labels.into_iter().collect(),
        }
    }
}

impl From<v1::LogRecord> for core::LogRecord {
    fn from(r: v1::LogRecord) -> Self {
        core::LogRecord {
            timestamp: from_timestamp(r.timestamp),
            source: r.source().into(),
            stream: r.stream().into(),
            level: r.level().into(),
            origin: r.origin,
            body: r.body,
            labels: r.labels.into_iter().collect(),
        }
    }
}

impl From<core::Target> for v1::Target {
    fn from(t: core::Target) -> Self {
        v1::Target {
            id: t.id,
            name: t.name,
            source: v1::SourceKind::from(t.source) as i32,
            state: t.state,
            labels: t.labels.into_iter().collect(),
        }
    }
}

impl From<v1::Target> for core::Target {
    fn from(t: v1::Target) -> Self {
        core::Target {
            source: t.source().into(),
            id: t.id,
            name: t.name,
            state: t.state,
            labels: t.labels.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_record_round_trips() {
        let original = core::LogRecord {
            timestamp: SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 123),
            source: core::SourceKind::Docker,
            origin: "api".into(),
            stream: core::Stream::Stderr,
            level: core::Level::Warn,
            body: "slow".into(),
            labels: [("k".to_string(), "v".to_string())].into(),
        };
        let back: core::LogRecord = v1::LogRecord::from(original.clone()).into();
        assert_eq!(back.timestamp, original.timestamp);
        assert_eq!(back.stream, original.stream);
        assert_eq!(back.level, original.level);
        assert_eq!(back.labels, original.labels);
    }
}
