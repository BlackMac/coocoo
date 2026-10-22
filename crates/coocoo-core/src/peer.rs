//! Links from this (home) daemon to peer daemons.
//!
//! Each configured peer gets one long-lived task that connects (directly or through an SSH
//! tunnel), learns the remote host id with `system.hello`, subscribes to the remote's
//! local-origin events and relays them into the local bus. Requests addressed to the peer are
//! forwarded with remapped ids; screencast/console streams are routed only to the local
//! connections that asked for them.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use coocoo_proto::{ApiError, ApiResult, ConnSink, HostInfo, HostKind, Request, events, util};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{self, Message, Utf8Bytes};

use crate::app::{App, Reply};
use crate::config::{PeerConfig, PeerKind};
use crate::ssh::{self, Tunnel};

/// Default daemon port, used when a peer URL or SSH peer omits it.
pub(crate) const DEFAULT_PORT: u16 = 7421;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PING_EVERY: Duration = Duration::from_secs(20);
const DEAD_AFTER: Duration = Duration::from_secs(60);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Outbound queue towards the peer (requests only; small and rare).
const OUT_QUEUE: usize = 256;

type Pending = HashMap<u64, oneshot::Sender<ApiResult<Box<RawValue>>>>;

#[derive(Clone, PartialEq)]
enum Outcome {
    Pending,
    Online,
    Failed(String),
}

struct LinkState {
    /// Remote host id, or a provisional `pending-…` id until the first `system.hello`.
    id: String,
    cfg: PeerConfig,
    /// The remote's own `HostInfo` from its `system.hello`.
    remote: Option<HostInfo>,
    online: bool,
    error: Option<String>,
    out: Option<mpsc::Sender<Message>>,
}

/// One forwarded `browser.watch` stream and the local connections receiving it.
struct BrowserWatch {
    profile: String,
    target_id: String,
    /// Last `browser.watch` params, replayed after a reconnect.
    params: Value,
    conns: Vec<ConnSink>,
}

/// One forwarded `terminal.attach` and the local connections receiving its output.
struct TerminalWatch {
    id: String,
    conns: Vec<ConnSink>,
}

#[derive(Default)]
struct Watches {
    /// Local connections with a forwarded `stats.watch {enabled:true}`.
    stats: HashSet<u64>,
    browser: Vec<BrowserWatch>,
    terminals: Vec<TerminalWatch>,
}

pub(crate) struct PeerLink {
    app: Weak<App>,
    local_id: String,
    state: Mutex<LinkState>,
    pending: Mutex<Pending>,
    next_id: AtomicU64,
    watches: Mutex<Watches>,
    outcome: watch::Sender<Outcome>,
    task: Mutex<Option<JoinHandle<()>>>,
    /// Part of the fleet (persisted, announces `host.updated`); `false` while `hosts.add` probes.
    registered: AtomicBool,
}

#[derive(Deserialize)]
struct Frame<'a> {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default, borrow)]
    event: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    host: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    result: Option<&'a RawValue>,
    #[serde(default)]
    error: Option<ApiError>,
    #[serde(default, borrow)]
    data: Option<&'a RawValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetRef<'a> {
    #[serde(borrow)]
    profile: Cow<'a, str>,
    #[serde(borrow)]
    target_id: Cow<'a, str>,
}

#[derive(Deserialize)]
struct TerminalRef<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
}

