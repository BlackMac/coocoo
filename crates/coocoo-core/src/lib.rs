//! The Coocoo daemon, shared by the headless `coocoo` binary and the Tauri desktop app.
//!
//! One daemon runs per machine. Clients (desktop UI, web/phone UI) connect to a single "home"
//! daemon over `/ws`; the home daemon links to peer daemons (direct WebSocket or SSH tunnel),
//! forwards requests carrying a foreign `host` and relays peer events, so one UI sees the
//! whole fleet.

mod app;
pub mod config;
mod conn;
mod insights;
mod peer;
mod server;
mod ssh;
mod ui;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::serve::ListenerExt;
use coocoo_agents::Agents;
use coocoo_browser::Browser;
use coocoo_insights::Insights;
use coocoo_proto::{Emitter, Paths};
use coocoo_workspace::{PidSource, Stats, Terminals, Workspace};
use parking_lot::Mutex;
use tokio::task::JoinHandle;

use crate::app::{App, Orch, Subsystems};
pub use crate::config::Config;

/// Bus capacity: how far a slow connection may lag before it is told to resync.
const BUS_CAPACITY: usize = 2048;

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
    app: Arc<App>,
    server: Mutex<Option<JoinHandle<()>>>,
}

impl Daemon {
    /// Loads (or creates) the config, binds the listener, starts every subsystem and the server.
    ///
    /// Runs on the caller's tokio runtime (the CLI's, or Tauri's in the desktop app).
    pub async fn start(opts: DaemonOptions) -> anyhow::Result<Arc<Daemon>> {
        let paths = opts.paths;
        paths.ensure().context("creating coocoo directories")?;
        let config = Config::load_or_create(&paths)?;
        let listen = opts.listen.unwrap_or(config.listen);
        let listener = match tokio::net::TcpListener::bind(listen).await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => anyhow::bail!(
                "{listen} is already in use — another coocoo daemon is probably running (check with `coocoo status`); \
                 pick another address with --listen"
            ),
            Err(err) => return Err(err).with_context(|| format!("binding {listen}")),
        };
        let local_addr = listener.local_addr().context("reading bound address")?;
        let listen_file = paths.listen_file();
        if let Err(err) = std::fs::write(&listen_file, local_addr.to_string()) {
            tracing::warn!(file = %listen_file.display(), "cannot record the listen address: {err}");
        }

        let emitter = Emitter::new(config.host_id.clone(), BUS_CAPACITY);
        let workspace = Workspace::new(emitter.clone(), &paths);
        let agents = Agents::new(emitter.clone(), &paths, config.agents.clone());
        let browser = Browser::new(emitter.clone(), &paths, config.browser.clone());
        let pids: PidSource = {
            let (agents, browser) = (agents.clone(), browser.clone());
            Arc::new(move || {
                let mut pids = agents.pids();
                pids.extend(browser.pids());
                pids
            })
        };
        let stats = Stats::new(emitter.clone(), pids);
        let terminals = Terminals::new(emitter.clone());
        let insights = Insights::new(&paths.data_dir, agents.omp_path().to_owned(), &config.insights);
        agents.set_insights(Arc::new(crate::insights::Sink(insights.clone())));
        {
            // Agents that existed before the ledger (or changed while the daemon was down).
            let (insights, agents) = (insights.clone(), agents.summaries());
            tokio::task::spawn_blocking(move || insights.backfill(&agents));
        }

        let host_id = config.host_id.clone();
        let token = config.token.clone();
        let app = App::new(paths, config, emitter.clone(), Subsystems { workspace, stats, terminals, agents, browser, insights });
        app.agents.set_orchestrator(Arc::new(Orch {
            app: Arc::downgrade(&app),
            host_id: host_id.clone(),
        }));
        app.start_peers();
        {
            // Warm the `omp --version` cache so the first `system.hello` does not wait for it.
            let app = app.clone();
            tokio::spawn(async move {
                app.omp_version().await;
            });
        }

        let router = server::router(app.clone());
        let mut stop = app.shutdown.subscribe();
        let listener = listener.tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });
        let server = tokio::spawn(async move {
            let service = router.into_make_service_with_connect_info::<SocketAddr>();
            let result = axum::serve(listener, service)
                .with_graceful_shutdown(async move {
                    let _ = stop.wait_for(|stop| *stop).await;
                })
                .await;
            if let Err(err) = result {
                tracing::error!("http server failed: {err}");
            }
        });
        // Agents a previous daemon paused or cut off for its restart learn that it is done.
        app.agents.resume_after_restart();
        tracing::info!(%local_addr, host = %host_id, "coocoo daemon listening");

        Ok(Arc::new(Daemon { local_addr, host_id, token, emitter, app, server: Mutex::new(Some(server)) }))
    }

    /// `ws://<local_addr>/ws` (loopback form when bound to 0.0.0.0).
    pub fn ws_url(&self) -> String {
        format!("ws://{}/ws", loopback(self.local_addr))
    }

    /// Current configuration (name, peers with learned ids/tokens, subsystem sections).
    pub fn config_snapshot(&self) -> Config {
        let peers = self.app.peers.read().iter().map(|p| p.config()).collect();
        let mut config = self.app.config.lock().clone();
        config.peers = peers;
        config
    }

    /// Before a restart (signal to `coocoo serve`): ask agents in a turn to pause and wait for
    /// them (bounded by `agents.restartGraceSecs`); they are told on the next start that the
    /// restart is done. Follow with [`Daemon::shutdown`].
    pub async fn drain(&self) {
        tracing::info!("restart: asking running agents to pause");
        self.app.agents.drain().await;
    }

    /// Graceful stop: peers, agents (sessions stay resumable), browsers, usage log, server.
    /// Idempotent; returns after everything stopped or the internal timeouts expired.
    pub async fn shutdown(&self) {
        if self.app.shutdown.send_replace(true) {
            return;
        }
        tracing::info!("shutting down");
        // Only our own record: a newer daemon on this home may have replaced it.
        let listen_file = self.app.paths.listen_file();
        if std::fs::read_to_string(&listen_file).is_ok_and(|s| s.trim() == self.local_addr.to_string()) {
            let _ = std::fs::remove_file(&listen_file);
        }
        self.app.stop_peers();
        let subsystems = async { tokio::join!(self.app.agents.shutdown(), self.app.browser.shutdown()) };
        if tokio::time::timeout(Duration::from_secs(15), subsystems).await.is_err() {
            tracing::warn!("subsystems did not stop within 15s");
        }
        // The usage log buffers up to 2 s of events; the stops above added the last ones.
        let insights = self.app.insights.clone();
        if let Err(err) = tokio::task::spawn_blocking(move || insights.flush()).await {
            tracing::warn!("flushing the usage log failed: {err}");
        }
        let server = self.server.lock().take();
        if let Some(server) = server
            && tokio::time::timeout(Duration::from_secs(5), server).await.is_err()
        {
            tracing::warn!("http server did not stop within 5s");
        }
    }
}

/// Replace an unspecified bind address (`0.0.0.0`, `::`) with the matching loopback address.
pub fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => SocketAddr::new(Ipv4Addr::LOCALHOST.into(), addr.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => SocketAddr::new(Ipv6Addr::LOCALHOST.into(), addr.port()),
        _ => addr,
    }
}
