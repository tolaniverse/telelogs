//! Tails pod logs through the Kubernetes API.
//!
//! Every container of every pod is a target named `namespace/pod/container`. Logs are followed
//! with the API's `follow`, and a pod watch adds containers as they start or restart.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result, bail};
use futures::stream::{self, BoxStream, StreamExt};
use futures::{AsyncBufReadExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::jiff::Timestamp;
use kube::api::{ListParams, LogParams};
use kube::config::KubeConfigOptions;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, Config};
use telelog_core::{Level, LogRecord, SourceKind, Stream, Target};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};

use crate::LiveStart;
use crate::docker::split_timestamp;

/// Lines buffered between the per-container tails and the consumer.
const CHANNEL_LINES: usize = 4096;

#[derive(Clone)]
pub struct KubeSource {
    client: Client,
    /// Namespaces to read; empty means all of them.
    namespaces: Vec<String>,
    /// Label selector pods must match, e.g. `app=api,tier!=batch`.
    selector: Option<String>,
    /// The cluster's API address, for logs.
    pub cluster: String,
}

impl KubeSource {
    /// Connects with `context` from the kubeconfig, or, without one, the way `kubectl` would:
    /// the in-cluster service account when running in a pod, else the current context.
    pub async fn connect(context: Option<&str>, namespaces: Vec<String>, selector: Option<String>) -> Result<Self> {
        let config = match context {
            Some(context) => Config::from_kubeconfig(&KubeConfigOptions {
                context: Some(context.to_string()),
                ..Default::default()
            })
            .await
            .with_context(|| format!("loading kubeconfig context {context:?}"))?,
            None => Config::infer().await.context("finding a Kubernetes config")?,
        };
        let cluster = config.cluster_url.to_string();
        let client = Client::try_from(config).context("creating the Kubernetes client")?;
        Ok(KubeSource {
            client,
            namespaces,
            selector: selector.filter(|s| !s.trim().is_empty()),
            cluster,
        })
    }

    fn list_params(&self) -> ListParams {
        match &self.selector {
            Some(selector) => ListParams::default().labels(selector),
            None => ListParams::default(),
        }
    }

    fn watch_config(&self) -> watcher::Config {
        match &self.selector {
            Some(selector) => watcher::Config::default().labels(selector),
            None => watcher::Config::default(),
        }
    }

    fn pods(&self) -> Vec<Api<Pod>> {
        if self.namespaces.is_empty() {
            vec![Api::all(self.client.clone())]
        } else {
            self.namespaces
                .iter()
                .map(|ns| Api::namespaced(self.client.clone(), ns))
                .collect()
        }
    }

    /// Every container of every pod; only running ones unless `all`.
    pub async fn list_targets(&self, all: bool) -> Result<Vec<Target>> {
        let mut targets = Vec::new();
        for api in self.pods() {
            let pods = api.list(&self.list_params()).await.context("listing pods")?;
            targets.extend(
                pods.items
                    .iter()
                    .flat_map(|pod| pod_targets(pod, None))
                    .filter(|(t, _)| all || t.state == "running")
                    .map(|(t, _)| t),
            );
        }
        Ok(targets)
    }

