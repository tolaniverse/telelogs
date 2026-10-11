//! Servers saved in the app and the Telelog Cloud sign-in, kept between launches in
//! `servers.json`. Their tokens are kept in the macOS Keychain, never in the file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::client::{Connection, Token};

const FILE: &str = "servers.json";
/// Keychain service the tokens are filed under, one item per server id.
const KEYCHAIN_SERVICE: &str = "telelogs";
/// Keychain item for the Telelog Cloud token. Server ids are hex, so it can't clash.
pub const CLOUD_TOKEN: &str = "telelog-cloud";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    /// Never changes: the token and the source selections are filed under it, so renaming a
    /// server or changing its URL keeps them.
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<PathBuf>,
}

impl Server {
    pub fn open(&self) -> Open {
        Open {
            saved: Some(self.clone()),
            cloud: None,
            connection: Connection {
                server: self.url.clone(),
                token: Token::keychain(&self.id),
                ca_cert: self.ca_cert.clone(),
                collector: None,
            },
        }
    }
}

/// Signed in to Telelog Cloud: the workspace's collectors can be opened like servers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudAccount {
    pub gateway: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<PathBuf>,
}

impl CloudAccount {
    /// The gateway itself, for listing collectors.
    pub fn connection(&self) -> Connection {
        Connection {
            server: self.gateway.clone(),
            token: Token::keychain(CLOUD_TOKEN),
            ca_cert: self.ca_cert.clone(),
            collector: None,
        }
    }

    pub fn open(&self, collector: CloudCollector) -> Open {
        let mut connection = self.connection();
        connection.collector = Some(collector.id.clone());
        Open {
            saved: None,
            cloud: Some(collector),
            connection,
        }
    }
}

/// A collector as the app remembers it between launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudCollector {
    pub id: String,
    pub name: String,
    pub host: String,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Servers {
    #[serde(default)]
    pub list: Vec<Server>,
    /// Id of the server opened most recently, which the next launch opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<CloudAccount>,
    /// The collector opened most recently, when that came after any saved server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cloud: Option<CloudCollector>,
}

impl Servers {
    pub fn load() -> Self {
        crate::config::read(FILE)
    }

    pub fn save(&self) {
        crate::config::write(FILE, self);
    }

    pub fn get(&self, id: &str) -> Option<&Server> {
        self.list.iter().find(|s| s.id == id)
    }

    pub fn find_url(&self, url: &str) -> Option<&Server> {
        let url = url.trim_end_matches('/');
        self.list.iter().find(|s| s.url == url)
    }

    /// What to open at launch: the server or collector used last, else the first saved server.
    pub fn initial(&self) -> Option<Open> {
        if let (Some(account), Some(collector)) = (&self.cloud, &self.last_cloud) {
            return Some(account.open(collector.clone()));
        }
        self.last
            .as_deref()
            .and_then(|id| self.get(id))
            .or_else(|| self.list.first())
            .map(Server::open)
    }

    /// Notes `open` as the one to reopen next launch. An unsaved server isn't remembered, nor
    /// a collector opened through another gateway than the signed-in one. True when that
    /// changed something to save.
    pub fn remember(&mut self, open: &Open) -> bool {
        let before = (self.last.clone(), self.last_cloud.clone());
        if let Some(server) = &open.saved {
            self.last = Some(server.id.clone());
            self.last_cloud = None;
        } else if let Some(collector) = &open.cloud
            && self
                .cloud
                .as_ref()
                .is_some_and(|account| account.gateway == open.connection.server)
        {
            self.last_cloud = Some(collector.clone());
        }
        (self.last.clone(), self.last_cloud.clone()) != before
    }

    /// Adds a server under a new id and returns it.
    pub fn add(&mut self, name: String, url: String, ca_cert: Option<PathBuf>) -> Server {
        let mut n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let id = loop {
            let id = format!("{n:x}");
            if self.get(&id).is_none() {
                break id;
            }
            n += 1;
        };
        let server = Server { id, name, url, ca_cert };
        self.list.push(server.clone());
        server
    }

    pub fn remove(&mut self, id: &str) {
        self.list.retain(|s| s.id != id);
        if self.last.as_deref() == Some(id) {
            self.last = None;
        }
    }
}

/// What the window is connected to: a saved server, a Telelog Cloud collector, or a server
/// given on the command line.
#[derive(Clone)]
pub struct Open {
    pub saved: Option<Server>,
    pub cloud: Option<CloudCollector>,
    pub connection: Connection,
}

impl Open {
    pub fn unsaved(connection: Connection) -> Self {
        Open {
            saved: None,
            cloud: None,
            connection,
        }
    }

    /// A server given on the command line and not saved yet.
    pub fn is_unsaved(&self) -> bool {
        self.saved.is_none() && self.cloud.is_none()
    }

    pub fn name(&self) -> String {
        match (&self.saved, &self.cloud) {
            (Some(server), _) => server.name.clone(),
            (None, Some(collector)) => collector.name.clone(),
            (None, None) if is_local(&self.connection.server) => "local server".into(),
            (None, None) => "unsaved server".into(),
        }
    }

