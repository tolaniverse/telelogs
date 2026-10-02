//! Background tasks: tail every container into the archive, and enforce retention.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use futures::StreamExt;
use telelog_core::LogRecord;
use telelog_sources::{LiveStart, Sources};

use super::Archive;

/// Lines per chunk before flushing early, so a burst doesn't build one huge object.
const MAX_CHUNK_LINES: usize = 50_000;
/// Lines held while the bucket is unreachable; past this the oldest are dropped.
const MAX_PENDING_LINES: usize = 500_000;
const RETRY_DELAY: Duration = Duration::from_secs(5);
const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// Archives every source's output until the task is dropped, reconnecting as needed.
pub async fn run(archive: Arc<Archive>, sources: Sources) {
    loop {
        if let Err(e) = ingest_once(&archive, &sources).await {
            tracing::warn!("archive ingest stopped: {e:#}");
            archive.record_error(&e);
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Deletes days past the retention period every hour.
pub async fn sweep_forever(archive: Arc<Archive>) {
    if archive.retention_days == 0 {
        return;
    }
    let mut tick = tokio::time::interval(SWEEP_EVERY);
    loop {
        tick.tick().await;
        match archive.sweep(archive.retention_days, SystemTime::now()).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "retention removed archived chunks"),
            Err(e) => {
                tracing::warn!("retention sweep failed: {e:#}");
                archive.record_error(&e);
            }
        }
    }
}

async fn ingest_once(archive: &Archive, sources: &Sources) -> Result<()> {
    // Resume from the newest archived line, so lines written while the server was down still
    // land in the bucket. Docker's `since` has second precision; exact duplicates are skipped.
    let resume_after = archive.checkpoint().await?;
    let start = match resume_after {
        Some(t) => LiveStart::Since(t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)),
        None => LiveStart::Backlog(0),
    };
    let mut records = sources.tail_live(start).await?;
    tracing::info!(location = %archive.location, "archiving logs");

    let mut pending: Vec<LogRecord> = Vec::new();
    let mut tick = tokio::time::interval(archive.flush_every);
    tick.tick().await;
    loop {
        tokio::select! {
            next = records.next() => match next {
                Some(Ok(record)) => {
                    if resume_after.is_some_and(|after| record.timestamp <= after) {
                        continue;
                    }
                    pending.push(record);
                    if pending.len() >= MAX_CHUNK_LINES {
                        flush(archive, &mut pending).await;
                    }
                }
                Some(Err(e)) => {
                    flush(archive, &mut pending).await;
                    return Err(e);
                }
                None => {
                    flush(archive, &mut pending).await;
                    return Ok(());
                }
            },
            _ = tick.tick() => flush(archive, &mut pending).await,
        }
    }
}

/// Writes pending lines; on failure keeps them for the next attempt, within a cap.
async fn flush(archive: &Archive, pending: &mut Vec<LogRecord>) {
    if pending.is_empty() {
        return;
    }
    match archive.write_chunk(pending).await {
        Ok(()) => pending.clear(),
        Err(e) => {
            tracing::warn!(lines = pending.len(), "archive flush failed, will retry: {e:#}");
            archive.record_error(&e);
            if pending.len() > MAX_PENDING_LINES {
                let dropped = pending.len() - MAX_PENDING_LINES;
                pending.drain(..dropped);
                tracing::error!(dropped, "archive unreachable for too long; dropped the oldest lines");
            }
        }
    }
}
