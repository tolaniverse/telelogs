//! Which sources are on, kept between launches for each server.
//!
//! Kubernetes pod names change with every rollout, so a namespace's checkbox is saved by
//! namespace and decides for pods that haven't been seen before. A source toggled on its own is
//! saved by name and overrides its namespace.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::namespace_of;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selections {
    /// Kubernetes namespaces turned all on or all off with their checkbox.
    #[serde(default)]
    pub namespaces: BTreeMap<String, bool>,
    /// Sources toggled one at a time, by name.
    #[serde(default)]
    pub targets: BTreeMap<String, bool>,
}

impl Selections {
    /// The saved choice for a source, if there is one.
    pub fn choice(&self, kubernetes: bool, name: &str) -> Option<bool> {
        self.targets.get(name).copied().or_else(|| {
            kubernetes
                .then(|| self.namespaces.get(namespace_of(name)).copied())
                .flatten()
        })
    }

    pub fn set_target(&mut self, name: &str, on: bool) {
        self.targets.insert(name.to_string(), on);
    }

    /// A namespace's checkbox sets all its pods, so it replaces their own choices.
    pub fn set_namespace(&mut self, namespace: &str, on: bool) {
        let prefix = format!("{namespace}/");
        self.targets.retain(|name, _| !name.starts_with(&prefix));
        self.namespaces.insert(namespace.to_string(), on);
    }

    /// The selections saved for `server`, or none if there are none or the file can't be read.
    pub fn load(server: &str) -> Self {
        let Some(path) = file() else {
            return Self::default();
        };
        read_all(&path).remove(server).unwrap_or_default()
    }

    /// Saves these selections for `server`, keeping other servers' as they are.
    pub fn save(&self, server: &str) {
        let Some(path) = file() else {
            return;
        };
        let mut all = read_all(&path);
        all.insert(server.to_string(), self.clone());
        if let Err(e) = write_all(&path, &all) {
            eprintln!("telelogs: saving source selections to {}: {e}", path.display());
        }
    }
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("telelogs").join("selections.json"))
}

fn read_all(path: &PathBuf) -> BTreeMap<String, Selections> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes through a temporary file, so a crash mid-write can't leave half a file behind.
fn write_all(path: &PathBuf, all: &BTreeMap<String, Selections>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(all)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::Selections;

    #[test]
    fn namespace_decides_for_new_pods() {
        let mut s = Selections::default();
        s.set_namespace("kube-system", false);
        assert_eq!(s.choice(true, "kube-system/coredns-7d9f-x2k4p/coredns"), Some(false));
        assert_eq!(s.choice(true, "default/auth-svc-54bf-t2xv7/auth"), None);
        // Docker names aren't namespaced, even when they contain a slash.
        assert_eq!(s.choice(false, "kube-system/odd"), None);
    }

    #[test]
    fn a_source_overrides_its_namespace_until_the_namespace_is_set_again() {
        let mut s = Selections::default();
        s.set_namespace("kafka", false);
        s.set_target("kafka/kafka-0/kafka", true);
        assert_eq!(s.choice(true, "kafka/kafka-0/kafka"), Some(true));
        assert_eq!(s.choice(true, "kafka/kafka-1/kafka"), Some(false));

        s.set_namespace("kafka", true);
        assert!(s.targets.is_empty());
        assert_eq!(s.choice(true, "kafka/kafka-0/kafka"), Some(true));
    }

    #[test]
    fn docker_sources_are_saved_by_name() {
        let mut s = Selections::default();
        s.set_target("postgres", false);
        assert_eq!(s.choice(false, "postgres"), Some(false));
        assert_eq!(s.choice(false, "redis"), None);
    }

    #[test]
    fn round_trips_through_json() {
        let mut s = Selections::default();
        s.set_namespace("cert-manager", false);
        s.set_target("api-1", true);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Selections>(&json).unwrap(), s);
        // A file from an older or newer version with fields missing still loads.
        assert_eq!(serde_json::from_str::<Selections>("{}").unwrap(), Selections::default());
    }
}
