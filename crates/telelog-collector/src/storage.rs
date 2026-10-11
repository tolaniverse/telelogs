//! Storage: while the gateway asks for it, every line goes to the gateway in numbered batches,
//! kept until acknowledged, so a dropped connection or a gateway restart loses nothing.
//!
//! The reader runs for as long as storage is on, across reconnects; each connection sends
//! whatever the spool holds that it hasn't sent yet.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use telelog_proto::cloud::v1::{Batch, Config, LogRecords, batch};
use telelog_proto::v1;
use telelog_sources::{LiveStart, Sources};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// A batch goes out after this long, or sooner when it fills up.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);
const BATCH_LINES: usize = 5_000;
/// Lines held while the gateway doesn't acknowledge them; past this the oldest batches go.
const MAX_PENDING_LINES: usize = 500_000;
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// Batches waiting for the gateway's acknowledgement, in order.
pub struct Spool {
    inner: Mutex<Pending>,
    /// The newest batch's seq, so a connection can wait for more.
    newest: watch::Sender<u64>,
}

#[derive(Default)]
struct Pending {
    batches: VecDeque<Batch>,
    lines: usize,
    next_seq: u64,
}

impl Default for Spool {
    fn default() -> Self {
        Spool {
            inner: Mutex::default(),
            newest: watch::Sender::new(0),
        }
    }
}

impl Spool {
    pub fn push(&self, records: Vec<v1::LogRecord>) {
        if records.is_empty() {
            return;
        }
        let mut pending = self.inner.lock().unwrap();
        pending.next_seq += 1;
        pending.lines += records.len();
        let seq = pending.next_seq;
        pending.batches.push_back(Batch {
            seq,
            body: Some(batch::Body::Records(LogRecords { records })),
        });
        let mut dropped = 0;
        while pending.lines > MAX_PENDING_LINES && pending.batches.len() > 1 {
            let oldest = pending.batches.pop_front().expect("more than one");
            pending.lines -= lines(&oldest);
            dropped += lines(&oldest);
        }
        drop(pending);
        if dropped > 0 {
            tracing::error!(
                dropped,
                "the gateway hasn't stored lines for too long; dropped the oldest"
            );
        }
        self.newest.send_replace(seq);
    }

    /// The gateway stored everything up to and including `seq`.
    pub fn ack(&self, seq: u64) {
        let mut pending = self.inner.lock().unwrap();
        while pending.batches.front().is_some_and(|b| b.seq <= seq) {
            let batch = pending.batches.pop_front().expect("checked");
            pending.lines -= lines(&batch);
        }
    }

    /// Batches newer than `seq`, to send.
    pub fn after(&self, seq: u64) -> Vec<Batch> {
        let pending = self.inner.lock().unwrap();
        pending.batches.iter().filter(|b| b.seq > seq).cloned().collect()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.newest.subscribe()
    }

    fn clear(&self) {
        let mut pending = self.inner.lock().unwrap();
        pending.batches.clear();
        pending.lines = 0;
    }
}

fn lines(batch: &Batch) -> usize {
    match &batch.body {
        Some(batch::Body::Records(records)) => records.records.len(),
        _ => 0,
    }
}

/// Turns storage on and off as the gateway's Config says.
pub struct Storage {
    pub spool: Arc<Spool>,
    sources: Sources,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl Storage {
    pub fn new(sources: Sources) -> Self {
        Storage {
            spool: Arc::default(),
            sources,
            reader: Mutex::default(),
        }
    }

    pub fn configure(&self, config: &Config) {
        let mut reader = self.reader.lock().unwrap();
        if config.store {
            // Already reading since an earlier connection: carry on, with nothing missed.
            if reader.as_ref().is_some_and(|task| !task.is_finished()) {
                return;
            }
            let resume_from = config.resume_from.map(|t| telelog_proto::from_timestamp(Some(t)));
            tracing::info!(resume_from = ?resume_from, "storage on");
            *reader = Some(tokio::spawn(read(
                self.sources.clone(),
                resume_from,
                self.spool.clone(),
            )));
        } else if let Some(task) = reader.take() {
            tracing::info!("storage off");
            task.abort();
            self.spool.clear();
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Some(task) = self.reader.lock().unwrap().take() {
            task.abort();
        }
    }
}

/// Reads every source into the spool, from just after `resume_from` (or from now).
async fn read(sources: Sources, mut resume_from: Option<SystemTime>, spool: Arc<Spool>) {
    loop {
        // Docker's `since` counts whole seconds, so lines at or before the resume point are
        // skipped here instead.
        let start = match resume_from {
            Some(t) => LiveStart::Since(t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)),
            None => LiveStart::Backlog(0),
        };
        let mut records = match sources.tail_live(start).await {
            Ok(records) => records,
            Err(e) => {
                tracing::warn!("storage can't read the sources: {e:#}");
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
        };
        let mut batch: Vec<v1::LogRecord> = Vec::new();
        let mut tick = tokio::time::interval(FLUSH_EVERY);
        tick.tick().await;
        loop {
            tokio::select! {
                next = records.next() => match next {
                    Some(Ok(record)) => {
                        if resume_from.is_some_and(|after| record.timestamp <= after) {
                            continue;
                        }
                        batch.push(record.into());
                        if batch.len() >= BATCH_LINES {
                            push(&spool, &mut batch, &mut resume_from);
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!("storage stopped reading: {e:#}");
                        break;
                    }
                    None => break,
                },
                _ = tick.tick() => push(&spool, &mut batch, &mut resume_from),
            }
        }
        push(&spool, &mut batch, &mut resume_from);
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Moves the batch into the spool; a restart of the reader then resumes after its newest line.
fn push(spool: &Spool, batch: &mut Vec<v1::LogRecord>, resume_from: &mut Option<SystemTime>) {
    if let Some(newest) = batch
        .iter()
        .filter_map(|r| r.timestamp)
        .max_by_key(|t| (t.seconds, t.nanos))
    {
        let newest = telelog_proto::from_timestamp(Some(newest));
        *resume_from = Some(resume_from.map_or(newest, |r| r.max(newest)));
    }
    spool.push(std::mem::take(batch));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(body: &str) -> v1::LogRecord {
        v1::LogRecord {
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn keeps_batches_until_acknowledged() {
        let spool = Spool::default();
        spool.push(vec![line("a")]);
        spool.push(vec![line("b"), line("c")]);
        spool.push(Vec::new());
        assert_eq!(spool.after(0).iter().map(|b| b.seq).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(spool.after(1).len(), 1);

        spool.ack(1);
        // A new connection resends what's left.
        assert_eq!(spool.after(0).iter().map(|b| b.seq).collect::<Vec<_>>(), [2]);
        spool.ack(5);
        assert!(spool.after(0).is_empty());
        spool.push(vec![line("d")]);
        assert_eq!(spool.after(0)[0].seq, 3, "numbers keep going up");
    }

    #[test]
    fn drops_the_oldest_when_too_much_waits() {
        let spool = Spool::default();
        let big = || vec![line("x"); MAX_PENDING_LINES / 2];
        spool.push(big());
        spool.push(big());
        spool.push(big());
        let seqs: Vec<_> = spool.after(0).iter().map(|b| b.seq).collect();
        assert_eq!(seqs, [2, 3]);
    }

    #[tokio::test]
    async fn tells_connections_about_new_batches() {
        let spool = Spool::default();
        let mut newest = spool.subscribe();
        spool.push(vec![line("a")]);
        newest.changed().await.unwrap();
        assert_eq!(*newest.borrow(), 1);
    }
}