    /// Streams `backlog` historical lines, then follows the container until it stops.
    pub fn tail(&self, target: &Target, backlog: u32) -> BoxStream<'static, Result<LogRecord>> {
        self.follow(
            target,
            LogParams {
                tail_lines: Some(i64::from(backlog)),
                ..Default::default()
            },
        )
    }

    /// Merges the tails of several targets into one stream.
    pub fn tail_many(&self, targets: &[Target], backlog: u32) -> BoxStream<'static, Result<LogRecord>> {
        stream::select_all(targets.iter().map(|t| self.tail(t, backlog))).boxed()
    }

    fn follow(&self, target: &Target, mut params: LogParams) -> BoxStream<'static, Result<LogRecord>> {
        let Some((namespace, pod, container)) = split_name(&target.name) else {
            let name = target.name.clone();
            return stream::once(async move { bail!("{name:?} is not a namespace/pod/container name") }).boxed();
        };
        params.container = Some(container.to_string());
        params.follow = true;
        params.timestamps = true;
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let (pod, origin, labels) = (pod.to_string(), target.name.clone(), target.labels.clone());
        let lines = stream::once(async move {
            let lines = api
                .log_stream(&pod, &params)
                .await
                .with_context(|| format!("reading logs of {origin}"))?
                .lines();
            anyhow::Ok(lines.map(move |line| {
                let line = line.with_context(|| format!("reading logs of {origin}"))?;
                Ok(record(&line, &origin, &labels))
            }))
        })
        .try_flatten()
        .try_filter(|r| std::future::ready(!r.body.trim().is_empty()))
        .boxed();
        crate::chain_levels(lines)
    }

    /// Streams a target, with its new `state`, whenever a container's state changes or its pod
    /// is deleted. Every container is reported once when the watch starts.
    pub fn watch_targets(&self) -> BoxStream<'static, Result<Target>> {
        let events = stream::select_all(
            self.pods()
                .into_iter()
                .map(|api| watcher(api, self.watch_config()).default_backoff().boxed()),
        );
        events
            .scan(HashMap::<String, String>::new(), |states, event| {
                let changed: Vec<Result<Target>> = match event {
                    Ok(watcher::Event::Apply(pod) | watcher::Event::InitApply(pod)) => pod_targets(&pod, None)
                        .into_iter()
                        .map(|(t, _)| t)
                        .filter(|t| states.insert(t.id.clone(), t.state.clone()).as_ref() != Some(&t.state))
                        .map(Ok)
                        .collect(),
                    Ok(watcher::Event::Delete(pod)) => pod_targets(&pod, Some("removed"))
                        .into_iter()
                        .map(|(t, _)| {
                            states.remove(&t.id);
                            Ok(t)
                        })
                        .collect(),
                    Ok(_) => Vec::new(),
                    Err(e) => vec![Err(anyhow::Error::new(e).context("watching pods"))],
                };
                std::future::ready(Some(stream::iter(changed)))
            })
            .flatten()
            .boxed()
    }

    /// Tails every running container and keeps adding containers as they start or restart.
    pub async fn tail_live(&self, start: LiveStart) -> Result<BoxStream<'static, Result<LogRecord>>> {
        // Fail now, with a clear error, if the cluster can't be reached or pods can't be listed.
        for api in self.pods() {
            api.list(&self.list_params().limit(1)).await.context("listing pods")?;
        }
        let (tx, rx) = mpsc::channel(CHANNEL_LINES);
        let driver = tokio::spawn(drive_live(self.clone(), start, tx));
        Ok(LiveStream { rx, driver }.boxed())
    }
}