impl PeerLink {
    pub fn new(app: &Arc<App>, cfg: PeerConfig, registered: bool) -> Arc<PeerLink> {
        let id = cfg.id.clone().unwrap_or_else(|| format!("pending-{}", util::new_id()));
        Arc::new(PeerLink {
            app: Arc::downgrade(app),
            local_id: app.host_id.clone(),
            state: Mutex::new(LinkState { id, cfg, remote: None, online: false, error: None, out: None }),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            watches: Mutex::new(Watches::default()),
            outcome: watch::Sender::new(Outcome::Pending),
            task: Mutex::new(None),
            registered: AtomicBool::new(registered),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let link = self.clone();
        *self.task.lock() = Some(tokio::spawn(link.run()));
    }

    /// Stops the link task (drops the SSH tunnel) and fails in-flight requests.
    pub fn stop(&self) {
        if let Some(task) = self.task.lock().take() {
            task.abort();
        }
        {
            let mut st = self.state.lock();
            st.online = false;
            st.out = None;
        }
        self.fail_pending("link removed");
    }

    pub fn id(&self) -> String {
        self.state.lock().id.clone()
    }

    pub fn config(&self) -> PeerConfig {
        self.state.lock().cfg.clone()
    }

    pub fn set_name(&self, name: String) {
        self.state.lock().cfg.name = Some(name);
    }

    pub fn set_registered(&self) {
        self.registered.store(true, Ordering::Relaxed);
    }

    fn registered(&self) -> bool {
        self.registered.load(Ordering::Relaxed)
    }

    /// Resolves once the first connection attempt succeeded (`Ok`) or failed (`Err`).
    pub async fn wait_first_outcome(&self, limit: Duration) -> Result<(), String> {
        let mut rx = self.outcome.subscribe();
        match tokio::time::timeout(limit, rx.wait_for(|o| *o != Outcome::Pending)).await {
            Ok(Ok(outcome)) => match &*outcome {
                Outcome::Online => Ok(()),
                Outcome::Failed(err) => Err(err.clone()),
                Outcome::Pending => Err("link stopped".to_owned()),
            },
            Ok(Err(_)) => Err("link stopped".to_owned()),
            Err(_) => Err(format!("no connection within {}s", limit.as_secs())),
        }
    }

    /// This daemon's view of the peer.
    pub fn info(&self) -> HostInfo {
        let st = self.state.lock();
        let remote = st.remote.as_ref();
        HostInfo {
            id: st.id.clone(),
            name: st.cfg.name.clone().or_else(|| remote.map(|r| r.name.clone())).unwrap_or_else(|| st.cfg.address()),
            kind: match st.cfg.kind {
                PeerKind::Direct => HostKind::Direct,
                PeerKind::Ssh => HostKind::Ssh,
            },
            online: st.online,
            os: remote.and_then(|r| r.os.clone()),
            arch: remote.and_then(|r| r.arch.clone()),
            version: remote.and_then(|r| r.version.clone()),
            address: Some(st.cfg.address()),
            error: st.error.clone(),
            omp_version: remote.and_then(|r| r.omp_version.clone()),
        }
    }

    fn display_name(&self) -> String {
        let st = self.state.lock();
        st.cfg.name.clone().or_else(|| st.remote.as_ref().map(|r| r.name.clone())).unwrap_or_else(|| st.cfg.address())
    }

    fn announce(&self) {
        if self.registered()
            && let Some(app) = self.app.upgrade()
        {
            app.emit_fleet(events::HOST_UPDATED, &self.info());
        }
    }

    // ─── Requests ─────────────────────────────────────────────────────────────────────────

    /// Forward a client request, keeping track of stream subscriptions so that several local
    /// connections can share one remote stream and remote streams stop with the last watcher.
    pub async fn forward(self: &Arc<Self>, sink: &ConnSink, method: &str, params: Value) -> ApiResult<Reply> {
        // Connection id 0 is the detached orchestrator caller: it never disconnects, so it is
        // not tracked.
        if sink.id != 0 {
            match method {
                "browser.watch" => return self.forward_watch(sink, params).await,
                "browser.unwatch" => return self.forward_unwatch(sink.id, params).await,
                "stats.watch" => return self.forward_stats_watch(sink.id, params).await,
                "terminal.attach" => return self.forward_attach(sink, params).await,
                "terminal.detach" => return self.forward_detach(sink.id, params).await,
                _ => {}
            }
        }
        self.request(method, params).await.map(Reply::Raw)
    }

    async fn forward_watch(&self, sink: &ConnSink, params: Value) -> ApiResult<Reply> {
        let target = TargetRef::deserialize(&params).map_err(ApiError::from)?;
        let (profile, target_id) = (target.profile.into_owned(), target.target_id.into_owned());
        let result = self.request("browser.watch", params.clone()).await?;
        let mut watches = self.watches.lock();
        match watches.browser.iter_mut().find(|w| w.profile == profile && w.target_id == target_id) {
            Some(w) => {
                w.params = params;
                w.conns.retain(|c| !c.is_closed());
                if !w.conns.iter().any(|c| c.id == sink.id) {
                    w.conns.push(sink.clone());
                }
            }
            None => watches.browser.push(BrowserWatch { profile, target_id, params, conns: vec![sink.clone()] }),
        }
        Ok(Reply::Raw(result))
    }

    async fn forward_unwatch(&self, conn_id: u64, params: Value) -> ApiResult<Reply> {
        let target = TargetRef::deserialize(&params).map_err(ApiError::from)?;
        let last = {
            let mut watches = self.watches.lock();
            match watches.browser.iter().position(|w| w.profile == target.profile && w.target_id == target.target_id) {
                Some(idx) => {
                    let w = &mut watches.browser[idx];
                    w.conns.retain(|c| c.id != conn_id && !c.is_closed());
                    let empty = w.conns.is_empty();
                    if empty {
                        watches.browser.remove(idx);
                    }
                    empty
                }
                None => true,
            }
        };
        if last { self.request("browser.unwatch", params).await.map(Reply::Raw) } else { Ok(Reply::Value(json!({}))) }
    }

    async fn forward_stats_watch(&self, conn_id: u64, params: Value) -> ApiResult<Reply> {
        #[derive(Deserialize)]
        struct P {
            enabled: bool,
        }
        let P { enabled } = P::deserialize(&params).map_err(ApiError::from)?;
        if enabled {
            let first = {
                let mut watches = self.watches.lock();
                watches.stats.insert(conn_id) && watches.stats.len() == 1
            };
            if !first {
                return Ok(Reply::Value(json!({})));
            }
            match self.request("stats.watch", params).await {
                Ok(result) => Ok(Reply::Raw(result)),
                Err(err) => {
                    self.watches.lock().stats.remove(&conn_id);
                    Err(err)
                }
            }
        } else {
            let last = {
                let mut watches = self.watches.lock();
                watches.stats.remove(&conn_id) && watches.stats.is_empty()
            };
            if last { self.request("stats.watch", params).await.map(Reply::Raw) } else { Ok(Reply::Value(json!({}))) }
        }
    }

    async fn forward_attach(&self, sink: &ConnSink, params: Value) -> ApiResult<Reply> {
        let id = TerminalRef::deserialize(&params).map_err(ApiError::from)?.id.into_owned();
        // Always forwarded: the remote returns this viewer's scrollback snapshot.
        let result = self.request("terminal.attach", params).await?;
        let mut watches = self.watches.lock();
        match watches.terminals.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.conns.retain(|c| !c.is_closed());
                if !t.conns.iter().any(|c| c.id == sink.id) {
                    t.conns.push(sink.clone());
                }
            }
            None => watches.terminals.push(TerminalWatch { id, conns: vec![sink.clone()] }),
        }
        Ok(Reply::Raw(result))
    }

