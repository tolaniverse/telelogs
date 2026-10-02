//! Log sources. Shared by the server and, for local mode, the desktop app.

pub mod docker;
pub mod kubernetes;

use anyhow::{Result, bail};
use futures::stream::{self, BoxStream, StreamExt};
use telelog_core::{LogRecord, SourceKind, Target};

pub use docker::{DockerSource, LiveStart};
pub use kubernetes::KubeSource;

/// Every source a server reads from, behind one interface.
#[derive(Clone, Default)]
pub struct Sources {
    pub docker: Option<DockerSource>,
    pub kubernetes: Option<KubeSource>,
}

impl Sources {
    pub fn is_empty(&self) -> bool {
        self.docker.is_none() && self.kubernetes.is_none()
    }

    /// Targets from every source. A source that fails is skipped with a warning, so one
    /// unreachable runtime doesn't hide the others; it's an error only if all of them fail.
    pub async fn list_targets(&self, all: bool) -> Result<Vec<Target>> {
        let (docker, kubernetes) = futures::join!(
            async {
                match &self.docker {
                    Some(d) => Some(d.list_targets(all).await),
                    None => None,
                }
            },
            async {
                match &self.kubernetes {
                    Some(k) => Some(k.list_targets(all).await),
                    None => None,
                }
            },
        );
        combine([("Docker", docker), ("Kubernetes", kubernetes)], |a, b| {
            a.extend(b);
        })
        .map(|t| t.unwrap_or_default())
    }

    /// Follows everything running, and whatever starts later, across all sources.
    pub async fn tail_live(&self, start: LiveStart) -> Result<BoxStream<'static, Result<LogRecord>>> {
        let (docker, kubernetes) = futures::join!(
            async {
                match &self.docker {
                    Some(d) => Some(d.tail_live(start).await),
                    None => None,
                }
            },
            async {
                match &self.kubernetes {
                    Some(k) => Some(k.tail_live(start).await),
                    None => None,
                }
            },
        );
        let streams = combine(
            [
                ("Docker", docker.map(|r| r.map(|s| vec![s]))),
                ("Kubernetes", kubernetes.map(|r| r.map(|s| vec![s]))),
            ],
            |a, b| a.extend(b),
        )?
        .unwrap_or_default();
        Ok(stream::select_all(streams).boxed())
    }

    /// Follows the given targets, each through the source it came from.
    pub fn tail_many(&self, targets: &[Target], backlog: u32) -> BoxStream<'static, Result<LogRecord>> {
        let of = |kind| targets.iter().filter(|t| t.source == kind).cloned().collect::<Vec<_>>();
        let mut streams = Vec::new();
        if let Some(d) = &self.docker {
            streams.push(d.tail_many(&of(SourceKind::Docker), backlog));
        }
        if let Some(k) = &self.kubernetes {
            streams.push(k.tail_many(&of(SourceKind::Kubernetes), backlog));
        }
        stream::select_all(streams).boxed()
    }

    /// State changes from every source.
    pub fn watch_targets(&self) -> BoxStream<'static, Result<Target>> {
        let mut streams = Vec::new();
        if let Some(d) = &self.docker {
            streams.push(d.watch_targets());
        }
        if let Some(k) = &self.kubernetes {
            streams.push(k.watch_targets());
        }
        if streams.is_empty() {
            return stream::pending().boxed();
        }
        stream::select_all(streams).boxed()
    }
}

/// Merges per-source results: failures are logged and skipped unless every source failed.
fn combine<T>(results: [(&str, Option<Result<T>>); 2], merge: impl Fn(&mut T, T)) -> Result<Option<T>> {
    let mut merged: Option<T> = None;
    let mut errors = Vec::new();
    for (name, result) in results {
        match result {
            None => {}
            Some(Ok(value)) => match &mut merged {
                Some(m) => merge(m, value),
                None => merged = Some(value),
            },
            Some(Err(e)) => {
                tracing::warn!("{name}: {e:#}");
                errors.push(format!("{name}: {e:#}"));
            }
        }
    }
    if merged.is_none() && !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_skips_a_failed_source() {
        let merged = combine(
            [
                ("Docker", Some(Ok(vec![1]))),
                ("Kubernetes", Some(Err(anyhow::anyhow!("down")))),
            ],
            |a, b| a.extend(b),
        )
        .unwrap();
        assert_eq!(merged, Some(vec![1]));
    }

    #[test]
    fn combine_fails_when_every_source_fails() {
        let err = combine::<Vec<u8>>(
            [
                ("Docker", Some(Err(anyhow::anyhow!("no socket")))),
                ("Kubernetes", None),
            ],
            |a, b| a.extend(b),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Docker: no socket"));
    }

    #[test]
    fn combine_with_no_sources_is_empty() {
        assert_eq!(
            combine::<Vec<u8>>([("Docker", None), ("Kubernetes", None)], |a, b| a.extend(b)).unwrap(),
            None
        );
    }
}
