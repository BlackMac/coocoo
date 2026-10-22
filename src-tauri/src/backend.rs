//! Picks the daemon the desktop UI talks to: an already-running `coocoo serve` for the same
//! config (attach), or a new daemon embedded in this process.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use coocoo_core::{Daemon, DaemonOptions};
use coocoo_proto::Paths;
use serde::Deserialize;

/// Port used when `config.toml` does not exist yet (matches the daemon default).
const DEFAULT_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7421);

/// The daemon the UI is connected to.
pub struct Backend {
    /// `ws://<loopback>:<port>/ws` (without the token query).
    pub ws_url: String,
    pub token: String,
    /// `Some` when the daemon runs inside this process and must be shut down on quit.
    pub daemon: Option<Arc<Daemon>>,
}

/// The `config.toml` keys the desktop shell needs; everything else belongs to `coocoo-core`.
#[derive(Deserialize)]
struct ConfigKeys {
    #[serde(rename = "hostId")]
    host_id: Option<String>,
    token: Option<String>,
    listen: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    ok: bool,
    host_id: String,
}

/// What occupies the configured listen address.
enum Probe {
    /// A healthy daemon for this very config.
    Ours(Backend),
    /// Something else (another config's daemon, an unrelated server).
    Busy(SocketAddr),
    Free,
}

impl Backend {
    /// Attaches to a running daemon that owns this config, otherwise starts one in-process.
    ///
    /// When the configured address is taken by something else, the embedded daemon binds an
    /// ephemeral loopback port instead so the desktop app still works; phone/peer access then
    /// needs the configured port freed.
    pub async fn connect(paths: Paths) -> anyhow::Result<Self> {
        let listen = match probe(&paths) {
            Probe::Ours(backend) => {
                tracing::info!(url = %backend.ws_url, "attached to running coocoo daemon");
                return Ok(backend);
            }
            Probe::Busy(addr) => {
                tracing::warn!(%addr, "configured listen address is in use; embedded daemon uses an ephemeral loopback port");
                Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            }
            Probe::Free => None,
        };
        let daemon = Daemon::start(DaemonOptions { paths, listen }).await.context("starting the embedded coocoo daemon")?;
        tracing::info!(addr = %daemon.local_addr, host = %daemon.host_id, "embedded coocoo daemon started");
        Ok(Self { ws_url: daemon.ws_url(), token: daemon.token.clone(), daemon: Some(daemon) })
    }
}

fn probe(paths: &Paths) -> Probe {
    let keys = std::fs::read_to_string(paths.config_dir.join("config.toml"))
        .ok()
        .and_then(|text| toml::from_str::<ConfigKeys>(&text).ok());
    let listen = keys.as_ref().and_then(|k| k.listen.as_deref()?.parse().ok()).unwrap_or(DEFAULT_LISTEN);
    let addr = loopback(listen);
    let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return Probe::Free };
    let health = fetch_health(stream, addr);
    match (keys, health) {
        (Some(ConfigKeys { host_id: Some(host_id), token: Some(token), .. }), Some(health))
            if health.ok && health.host_id == host_id =>
        {
            Probe::Ours(Backend { ws_url: format!("ws://{addr}/ws"), token, daemon: None })
        }
        _ => Probe::Busy(addr),
    }
}

/// Wildcard binds are reached over loopback.
fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port()),
        _ => addr,
    }
}

/// `GET /api/health` over plain HTTP/1.0 (no HTTP client library for one request at startup).
/// `None` when the peer is not a coocoo daemon or answers slowly.
fn fetch_health(mut stream: TcpStream, addr: SocketAddr) -> Option<Health> {
    let timeout = Some(Duration::from_millis(800));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;
    write!(stream, "GET /api/health HTTP/1.0\r\nHost: {addr}\r\nAccept: application/json\r\n\r\n").ok()?;
    let mut response = Vec::with_capacity(512);
    stream.take(64 * 1024).read_to_end(&mut response).ok()?;
    let status_line = response.split(|&b| b == b'\n').next()?;
    if !status_line.windows(5).any(|w| w == b" 200 ") {
        return None;
    }
    let body = response.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    serde_json::from_slice(&response[body..]).ok()
}
