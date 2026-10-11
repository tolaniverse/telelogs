//! Which sources are on, kept between launches for each server.
//!
//! Kubernetes pod names change with every rollout, so a namespace's checkbox is saved by
//! namespace and decides for pods that haven't been seen before. A source toggled on its own is
//! saved by name and overrides its namespace.

use std::collections::BTreeMap;

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

    /// The selections saved under `key`. A saved server is keyed by its id; before servers were
    /// saved, selections were keyed by URL, so `legacy` names the URL to fall back on.
    pub fn load(key: &str, legacy: Option<&str>) -> Self {
        let mut all = read_all();
        all.remove(key)
            .or_else(|| legacy.and_then(|url| all.remove(url)))
            .unwrap_or_default()
    }

    /// Saves these selections under `key`, keeping other servers' as they are.
    pub fn save(&self, key: &str) {
        let mut all = read_all();
        all.insert(key.to_string(), self.clone());
        crate::config::write(FILE, &all);
    }

    /// Drops the selections saved under `key`, for a server that was removed.
    pub fn forget(key: &str) {
        let mut all = read_all();
        if all.remove(key).is_some() {
            crate::config::write(FILE, &all);
        }
    }
}

const FILE: &str = "selections.json";

fn read_all() -> BTreeMap<String, Selections> {
    crate::config::read(FILE)
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
