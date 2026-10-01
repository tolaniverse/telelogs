//! Long-term log storage in an object storage bucket you own (S3, R2, MinIO, GCS, or a local
//! directory). The server archives every line it sees; the app queries it for history older
//! than its in-memory buffer.

mod chunk;
pub mod ingest;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use object_store::aws::AmazonS3Builder;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use serde::{Deserialize, Serialize};
use telelog_core::{Filter, LogRecord};

/// Most chunks fetched at once while answering a query.
const FETCH_CONCURRENCY: usize = 8;
/// Objects counted before storage stats give up and report a lower bound.
const STATS_SCAN_LIMIT: u64 = 1_000_000;
const CHECKPOINT: &str = "_checkpoint.json";

/// What the ingest loop has done lately, for the Storage page.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub last_flush: Option<SystemTime>,
    pub last_error: Option<String>,
    pub lines_archived: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub objects: u64,
    pub bytes: u64,
    /// Start of the oldest archived day.
    pub oldest: Option<SystemTime>,
    /// True when the bucket held more objects than were counted.
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    /// Unix nanoseconds of the newest archived line.
    last: i64,
}

pub struct Archive {
    store: Arc<dyn ObjectStore>,
    /// `<prefix>/v1`: everything this format writes lives under it.
    root: Path,
    /// The bucket URL without credentials, for display.
    pub location: String,
    pub provider: &'static str,
    pub flush_every: Duration,
    /// Days to keep; 0 keeps everything.
    pub retention_days: u32,
    status: Mutex<Status>,
}