/// Watches pods and starts a tail for each running container it hasn't tailed yet. A container
/// that restarts has a new start time, so it gets a new tail.
async fn drive_live(source: KubeSource, start: LiveStart, tx: mpsc::Sender<Result<LogRecord>>) {
    let apis = source.pods();
    let mut listing = apis.len();
    let mut events = stream::select_all(
        apis.into_iter()
            .map(|api| watcher(api, source.watch_config()).default_backoff().boxed()),
    );
    // Dropping the set (when the consumer goes away and this task is aborted) stops every tail.
    let mut tails = JoinSet::new();
    let mut tailed: HashSet<String> = HashSet::new();

    while let Some(event) = events.next().await {
        let pod = match event {
            Ok(watcher::Event::InitApply(pod) | watcher::Event::Apply(pod)) => pod,
            Ok(watcher::Event::InitDone) => {
                listing = listing.saturating_sub(1);
                continue;
            }
            Ok(watcher::Event::Delete(pod)) => {
                let prefix = format!("{}/", target_prefix(&pod));
                tailed.retain(|key| !key.starts_with(&prefix));
                continue;
            }
            Ok(watcher::Event::Init) => continue,
            Err(e) => {
                tracing::warn!("watching pods: {e}");
                continue;
            }
        };
        for (target, started) in pod_targets(&pod, None) {
            let Some(started) = started.filter(|_| target.state == "running") else {
                continue;
            };
            if !tailed.insert(format!("{}@{started}", target.id)) {
                continue;
            }
            // Containers found by the first listing start where the caller asked; later ones
            // start from the moment they did, so nothing they printed is missed.
            let params = match (listing > 0, start) {
                (true, LiveStart::Backlog(n)) => LogParams {
                    tail_lines: Some(i64::from(n)),
                    ..Default::default()
                },
                (true, LiveStart::Since(secs)) => LogParams {
                    since_time: Some(Timestamp::from_second(secs).unwrap_or(started).max(started)),
                    ..Default::default()
                },
                (false, _) => {
                    tracing::info!(container = %target.name, "container started, tailing");
                    LogParams {
                        since_time: Some(started),
                        ..Default::default()
                    }
                }
            };
            let mut lines = source.follow(&target, params);
            let tx = tx.clone();
            tails.spawn(async move {
                while let Some(line) = lines.next().await {
                    match line {
                        Ok(record) => {
                            if tx.send(Ok(record)).await.is_err() {
                                return;
                            }
                        }
                        // A container that is starting or gone ends its tail; the watch starts a
                        // new one if it runs again.
                        Err(e) => {
                            tracing::debug!("{e:#}");
                            return;
                        }
                    }
                }
            });
        }
        // Reap finished tails so the set doesn't grow with every restart.
        while tails.try_join_next().is_some() {}
    }
}

/// Lines from every tailed container. Dropping it stops the watch and all tails.
struct LiveStream {
    rx: mpsc::Receiver<Result<LogRecord>>,
    driver: JoinHandle<()>,
}