    async fn forward_detach(&self, conn_id: u64, params: Value) -> ApiResult<Reply> {
        let id = TerminalRef::deserialize(&params).map_err(ApiError::from)?.id;
        let last = {
            let mut watches = self.watches.lock();
            match watches.terminals.iter().position(|t| t.id == id) {
                Some(idx) => {
                    let t = &mut watches.terminals[idx];
                    t.conns.retain(|c| c.id != conn_id && !c.is_closed());
                    let empty = t.conns.is_empty();
                    if empty {
                        watches.terminals.remove(idx);
                    }
                    empty
                }
                None => true,
            }
        };
        if last { self.request("terminal.detach", params).await.map(Reply::Raw) } else { Ok(Reply::Value(json!({}))) }
    }

    /// Drop a closed local connection's subscriptions; stop remote streams nobody watches.
    pub fn conn_closed(self: &Arc<Self>, conn_id: u64) {
        let (unwatch, detach, stats_off) = {
            let mut watches = self.watches.lock();
            let mut unwatch = Vec::new();
            watches.browser.retain_mut(|w| {
                let before = w.conns.len();
                w.conns.retain(|c| c.id != conn_id && !c.is_closed());
                if w.conns.is_empty() && before > 0 {
                    unwatch.push(json!({ "profile": w.profile, "targetId": w.target_id }));
                    false
                } else {
                    true
                }
            });
            let mut detach = Vec::new();
            watches.terminals.retain_mut(|t| {
                t.conns.retain(|c| c.id != conn_id && !c.is_closed());
                if t.conns.is_empty() {
                    detach.push(json!({ "id": t.id }));
                }
                !t.conns.is_empty()
            });
            let stats_off = watches.stats.remove(&conn_id) && watches.stats.is_empty();
            (unwatch, detach, stats_off)
        };
        if unwatch.is_empty() && detach.is_empty() && !stats_off {
            return;
        }
        let link = self.clone();
        tokio::spawn(async move {
            for params in unwatch {
                let _ = link.request("browser.unwatch", params).await;
            }
            for params in detach {
                let _ = link.request("terminal.detach", params).await;
            }
            if stats_off {
                let _ = link.request("stats.watch", json!({ "enabled": false })).await;
            }
        });
    }