    /// Where it is, as shown under the name: the server's address, or the collector's host.
    pub fn address(&self) -> &str {
        match &self.cloud {
            Some(collector) if !collector.host.is_empty() => &collector.host,
            Some(_) => "Telelog Cloud",
            None => address(&self.connection.server),
        }
    }

    /// What this server's source selections are saved under.
    pub fn key(&self) -> &str {
        match (&self.saved, &self.cloud) {
            (Some(server), _) => &server.id,
            (None, Some(collector)) => &collector.id,
            (None, None) => &self.connection.server,
        }
    }
}

/// The URL without its scheme, as shown in the sidebar and title bar.
pub fn address(url: &str) -> &str {
    url.trim_start_matches("http://").trim_start_matches("https://")
}

fn is_local(url: &str) -> bool {
    let address = address(url);
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|host| address == *host || address.starts_with(&format!("{host}:")))
}

/// A server URL as typed, trimmed and checked, or why it won't do.
pub fn check_url(url: &str) -> Result<String, &'static str> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("enter the server's URL");
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("the URL starts with http:// or https://");
    }
    if address(url).is_empty() {
        return Err("the URL needs a host");
    }
    Ok(url.to_string())
}

/// The token saved for a server, if there is one.
pub fn token(id: &str) -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, id).ok()?.get_password().ok()
}

/// Saves a server's token, or deletes it for `None`.
pub fn set_token(id: &str, token: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id).map_err(|e| e.to_string())?;
    let result = match token {
        Some(token) => entry.set_password(token),
        None => match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        },
    };
    result.map_err(|e| format!("Keychain: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_urls() {
        assert_eq!(
            check_url(" https://logs.example.com:7070/ "),
            Ok("https://logs.example.com:7070".into())
        );
        assert!(check_url("").is_err());
        assert!(check_url("logs.example.com:7070").is_err());
        assert!(check_url("https://").is_err());
    }

    #[test]
    fn names_unsaved_servers_by_where_they_are() {
        let open = |url: &str| {
            Open::unsaved(Connection {
                server: url.into(),
                token: Token::None,
                ca_cert: None,
                collector: None,
            })
        };
        assert_eq!(open("http://127.0.0.1:7070").name(), "local server");
        assert_eq!(open("http://localhost").name(), "local server");
        assert_eq!(open("https://localhost.example.com").name(), "unsaved server");
        assert_eq!(open("https://logs.example.com").key(), "https://logs.example.com");
    }

    #[test]
    fn opens_the_last_server_used() {
        let mut servers = Servers::default();
        let a = servers.add("a".into(), "http://a:7070".into(), None);
        let b = servers.add("b".into(), "http://b:7070".into(), None);
        assert_ne!(a.id, b.id);
        let initial = |servers: &Servers| servers.initial().map(|open| open.key().to_string());
        assert_eq!(initial(&servers), Some(a.id.clone()));
        servers.remember(&b.open());
        assert_eq!(initial(&servers), Some(b.id.clone()));
        assert_eq!(servers.find_url("http://b:7070/"), Some(&b));

        servers.remove(&b.id);
        assert_eq!(servers.last, None);
        assert_eq!(initial(&servers), Some(a.id.clone()));
    }

    #[test]
    fn reopens_the_last_cloud_collector() {
        let mut servers = Servers::default();
        let saved = servers.add("a".into(), "http://a:7070".into(), None);
        let account = CloudAccount {
            gateway: "https://gateway.example.com".into(),
            ca_cert: None,
        };
        servers.cloud = Some(account.clone());
        let collector = CloudCollector {
            id: "col_1".into(),
            name: "build-box".into(),
            host: "build-box.internal".into(),
        };
        let open = account.open(collector.clone());
        assert_eq!(open.connection.collector.as_deref(), Some("col_1"));
        assert_eq!(
            (open.name(), open.address(), open.key()),
            ("build-box".into(), "build-box.internal", "col_1")
        );

        servers.remember(&open);
        assert_eq!(servers.initial().unwrap().cloud, Some(collector));
        // Opening a saved server afterwards wins at the next launch.
        servers.remember(&saved.open());
        assert_eq!(servers.initial().unwrap().key(), saved.id);
        // Signed out, a remembered collector can't be opened.
        servers.remember(&open);
        servers.cloud = None;
        assert_eq!(servers.initial().unwrap().key(), saved.id);
    }

    #[test]
    fn round_trips_through_json() {
        let mut servers = Servers::default();
        servers.add(
            "prod".into(),
            "https://logs.example.com".into(),
            Some("/etc/ca.pem".into()),
        );
        let json = serde_json::to_string(&servers).unwrap();
        assert!(!json.contains("token"));
        assert_eq!(serde_json::from_str::<Servers>(&json).unwrap(), servers);
        assert_eq!(serde_json::from_str::<Servers>("{}").unwrap(), Servers::default());
    }
}
