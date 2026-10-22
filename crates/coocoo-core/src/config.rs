//! `<config_dir>/config.toml`: host identity, auth token, listen address, peers and the
//! subsystem sections. Created on first run with `0600` permissions (it holds the token).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use coocoo_agents::AgentsConfig;
use coocoo_browser::BrowserConfig;
use coocoo_proto::{Paths, util};
use serde::{Deserialize, Serialize};

/// Default listen address: loopback only. Use `--listen 0.0.0.0:7421` (or SSH) for remote access.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7421";

/// Daemon configuration as stored in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Stable host id, generated once.
    #[serde(default = "util::new_id")]
    pub host_id: String,
    /// Display name (defaults to the machine hostname).
    #[serde(default = "hostname")]
    pub name: String,
    /// Secret required by `/ws?token=`.
    #[serde(default = "util::new_token")]
    pub token: String,
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    /// Peer daemons this daemon links to (its "fleet").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<PeerConfig>,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
}

/// How a peer is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerKind {
    /// Direct WebSocket URL (LAN, Tailscale, WireGuard…).
    Direct,
    /// `ssh -L` tunnel spawned by this daemon.
    Ssh,
}

/// One `[[peers]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerConfig {
    /// Remote host id, filled in once the peer has been reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Display-name override; the remote's own name is used when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: PeerKind,
    /// Direct: `ws://host:port` (with or without `/ws`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The remote daemon's token. SSH peers fetch it with `coocoo token` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// SSH: `user@host` (or an alias from `~/.ssh/config`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    /// SSH: port the remote daemon listens on (loopback), default 7421.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
    /// SSH: install and start `coocoo` on the remote when it is missing or not running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub install: bool,
}

impl PeerConfig {
    /// Human-readable address: the URL for direct peers, the SSH target for SSH peers.
    pub fn address(&self) -> String {
        match self.kind {
            PeerKind::Direct => self.url.clone().unwrap_or_default(),
            PeerKind::Ssh => match self.ssh_port {
                Some(port) => format!("{}:{port}", self.target.as_deref().unwrap_or_default()),
                None => self.target.clone().unwrap_or_default(),
            },
        }
    }
}

fn default_listen() -> SocketAddr {
    DEFAULT_LISTEN.parse().expect("valid default listen address")
}

impl Config {
    /// `<config_dir>/config.toml`.
    pub fn path(paths: &Paths) -> PathBuf {
        paths.config_dir.join("config.toml")
    }

    /// Loads `config.toml`, creating it (fresh host id, hostname, token) on first run.
    /// Missing required keys are filled in and written back.
    pub fn load_or_create(paths: &Paths) -> anyhow::Result<Config> {
        let path = Self::path(paths);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config: Config = toml::from_str("").context("building default config")?;
                config.save(paths)?;
                return Ok(config);
            }
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        let table: toml::Table = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let complete = ["hostId", "name", "token", "listen"].iter().all(|key| table.contains_key(*key));
        let config: Config =
            toml::Value::Table(table).try_into().with_context(|| format!("parsing {}", path.display()))?;
        if complete {
            restrict_permissions(&path);
        } else {
            config.save(paths)?;
        }
        Ok(config)
    }

    /// Atomically rewrites `config.toml` with `0600` permissions.
    pub fn save(&self, paths: &Paths) -> anyhow::Result<()> {
        let path = Self::path(paths);
        let text = toml::to_string_pretty(self).context("encoding config")?;
        write_private(&path, text.as_bytes()).with_context(|| format!("writing {}", path.display()))
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Machine hostname (fallback `coocoo`).
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for `buf.len()` bytes; gethostname NUL-terminates on success.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]).trim().to_owned();
            if !name.is_empty() {
                return name;
            }
        }
    }
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "coocoo".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reloads() {
        let root = std::env::temp_dir().join(format!("coocoo-config-{}", util::new_id()));
        let paths = Paths::under(root.clone());
        let first = Config::load_or_create(&paths).unwrap();
        assert_eq!(first.listen.to_string(), DEFAULT_LISTEN);
        assert_eq!(first.token.len(), 64);
        let mut edited = first.clone();
        edited.peers.push(PeerConfig {
            id: None,
            name: Some("box".into()),
            kind: PeerKind::Ssh,
            url: None,
            token: None,
            target: Some("me@box".into()),
            ssh_port: Some(2222),
            remote_port: None,
            install: true,
        });
        edited.save(&paths).unwrap();
        let text = std::fs::read_to_string(Config::path(&paths)).unwrap();
        assert!(text.contains("hostId") && text.contains("[[peers]]") && text.contains("sshPort = 2222"), "{text}");
        let again = Config::load_or_create(&paths).unwrap();
        assert_eq!(again.host_id, first.host_id);
        assert_eq!(again.peers.len(), 1);
        assert_eq!(again.peers[0].address(), "me@box:2222");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Config::path(&paths)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fills_missing_keys() {
        let root = std::env::temp_dir().join(format!("coocoo-config-{}", util::new_id()));
        let paths = Paths::under(root.clone());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(Config::path(&paths), "listen = \"127.0.0.1:9000\"\n").unwrap();
        let config = Config::load_or_create(&paths).unwrap();
        assert_eq!(config.listen.port(), 9000);
        let text = std::fs::read_to_string(Config::path(&paths)).unwrap();
        assert!(text.contains(&config.token));
        std::fs::remove_dir_all(root).unwrap();
    }
}