impl futures::Stream for LiveStream {
    type Item = Result<LogRecord>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for LiveStream {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

fn target_prefix(pod: &Pod) -> String {
    format!(
        "{}/{}",
        pod.metadata.namespace.as_deref().unwrap_or("default"),
        pod.metadata.name.as_deref().unwrap_or_default()
    )
}

/// `namespace/pod/container` back into its parts.
fn split_name(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
        .filter(|(ns, pod, c)| !ns.is_empty() && !pod.is_empty() && !c.is_empty())
}

/// One target per app container of `pod` (init and debug containers are left out), with the
/// time each running container started. `state` overrides the containers' own state.
fn pod_targets(pod: &Pod, state: Option<&str>) -> Vec<(Target, Option<Timestamp>)> {
    let Some(spec) = &pod.spec else {
        return Vec::new();
    };
    let prefix = target_prefix(pod);
    let status = pod.status.as_ref();
    let statuses: HashMap<&str, _> = status
        .and_then(|s| s.container_statuses.as_ref())
        .into_iter()
        .flatten()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let phase = status.and_then(|s| s.phase.as_deref()).unwrap_or("Pending");
    let pod_labels = pod.metadata.labels.as_ref();
    let app = pod_labels.and_then(|l| {
        l.get("app.kubernetes.io/name")
            .or_else(|| l.get("app"))
            .or_else(|| l.get("k8s-app"))
            .cloned()
    });

    spec.containers
        .iter()
        .map(|container| {
            let container_status = statuses.get(container.name.as_str());
            let running = container_status
                .and_then(|s| s.state.as_ref())
                .and_then(|s| s.running.as_ref());
            let own_state = match container_status.and_then(|s| s.state.as_ref()) {
                Some(s) if s.running.is_some() => "running".to_string(),
                Some(s) if s.terminated.is_some() => "exited".to_string(),
                Some(s) if s.waiting.is_some() => "waiting".to_string(),
                _ => phase.to_lowercase(),
            };
            let name = format!("{prefix}/{}", container.name);
            let mut labels = BTreeMap::new();
            let namespace = pod.metadata.namespace.clone().unwrap_or_else(|| "default".into());
            labels.insert("namespace".to_string(), namespace);
            labels.insert("pod".to_string(), pod.metadata.name.clone().unwrap_or_default());
            labels.insert("container".to_string(), container.name.clone());
            if let Some(node) = &spec.node_name {
                labels.insert("node".to_string(), node.clone());
            }
            if let Some(image) = &container.image {
                labels.insert("image".to_string(), image.clone());
            }
            if let Some(app) = &app {
                labels.insert("app".to_string(), app.clone());
            }
            let target = Target {
                id: name.clone(),
                name,
                source: SourceKind::Kubernetes,
                state: state.map_or(own_state, String::from),
                labels,
            };
            (target, running.and_then(|r| r.started_at.as_ref()).map(|t| t.0))
        })
        .collect()
}

/// A line from the logs API, which prefixes each line with an RFC 3339 timestamp when asked.
/// The API merges stdout and stderr, so every line is reported as stdout.
fn record(line: &str, origin: &str, labels: &BTreeMap<String, String>) -> LogRecord {
    let (timestamp, body) = split_timestamp(line);
    LogRecord {
        timestamp,
        source: SourceKind::Kubernetes,
        origin: origin.to_string(),
        stream: Stream::Stdout,
        level: Level::sniff(body),
        body: body.to_string(),
        labels: labels.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateRunning, ContainerStateWaiting, ContainerStatus, PodSpec, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

    fn pod() -> Pod {
        let status = |name: &str, state: ContainerState| ContainerStatus {
            name: name.into(),
            state: Some(state),
            ..Default::default()
        };
        Pod {
            metadata: ObjectMeta {
                name: Some("api-7d9f".into()),
                namespace: Some("shop".into()),
                labels: Some(BTreeMap::from([("app.kubernetes.io/name".into(), "api".into())])),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("node-1".into()),
                containers: vec![
                    Container {
                        name: "web".into(),
                        image: Some("nginx:1.27".into()),
                        ..Default::default()
                    },
                    Container {
                        name: "sidecar".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![
                    status(
                        "web",
                        ContainerState {
                            running: Some(ContainerStateRunning {
                                started_at: Some(Time(Timestamp::from_second(1_790_000_000).unwrap())),
                            }),
                            ..Default::default()
                        },
                    ),
                    status(
                        "sidecar",
                        ContainerState {
                            waiting: Some(ContainerStateWaiting {
                                reason: Some("CrashLoopBackOff".into()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ),
                ]),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn one_target_per_container() {
        let targets = pod_targets(&pod(), None);
        assert_eq!(targets.len(), 2);
        let (web, started) = &targets[0];
        assert_eq!(web.name, "shop/api-7d9f/web");
        assert_eq!(web.source, SourceKind::Kubernetes);
        assert_eq!(web.state, "running");
        assert_eq!(started.unwrap().as_second(), 1_790_000_000);
        assert_eq!(web.labels["namespace"], "shop");
        assert_eq!(web.labels["node"], "node-1");
        assert_eq!(web.labels["image"], "nginx:1.27");
        assert_eq!(web.labels["app"], "api");
        let (sidecar, started) = &targets[1];
        assert_eq!(sidecar.state, "waiting");
        assert!(started.is_none());
    }

    #[test]
    fn deleted_pods_are_removed() {
        assert!(
            pod_targets(&pod(), Some("removed"))
                .iter()
                .all(|(t, _)| t.state == "removed")
        );
    }

    #[test]
    fn splits_target_names() {
        assert_eq!(split_name("shop/api-7d9f/web"), Some(("shop", "api-7d9f", "web")));
        assert_eq!(split_name("api"), None);
        assert_eq!(split_name("shop//web"), None);
    }

    #[test]
    fn parses_log_lines() {
        let r = record(
            "2026-10-02T22:00:00.5Z ERROR payment failed",
            "shop/api-7d9f/web",
            &BTreeMap::new(),
        );
        assert_eq!(r.body, "ERROR payment failed");
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.source, SourceKind::Kubernetes);
    }
}
