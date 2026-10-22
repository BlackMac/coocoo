//! The Coocoo daemon, shared by the headless `coocoo` binary and the Tauri desktop app.

use std::net::SocketAddr;
use std::sync::Arc;

use coocoo_proto::{Emitter, Paths};

pub struct DaemonOptions {
    pub paths: Paths,
    /// Overrides `listen` from `config.toml`.
    pub listen: Option<SocketAddr>,
}

/// A running daemon.
pub struct Daemon {
    pub local_addr: SocketAddr,
    pub host_id: String,
    pub token: String,
    pub emitter: Emitter,
}

impl Daemon {
    /// Loads (or creates) the config, binds the listener, starts every subsystem and the server.
    pub async fn start(opts: DaemonOptions) -> anyhow::Result<Arc<Daemon>> {
        let _ = opts;
        todo!()
    }

    /// `ws://<local_addr>/ws` (loopback form when bound to 0.0.0.0).
    pub fn ws_url(&self) -> String {
        todo!()
    }

    /// Graceful stop: peers, agents (sessions stay resumable), browsers, server.
    pub async fn shutdown(&self) {
        todo!()
    }
}
