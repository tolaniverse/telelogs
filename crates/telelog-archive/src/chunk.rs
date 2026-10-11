//! The on-bucket format: zstd-compressed NDJSON, one log line per JSON object.
//!
//! Chunks are stored under `<root>/<YYYY-MM-DD>/<HH>/<start_ms>-<end_ms>-<id>.ndjson.zst`, keyed
//! by the UTC hour of their earliest line, so a time-range query only lists the hours it needs
//! and can skip chunks by name alone.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use telelog_core::{Level, LogRecord, SourceKind, Stream};

pub const EXTENSION: &str = ".ndjson.zst";

#[derive(Serialize, Deserialize)]
struct Line {
    /// Unix time in nanoseconds.
    ts: i64,
    src: String,
    origin: String,
    stream: String,
    level: String,
    body: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    labels: BTreeMap<String, String>,
}

pub fn unix_nanos(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

pub fn unix_millis(t: SystemTime) -> i64 {
    unix_nanos(t) / 1_000_000
}

pub fn from_unix_nanos(nanos: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(nanos.max(0) as u64)
}

pub fn encode(records: &[LogRecord]) -> Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 3)?;
    for r in records {
        let line = Line {
            ts: unix_nanos(r.timestamp),
            src: r.source.as_str().into(),
            origin: r.origin.clone(),
            stream: r.stream.as_str().into(),
            level: r.level.as_str().into(),
            body: r.body.clone(),
            labels: r.labels.clone(),
        };
        serde_json::to_writer(&mut encoder, &line)?;
        encoder.write_all(b"\n")?;
    }
    Ok(encoder.finish()?)
}

pub fn decode(bytes: &[u8]) -> Result<Vec<LogRecord>> {
    let raw = zstd::decode_all(bytes).context("decompressing chunk")?;
    raw.split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let line: Line = serde_json::from_slice(line).context("parsing archived line")?;
            Ok(LogRecord {
                timestamp: from_unix_nanos(line.ts),
                source: SourceKind::parse(&line.src).unwrap_or(SourceKind::Docker),
                origin: line.origin,
                stream: Stream::parse(&line.stream).unwrap_or(Stream::Stdout),
                level: Level::parse(&line.level).unwrap_or(Level::Unknown),
                body: line.body,
                labels: line.labels,
            })
        })
        .collect()
}

/// `YYYY-MM-DD` and `HH` (UTC) partition names for a time.
pub fn partition(t: SystemTime) -> (String, String) {
    let t: DateTime<Utc> = t.into();
    (t.format("%Y-%m-%d").to_string(), t.format("%H").to_string())
}

pub fn file_name(start_ms: i64, end_ms: i64, id: &str) -> String {
    format!("{start_ms}-{end_ms}-{id}{EXTENSION}")
}

/// The `(start_ms, end_ms)` a chunk covers, from its file name.
pub fn parse_file_name(name: &str) -> Option<(i64, i64)> {
    let stem = name.strip_suffix(EXTENSION)?;
    let mut parts = stem.splitn(3, '-');
    let start = parts.next()?.parse().ok()?;
    let end = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(secs: u64, body: &str) -> LogRecord {
        LogRecord {
            timestamp: UNIX_EPOCH + Duration::from_nanos(secs * 1_000_000_000 + 123_456_789),
            source: SourceKind::Docker,
            origin: "api".into(),
            stream: Stream::Stderr,
            level: Level::Warn,
            body: body.into(),
            labels: BTreeMap::from([("image".into(), "nginx".into())]),
        }
    }

    #[test]
    fn round_trips_records_exactly() {
        let records = vec![record(1, "first"), record(2, "multi\nline \"quoted\" ✓")];
        let decoded = decode(&encode(&records).unwrap()).unwrap();
        assert_eq!(decoded.len(), 2);
        for (a, b) in records.iter().zip(&decoded) {
            assert_eq!(a.timestamp, b.timestamp);
            assert_eq!(a.body, b.body);
            assert_eq!(a.level, b.level);
            assert_eq!(a.stream, b.stream);
            assert_eq!(a.labels, b.labels);
        }
    }

    #[test]
    fn file_names_carry_their_range() {
        let name = file_name(1000, 2000, "ab12");
        assert_eq!(parse_file_name(&name), Some((1000, 2000)));
        assert_eq!(parse_file_name("_checkpoint.json"), None);
        assert_eq!(parse_file_name("1-2.ndjson.zst"), None);
    }

    #[test]
    fn partitions_by_utc_hour() {
        let t = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert_eq!(partition(t), ("2026-09-21".into(), "14".into()));
    }
}