    /// Re-establish forwarded streams on a fresh connection. Terminals are not re-attached here:
    /// output sent while the link was down is gone, so clients re-attach (and replay the
    /// scrollback) when the host comes back online.
    fn rewatch(self: &Arc<Self>) {
        let (browser, stats) = {
            let mut watches = self.watches.lock();
            watches.browser.retain_mut(|w| {
                w.conns.retain(|c| !c.is_closed());
                !w.conns.is_empty()
            });
            let params: Vec<Value> = watches.browser.iter().map(|w| w.params.clone()).collect();
            (params, !watches.stats.is_empty())
        };
        if browser.is_empty() && !stats {
            return;
        }
        let link = self.clone();
        tokio::spawn(async move {
            for params in browser {
                if let Err(err) = link.request("browser.watch", params).await {
                    tracing::debug!("re-watching remote tab failed: {err}");
                }
            }
            if stats {
                let _ = link.request("stats.watch", json!({ "enabled": true })).await;
            }
        });
    }

    /// Send one request to the peer and await its raw result.
    pub async fn request(&self, method: &str, params: Value) -> ApiResult<Box<RawValue>> {
        let out = self.state.lock().out.clone();
        let out = out.ok_or_else(|| self.offline_error())?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);
        let _guard = PendingGuard { pending: &self.pending, id };
        let text = serde_json::to_string(&Request { id, method: method.to_owned(), params, host: None })
            .map_err(ApiError::internal)?;
        if out.send(Message::Text(text.into())).await.is_err() {
            return Err(self.offline_error());
        }
        let limit = request_timeout(method);
        match tokio::time::timeout(limit, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ApiError::unavailable(format!("lost connection to {}", self.display_name()))),
            Err(_) => Err(ApiError::unavailable(format!(
                "{} did not answer {method} within {}s",
                self.display_name(),
                limit.as_secs()
            ))),
        }
    }

    fn offline_error(&self) -> ApiError {
        let st = self.state.lock();
        let name = st.cfg.name.clone().or_else(|| st.remote.as_ref().map(|r| r.name.clone())).unwrap_or_else(|| st.cfg.address());
        match &st.error {
            Some(err) => ApiError::unavailable(format!("host {name} is offline: {err}")),
            None => ApiError::unavailable(format!("host {name} is offline")),
        }
    }

    fn fail_pending(&self, why: &str) {
        let drained: Vec<_> = self.pending.lock().drain().collect();
        for (_, tx) in drained {
            let _ = tx.send(Err(ApiError::unavailable(why.to_owned())));
        }
    }

    // ─── Connection lifecycle ─────────────────────────────────────────────────────────────

    async fn run(self: Arc<Self>) {
        let mut backoff = BACKOFF_MIN;
        let mut tunnel: Option<Tunnel> = None;
        loop {
            let error = self.attempt(&mut tunnel).await.err();
            if let Some(err) = &error {
                self.outcome.send_if_modified(|o| {
                    if *o == Outcome::Pending {
                        *o = Outcome::Failed(err.clone());
                        true
                    } else {
                        false
                    }
                });
                tracing::debug!(peer = %self.id(), "peer link: {err}");
            }
            if self.set_offline(error) {
                backoff = BACKOFF_MIN;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    /// Returns whether the link had been online.
    fn set_offline(&self, error: Option<String>) -> bool {
        let (was_online, changed) = {
            let mut st = self.state.lock();
            let was_online = st.online;
            let changed = was_online || st.error != error;
            st.online = false;
            st.out = None;
            st.error = error;
            (was_online, changed)
        };
        self.fail_pending("connection to peer lost");
        if changed {
            self.announce();
        }
        was_online
    }

    async fn attempt(self: &Arc<Self>, tunnel: &mut Option<Tunnel>) -> Result<(), String> {
        let cfg = self.config();
        let ws = match cfg.kind {
            PeerKind::Direct => {
                let (base, token) = parse_direct(cfg.url.as_deref().unwrap_or_default(), cfg.token.as_deref())?;
                connect(&ws_url(&base, &token)).await?
            }
            PeerKind::Ssh => self.connect_ssh(&cfg, tunnel).await?,
        };
        self.session(ws).await
    }

    async fn connect_ssh(&self, cfg: &PeerConfig, tunnel: &mut Option<Tunnel>) -> Result<WsStream, String> {
        let target = cfg.target.as_deref().ok_or("ssh peer without target")?;
        let remote_port = cfg.remote_port.unwrap_or(DEFAULT_PORT);
        let mut installed = false;
        let token = match &cfg.token {
            Some(token) => token.clone(),
            None => {
                let token = match ssh::fetch_token(target, cfg.ssh_port).await {
                    Ok(token) => token,
                    Err(ssh::RemoteError::Command(err)) if cfg.install => {
                        tracing::info!("installing coocoo on {target} ({err})");
                        ssh::install(target, cfg.ssh_port, remote_port).await?;
                        installed = true;
                        ssh::fetch_token(target, cfg.ssh_port).await.map_err(|e| e.to_string())?
                    }
                    Err(ssh::RemoteError::Command(err)) => {
                        return Err(format!("{err} — install coocoo on {target}, or add the host with install enabled"));
                    }
                    Err(err) => return Err(err.to_string()),
                };
                self.update_cfg(|c| c.token = Some(token.clone()));
                token
            }
        };
        if !tunnel.as_mut().is_some_and(Tunnel::alive) {
            *tunnel = None;
            *tunnel = Some(Tunnel::spawn(target, cfg.ssh_port, remote_port).await?);
        }
        let port = tunnel.as_ref().map(|t| t.port).unwrap_or_default();
        let url = ws_url(&format!("ws://127.0.0.1:{port}"), &token);
        let first = match connect(&url).await {
            Ok(ws) => return Ok(ws),
            Err(err) => err,
        };
        if first.starts_with("unauthorized") {
            // The remote config was regenerated: fetch the token again next time.
            self.update_cfg(|c| c.token = None);
            return Err(first);
        }
        if !cfg.install {
            return Err(format!(
                "ssh tunnel is up but no daemon answers on {target} port {remote_port} ({first}); start `coocoo serve` there"
            ));
        }
        if !installed {
            ssh::start_remote(target, cfg.ssh_port, remote_port).await?;
        }
        // Give the freshly started daemon a moment to bind.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match connect(&url).await {
                Ok(ws) => return Ok(ws),
                Err(err) if Instant::now() >= deadline => {
                    return Err(format!("coocoo was started on {target} but does not answer on port {remote_port}: {err}"));
                }
                Err(_) => {}
            }
        }
    }

    /// Update the peer config, persisting it when the link is part of the fleet.
    fn update_cfg(&self, f: impl FnOnce(&mut PeerConfig)) {
        f(&mut self.state.lock().cfg);
        if self.registered()
            && let Some(app) = self.app.upgrade()
        {
            app.save_config();
        }
    }

    async fn session(self: &Arc<Self>, ws: WsStream) -> Result<(), String> {
        let app = self.app.upgrade().ok_or("daemon stopped")?;
        let (mut tx, mut rx) = ws.split();

        let hello = handshake_call(&mut tx, &mut rx, u64::MAX, "system.hello", Value::Null).await?;
        let remote: HostInfo = hello
            .get("host")
            .cloned()
            .and_then(|h| serde_json::from_value(h).ok())
            .ok_or("peer sent an invalid system.hello")?;
        if remote.id == self.local_id {
            return Err("that address is this daemon itself".to_owned());
        }
        if app.peers.read().iter().any(|l| !std::ptr::eq(l.as_ref(), self.as_ref()) && l.id() == remote.id) {
            return Err(format!("host {} ({}) is already in the fleet", remote.name, remote.id));
        }
        handshake_call(&mut tx, &mut rx, u64::MAX - 1, "system.subscribe", json!({ "scope": "local" })).await?;

        let (out_tx, mut out_rx) = mpsc::channel::<Message>(OUT_QUEUE);
        let (old_id, id_changed) = {
            let mut st = self.state.lock();
            let old = std::mem::replace(&mut st.id, remote.id.clone());
            let changed = old != remote.id || st.cfg.id.as_deref() != Some(remote.id.as_str());
            st.cfg.id = Some(remote.id.clone());
            st.remote = Some(remote);
            st.online = true;
            st.error = None;
            st.out = Some(out_tx);
            (old, changed)
        };
        if id_changed && self.registered() {
            app.save_config();
            if old_id != self.id() {
                app.emit_fleet(events::HOST_REMOVED, &json!({ "id": old_id }));
            }
        }
        self.outcome.send_replace(Outcome::Online);
        self.announce();
        self.rewatch();
        drop(app);

        let mut ping = tokio::time::interval(PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping.tick().await;
        let mut last_seen = Instant::now();
        loop {
            tokio::select! {
                frame = rx.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = Instant::now();
                        self.on_frame(&text);
                    }
                    Some(Ok(Message::Close(_))) | None => return Err("connection closed by peer".to_owned()),
                    Some(Ok(_)) => last_seen = Instant::now(),
                    Some(Err(err)) => return Err(format!("connection error: {err}")),
                },
                msg = out_rx.recv() => match msg {
                    Some(msg) => tx.send(msg).await.map_err(|e| format!("send failed: {e}"))?,
                    None => return Err("link stopped".to_owned()),
                },
                _ = ping.tick() => {
                    if last_seen.elapsed() > DEAD_AFTER {
                        return Err("peer stopped responding".to_owned());
                    }
                    tx.send(Message::Ping(Default::default())).await.map_err(|e| format!("send failed: {e}"))?;
                }
            }
        }
    }

    fn on_frame(&self, text: &Utf8Bytes) {
        let frame: Frame<'_> = match serde_json::from_str(text.as_str()) {
            Ok(frame) => frame,
            Err(err) => {
                tracing::warn!("invalid frame from peer: {err}");
                return;
            }
        };
        if let Some(event) = frame.event {
            let host = frame.host.unwrap_or_default();
            if host == self.local_id {
                return;
            }
            match &*event {
                events::BROWSER_FRAME | events::BROWSER_CONSOLE => {
                    if let Some(target) = frame.data.and_then(|d| serde_json::from_str::<TargetRef<'_>>(d.get()).ok()) {
                        self.deliver(&target, text);
                    }
                }
                events::TERMINAL_OUTPUT | events::TERMINAL_EXIT => {
                    if let Some(term) = frame.data.and_then(|d| serde_json::from_str::<TerminalRef<'_>>(d.get()).ok()) {
                        self.deliver_terminal(&term.id, event == events::TERMINAL_EXIT, text);
                    }
                }
                events::HOST_UPDATED => {
                    let info = frame.data.and_then(|d| serde_json::from_str::<HostInfo>(d.get()).ok());
                    if let Some(info) = info.filter(|i| i.id == host) {
                        self.state.lock().remote = Some(info);
                        self.announce();
                    }
                }
                // The peer's own fleet is not ours.
                events::HOST_REMOVED => {}
                _ => {
                    if let Some(app) = self.app.upgrade() {
                        app.emitter.relay(event.into_owned(), host.into_owned(), Arc::from(text.as_str()));
                    }
                }
            }
        } else if let Some(id) = frame.id {
            let result = match frame.error {
                Some(err) => Err(err),
                None => Ok(frame.result.map(ToOwned::to_owned).unwrap_or_else(null_raw)),
            };
            if let Some(tx) = self.pending.lock().remove(&id) {
                let _ = tx.send(result);
            }
        }
    }

    /// Route a screencast frame / console line to the local connections watching that tab.
    fn deliver(&self, target: &TargetRef<'_>, text: &Utf8Bytes) {
        let mut watches = self.watches.lock();
        let Some(w) = watches.browser.iter_mut().find(|w| w.profile == target.profile && w.target_id == target.target_id)
        else {
            return;
        };
        let shared: Arc<str> = Arc::from(text.as_str());
        w.conns.retain(|c| !c.is_closed());
        for conn in &w.conns {
            // Lossy: a slow viewer gets fewer frames instead of unbounded memory.
            conn.try_send(shared.clone());
        }
    }

    /// Route terminal output (or its exit) to the local connections attached to it.
    fn deliver_terminal(&self, id: &str, exited: bool, text: &Utf8Bytes) {
        let mut watches = self.watches.lock();
        let Some(idx) = watches.terminals.iter().position(|t| t.id == id) else { return };
        let shared: Arc<str> = Arc::from(text.as_str());
        let t = &mut watches.terminals[idx];
        t.conns.retain(|c| !c.is_closed());
        for conn in &t.conns {
            // A full client queue drops output; the client re-attaches to resync.
            conn.try_send(shared.clone());
        }
        if exited || t.conns.is_empty() {
            watches.terminals.remove(idx);
        }
    }
}

