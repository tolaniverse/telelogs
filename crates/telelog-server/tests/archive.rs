//! The archive against in-memory and on-disk stores: writing, querying, retention, stats.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use telelog_core::{Filter, Level, LogRecord, SourceKind, Stream};
use telelog_server::archive::Archive;

const HOUR: u64 = 3600;
const DAY: u64 = 86_400;
/// 2026-09-01T00:00:00Z
const BASE: u64 = 1_788_220_800;

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(BASE + secs)
}

fn line(secs: u64, level: Level, body: &str) -> LogRecord {
    LogRecord {
        timestamp: at(secs),
        source: SourceKind::Docker,
        origin: "api".into(),
        stream: Stream::Stdout,
        level,
        body: body.into(),
        labels: BTreeMap::new(),
    }
}

/// Ten lines per hour over 3 days, written one chunk per hour; every 10th line is an error.
async fn filled() -> Archive {
    let archive = Archive::open("memory://").unwrap();
    for hour in 0..72 {
        let records: Vec<_> = (0..10)
            .map(|i| {
                let secs = hour * HOUR + i * 60;
                let level = if i == 9 { Level::Error } else { Level::Info };
                line(secs, level, &format!("request {hour}-{i}"))
            })
            .collect();
        archive.write_chunk(&records).await.unwrap();
    }
    archive
}

fn range(from: u64, to: u64) -> Filter {
    let mut filter = Filter::new("");
    filter.from = Some(at(from));
    filter.to = Some(at(to));
    filter
}

#[tokio::test]
async fn query_returns_lines_in_range_oldest_first() {
    let archive = filled().await;
    let got = archive.query(&range(10 * HOUR, 12 * HOUR - 1), 10_000).await.unwrap();
    assert_eq!(got.len(), 20);
    assert!(got.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
    assert_eq!(got.first().unwrap().body, "request 10-0");
    assert_eq!(got.last().unwrap().body, "request 11-9");
}

#[tokio::test]
async fn query_keeps_the_newest_lines_when_limited() {
    let archive = filled().await;
    let got = archive.query(&Filter::new(""), 25).await.unwrap();
    assert_eq!(got.len(), 25);
    assert_eq!(got.last().unwrap().body, "request 71-9");
    assert_eq!(got.first().unwrap().body, "request 69-5");
}

#[tokio::test]
async fn query_applies_text_and_level_filters_across_days() {
    let archive = filled().await;
    let mut filter = Filter::new("");
    filter.min_level = Some(Level::Error);
    assert_eq!(archive.query(&filter, 10_000).await.unwrap().len(), 72);

    let got = archive.query(&Filter::new("request 50-3"), 10_000).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].timestamp, at(50 * HOUR + 180));
}

#[tokio::test]
async fn checkpoint_tracks_the_newest_line() {
    let archive = Archive::open("memory://").unwrap();
    assert_eq!(archive.checkpoint().await.unwrap(), None);
    archive
        .write_chunk(&[line(5, Level::Info, "a"), line(9, Level::Info, "b")])
        .await
        .unwrap();
    assert_eq!(archive.checkpoint().await.unwrap(), Some(at(9)));
    assert_eq!(archive.status().lines_archived, 2);
}

#[tokio::test]
async fn sweep_deletes_whole_days_past_retention() {
    let archive = filled().await;
    // Data covers Sep 1-3. "Now" is Sep 4 00:00; keeping 1 day puts the cutoff at Sep 3 00:00,
    // so Sep 1 and Sep 2 (48 hourly chunks) are deleted and Sep 3 stays.
    let removed = archive.sweep(1, at(3 * DAY)).await.unwrap();
    assert_eq!(removed, 48);
    let remaining = archive.query(&Filter::new(""), 10_000).await.unwrap();
    assert_eq!(remaining.len(), 24 * 10);
    assert_eq!(remaining.first().unwrap().body, "request 48-0");

    let stats = archive.stats().await.unwrap();
    assert_eq!(stats.objects, 24);
    assert_eq!(stats.oldest, Some(at(2 * DAY)));
}

#[tokio::test]
async fn reopening_a_directory_finds_earlier_chunks() {
    let dir = std::env::temp_dir().join(format!("telelog-archive-{}", std::process::id()));
    let url = format!("file://{}", dir.display());
    Archive::open(&url)
        .unwrap()
        .write_chunk(&[line(0, Level::Warn, "kept on disk")])
        .await
        .unwrap();

    let reopened = Archive::open(&url).unwrap();
    let got = reopened.query(&Filter::new("kept"), 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].level, Level::Warn);
    assert_eq!(reopened.checkpoint().await.unwrap(), Some(at(0)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejects_unknown_schemes() {
    assert!(Archive::open("ftp://example.com/logs").is_err());
}