impl Archive {
    /// Opens `s3://bucket/prefix`, `gs://bucket/prefix`, `file:///path` or `memory://`.
    /// S3 and GCS read credentials from the standard environment variables; for R2 or MinIO
    /// set `AWS_ENDPOINT` (and `AWS_ALLOW_HTTP=true` for a plain-HTTP endpoint).
    pub fn open(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url).with_context(|| format!("invalid archive URL {url:?}"))?;
        let host = parsed.host_str().unwrap_or_default();
        let prefix = parsed.path().trim_matches('/');
        let (store, provider, prefix): (Arc<dyn ObjectStore>, _, _) = match parsed.scheme() {
            "s3" => {
                let store = AmazonS3Builder::from_env()
                    .with_url(format!("s3://{host}"))
                    .build()
                    .context("configuring S3")?;
                let provider = if std::env::var_os("AWS_ENDPOINT").is_some() {
                    "S3-compatible"
                } else {
                    "Amazon S3"
                };
                (Arc::new(store), provider, prefix)
            }
            "gs" => {
                let store = GoogleCloudStorageBuilder::from_env()
                    .with_url(format!("gs://{host}"))
                    .build()
                    .context("configuring Google Cloud Storage")?;
                (Arc::new(store), "Google Cloud Storage", prefix)
            }
            "file" => {
                let dir = parsed
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file URL {url:?}"))?;
                std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
                (Arc::new(LocalFileSystem::new_with_prefix(&dir)?), "Local disk", "")
            }
            "memory" => (Arc::new(InMemory::new()), "In memory", ""),
            other => bail!("unsupported archive URL scheme {other:?}; use s3://, gs://, file:// or memory://"),
        };
        let location = match parsed.scheme() {
            "file" | "memory" => format!("{}://{}", parsed.scheme(), parsed.path()),
            scheme if prefix.is_empty() => format!("{scheme}://{host}"),
            scheme => format!("{scheme}://{host}/{prefix}"),
        };
        Ok(Self::with_store(store, prefix, location, provider))
    }

    pub fn with_store(store: Arc<dyn ObjectStore>, prefix: &str, location: String, provider: &'static str) -> Self {
        let root = if prefix.is_empty() {
            Path::from("v1")
        } else {
            Path::from(prefix).join("v1")
        };
        Archive {
            store,
            root,
            location,
            provider,
            flush_every: Duration::from_secs(60),
            retention_days: 0,
            status: Mutex::default(),
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn record_error(&self, error: &anyhow::Error) {
        self.status.lock().unwrap().last_error = Some(format!("{error:#}"));
    }

    /// Writes one chunk and moves the checkpoint forward. `records` must not be empty.
    pub async fn write_chunk(&self, records: &[LogRecord]) -> Result<()> {
        let first = records.iter().map(|r| r.timestamp).min().context("empty chunk")?;
        let last = records.iter().map(|r| r.timestamp).max().context("empty chunk")?;
        let (date, hour) = chunk::partition(first);
        let name = chunk::file_name(chunk::unix_millis(first), chunk::unix_millis(last), &random_id()?);
        let path = self.root.clone().join(date).join(hour).join(name);
        let body = chunk::encode(records)?;
        self.store
            .put(&path, PutPayload::from(Bytes::from(body)))
            .await
            .with_context(|| format!("writing {path}"))?;

        let checkpoint = serde_json::to_vec(&Checkpoint {
            last: chunk::unix_nanos(last),
        })?;
        self.store
            .put(
                &self.root.clone().join(CHECKPOINT),
                PutPayload::from(Bytes::from(checkpoint)),
            )
            .await
            .context("writing checkpoint")?;

        let mut status = self.status.lock().unwrap();
        status.last_flush = Some(SystemTime::now());
        status.last_error = None;
        status.lines_archived += records.len() as u64;
        Ok(())
    }

    /// The newest archived line's time, if anything has been archived.
    pub async fn checkpoint(&self) -> Result<Option<SystemTime>> {
        match self.store.get(&self.root.clone().join(CHECKPOINT)).await {
            Ok(object) => {
                let checkpoint: Checkpoint = serde_json::from_slice(&object.bytes().await?)?;
                Ok(Some(chunk::from_unix_nanos(checkpoint.last)))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(e).context("reading checkpoint"),
        }
    }

    /// Child "directories" of `dir`, e.g. the day or hour partitions.
    async fn list_dirs(&self, dir: &Path) -> Result<Vec<String>> {
        let listing = self.store.list_with_delimiter(Some(dir)).await?;
        Ok(listing
            .common_prefixes
            .iter()
            .filter_map(|p| p.filename().map(str::to_string))
            .collect())
    }

    /// The newest `limit` archived lines matching `filter`, oldest first.
    pub async fn query(&self, filter: &Filter, limit: usize) -> Result<Vec<LogRecord>> {
        let to = filter.to.unwrap_or_else(SystemTime::now);
        // Chunks live under the hour they start in and may run past it (flushes are at most
        // an hour apart), so look one hour earlier than the range.
        let from_key = filter.from.map(|from| {
            let (date, hour) = chunk::partition(from.checked_sub(Duration::from_secs(3600)).unwrap_or(from));
            format!("{date}/{hour}")
        });
        let (to_date, to_hour) = chunk::partition(to);
        let to_key = format!("{to_date}/{to_hour}");
        let (from_ms, to_ms) = (filter.from.map_or(i64::MIN, chunk::unix_millis), chunk::unix_millis(to));

        let mut dates = self.list_dirs(&self.root).await?;
        dates.retain(|d| d.as_str() <= to_date.as_str() && from_key.as_ref().is_none_or(|k| k[..10] <= *d.as_str()));
        dates.sort_unstable_by(|a, b| b.cmp(a));

        let mut found = Vec::new();
        // Chunks within an hour aren't ordered, and one starting an hour earlier can still hold
        // later lines, so keep reading one more hour after reaching the limit.
        let mut hours_after_limit = 0;
        'dates: for date in dates {
            let date_dir = self.root.clone().join(date.as_str());
            let mut hours = self.list_dirs(&date_dir).await?;
            hours.sort_unstable_by(|a, b| b.cmp(a));
            for hour in hours {
                let key = format!("{date}/{hour}");
                if key > to_key || from_key.as_ref().is_some_and(|k| key < *k) {
                    continue;
                }
                let listing = self
                    .store
                    .list_with_delimiter(Some(&date_dir.clone().join(hour.as_str())))
                    .await?;
                let chunks: Vec<Path> = listing
                    .objects
                    .into_iter()
                    .filter(|o| {
                        o.location
                            .filename()
                            .and_then(chunk::parse_file_name)
                            .is_some_and(|(start, end)| end >= from_ms && start <= to_ms)
                    })
                    .map(|o| o.location)
                    .collect();
                let mut fetches = futures::stream::iter(chunks)
                    .map(|path| async move {
                        let bytes = self.store.get(&path).await?.bytes().await?;
                        chunk::decode(&bytes).with_context(|| format!("reading {path}"))
                    })
                    .buffer_unordered(FETCH_CONCURRENCY);
                while let Some(records) = fetches.try_next().await? {
                    found.extend(records.into_iter().filter(|r| filter.matches(r)));
                }
                if found.len() >= limit {
                    hours_after_limit += 1;
                    if hours_after_limit > 1 {
                        break 'dates;
                    }
                }
            }
        }

        found.sort_by_key(|r| r.timestamp);
        if found.len() > limit {
            found.drain(..found.len() - limit);
        }
        Ok(found)
    }

    /// Deletes whole days older than `keep_days` before `now`. Returns the objects removed.
    pub async fn sweep(&self, keep_days: u32, now: SystemTime) -> Result<u64> {
        let cutoff = now
            .checked_sub(Duration::from_secs(u64::from(keep_days) * 86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let (cutoff_date, _) = chunk::partition(cutoff);
        let mut removed = 0;
        for date in self.list_dirs(&self.root).await? {
            if date >= cutoff_date {
                continue;
            }
            let paths = self
                .store
                .list(Some(&self.root.clone().join(date.as_str())))
                .map_ok(|o| o.location)
                .boxed();
            let mut deletions = self.store.delete_stream(paths);
            while let Some(result) = deletions.next().await {
                result?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub async fn stats(&self) -> Result<Stats> {
        let mut stats = Stats::default();
        let mut objects = self.store.list(Some(&self.root));
        while let Some(object) = objects.try_next().await? {
            if object.location.filename() == Some(CHECKPOINT) {
                continue;
            }
            if stats.objects == STATS_SCAN_LIMIT {
                stats.truncated = true;
                break;
            }
            stats.objects += 1;
            stats.bytes += object.size;
        }
        let mut dates = self.list_dirs(&self.root).await?;
        dates.sort_unstable();
        stats.oldest = dates.first().and_then(|d| {
            chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().into())
        });
        Ok(stats)
    }
}

fn random_id() -> Result<String> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("reading system randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