struct PendingGuard<'a> {
    pending: &'a Mutex<Pending>,
    id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.pending.lock().remove(&self.id);
    }
}

fn null_raw() -> Box<RawValue> {
    RawValue::from_string("null".to_owned()).expect("valid JSON")
}

/// Methods that may legitimately take long on the remote side.
fn request_timeout(method: &str) -> Duration {
    const LONG: &[&str] = &[
        "agents.create",
        "agents.delete",
        "agents.archive",
        "agents.hibernate",
        "agents.models",
        "agents.prompt",
        "boards.cardStart",
        "boards.cardDone",
        "boards.sourceAdd",
        "boards.sourceRefresh",
        "boards.sourceUpdate",
        "git.commit",
        "git.merge",
        "git.diff",
        "git.worktreeCreate",
        "git.worktreeRemove",
        "projects.add",
        "projects.refresh",
        "browser.start",
        "browser.open",
        "browser.screenshot",
        "browser.inspect",
        "hosts.add",
    ];
    if LONG.contains(&method) { Duration::from_secs(120) } else { Duration::from_secs(30) }
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;
type WsSource = futures_util::stream::SplitStream<WsStream>;

async fn connect(url: &str) -> Result<WsStream, String> {
    match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async_with_config(url, None, true)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(tungstenite::Error::Http(resp))) if resp.status() == 401 => {
            Err("unauthorized: the peer rejected the token".to_owned())
        }
        Ok(Err(tungstenite::Error::Http(resp))) => Err(format!("peer answered HTTP {}", resp.status())),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err(format!("connection timed out after {}s", CONNECT_TIMEOUT.as_secs())),
    }
}

