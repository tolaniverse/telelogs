//! Servers saved in the app, kept between launches in `servers.json`. Their tokens are kept in
//! the macOS Keychain, never in the file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::client::Connection;

const FILE: &str = "servers.json";
/// Keychain service the tokens are filed under, one item per server id.
const KEYCHAIN_SERVICE: &str = "telelogs";

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
            connection: Connection {
                server: self.url.clone(),
                token: token(&self.id),
                ca_cert: self.ca_cert.clone(),
            },
        }
    }
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Servers {
    #[serde(default)]
    pub list: Vec<Server>,
    /// Id of the server opened most recently, which the next launch opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
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

    /// The server to open at launch: the last one used, else the first saved.
    pub fn initial(&self) -> Option<&Server> {
        self.last
            .as_deref()
            .and_then(|id| self.get(id))
            .or_else(|| self.list.first())
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

/// What the window is connected to: a saved server, or one given on the command line.
#[derive(Clone)]
pub struct Open {
    pub saved: Option<Server>,
    pub connection: Connection,
}

impl Open {
    pub fn unsaved(connection: Connection) -> Self {
        Open {
            saved: None,
            connection,
        }
    }

    pub fn name(&self) -> String {
        match &self.saved {
            Some(server) => server.name.clone(),
            None if is_local(&self.connection.server) => "local server".into(),
            None => "unsaved server".into(),
        }
    }

    /// What this server's source selections are saved under.
    pub fn key(&self) -> &str {
        match &self.saved {
            Some(server) => &server.id,
            None => &self.connection.server,
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
                token: None,
                ca_cert: None,
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
        assert_eq!(servers.initial(), Some(&a));
        servers.last = Some(b.id.clone());
        assert_eq!(servers.initial(), Some(&b));
        assert_eq!(servers.find_url("http://b:7070/"), Some(&b));

        servers.remove(&b.id);
        assert_eq!(servers.last, None);
        assert_eq!(servers.initial(), Some(&a));
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