/// One request/response during the handshake, before the session loop runs.
async fn handshake_call(tx: &mut WsSink, rx: &mut WsSource, id: u64, method: &str, params: Value) -> Result<Value, String> {
    let text = serde_json::to_string(&Request { id, method: method.to_owned(), params, host: None })
        .map_err(|e| e.to_string())?;
    let exchange = async {
        tx.send(Message::Text(text.into())).await.map_err(|e| e.to_string())?;
        while let Some(msg) = rx.next().await {
            let Message::Text(text) = msg.map_err(|e| e.to_string())? else { continue };
            let Ok(frame) = serde_json::from_str::<Frame<'_>>(text.as_str()) else { continue };
            if frame.event.is_some() || frame.id != Some(id) {
                continue;
            }
            if let Some(err) = frame.error {
                return Err(format!("{method} failed: {err}"));
            }
            return match frame.result {
                Some(raw) => serde_json::from_str(raw.get()).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            };
        }
        Err("connection closed during handshake".to_owned())
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, exchange).await.map_err(|_| format!("{method} timed out"))?
}

/// Normalize a direct peer address into `ws://authority` plus its token.
///
/// Accepts `ws://host:port[/ws]`, `http://host:port[/…]`, bare `host[:port]`, and the pairing
/// link `http://host:port/#token=…` (the token is taken from the fragment when not given).
pub(crate) fn parse_direct(url: &str, token: Option<&str>) -> Result<(String, String), String> {
    let url = url.trim();
    let rest = if let Some(rest) = url.strip_prefix("ws://").or_else(|| url.strip_prefix("http://")) {
        rest
    } else if url.starts_with("wss://") || url.starts_with("https://") {
        return Err("TLS peers (wss://) are not supported; use ws:// over a LAN/VPN, or add the host over SSH".to_owned());
    } else if url.contains("://") {
        return Err(format!("unsupported peer URL `{url}`"));
    } else {
        url
    };
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("invalid peer URL `{url}`"));
    }
    let has_port = match authority.rfind(']') {
        Some(close) => authority[close..].contains(':'),
        None => authority.contains(':'),
    };
    let authority = if has_port { authority.to_owned() } else { format!("{authority}:{DEFAULT_PORT}") };
    let from_fragment = fragment.split('&').find_map(|kv| kv.strip_prefix("token="));
    let token = token.map(str::trim).filter(|t| !t.is_empty()).or(from_fragment).ok_or("missing token")?;
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')) {
        return Err("invalid token".to_owned());
    }
    Ok((format!("ws://{authority}"), token.to_owned()))
}

/// Link URL: only local-origin events from the start (no relay loops even before
/// `system.subscribe` is processed).
fn ws_url(base: &str, token: &str) -> String {
    format!("{base}/ws?token={token}&scope=local")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_urls() {
        let t = Some("abc");
        assert_eq!(parse_direct("ws://h:1/ws", t).unwrap(), ("ws://h:1".into(), "abc".into()));
        assert_eq!(parse_direct("http://10.0.0.2:7421/#token=f00d", None).unwrap(), ("ws://10.0.0.2:7421".into(), "f00d".into()));
        assert_eq!(parse_direct("box", t).unwrap().0, "ws://box:7421");
        assert_eq!(parse_direct("[::1]", t).unwrap().0, "ws://[::1]:7421");
        assert_eq!(parse_direct("ws://[::1]:9/", t).unwrap().0, "ws://[::1]:9");
        assert!(parse_direct("wss://h", t).is_err());
        assert!(parse_direct("ws://h", None).is_err());
        assert!(parse_direct("ws://h", Some("a b")).is_err());
    }
}
