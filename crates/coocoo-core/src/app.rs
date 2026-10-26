//! Shared daemon state and request routing.

use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use coocoo_agents::Agents;
use coocoo_insights::Insights;
use coocoo_browser::Browser;
use coocoo_proto::bus::encode_event;
use coocoo_proto::error::{ok, params};
use coocoo_proto::orchestrator::BoxFuture;
use coocoo_proto::insights::{ORIGIN, Origin, TOOL_AGENT, current_origin};
use coocoo_proto::{
    ApiError, ApiResult, ConnSink, Emitter, HostInfo, HostKind, Orchestrator, PROTOCOL_VERSION, Paths, Response,
    events,
};
use coocoo_workspace::{Stats, Terminals, Workspace};
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use serde::Serialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, mpsc, watch};

use crate::config::{Config, PeerConfig, PeerKind};
use crate::peer::PeerLink;

/// Daemon version (`CARGO_PKG_VERSION`).
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) struct App {
    pub host_id: String,
    pub token: String,
    pub paths: Paths,
    pub config: Mutex<Config>,
    pub emitter: Emitter,
    pub workspace: Arc<Workspace>,
    pub stats: Arc<Stats>,
    pub terminals: Arc<Terminals>,
    pub agents: Arc<Agents>,
    pub insights: Arc<Insights>,
    /// Web Push sender (subscriptions, VAPID).
    pub push: Arc<crate::push::Push>,
    pub browser: Arc<Browser>,
    pub peers: RwLock<Vec<Arc<PeerLink>>>,
    /// Becomes `true` once on shutdown; connection loops and the server watch it.
    pub shutdown: watch::Sender<bool>,
    /// Pre-encoded `system.resync` event for lagging connections.
    pub resync: Arc<str>,
    omp_version: OnceCell<Option<String>>,
    next_conn: AtomicU64,
    /// Sink for calls without a connection (orchestrator); everything sent to it is dropped.
    detached: ConnSink,
    /// Last logged `browser.input` per client and profile (logged at most every 10 min).
    pub input_logged: Mutex<std::collections::HashMap<String, i64>>,
}

/// Per-connection flags shared between the connection loop and request handlers.
#[derive(Default)]
pub(crate) struct ConnState {
    /// Forward only events that originated on this daemon (`system.subscribe {scope:"local"}`).
    pub scope_local: AtomicBool,
    /// The connection is a peer daemon's link: its requests are never forwarded further.
    pub peer: AtomicBool,
    /// `(client id, device)` the client named in `system.hello`, for the usage log.
    pub client: Mutex<Option<(String, String)>>,
}

impl ConnState {
    /// Who is behind a request from this connection: a person on a client, or for peer links
    /// whoever the forwarding daemon says (`Request.origin`).
    pub fn origin(&self, forwarded: Option<Origin>) -> Option<Origin> {
        if self.peer.load(Ordering::Relaxed) {
            return forwarded;
        }
        let client = self.client.lock().clone();
        Some(Origin {
            actor: coocoo_proto::insights::Actor::User,
            device: client.as_ref().map(|(_, device)| device.clone()),
            client_id: client.map(|(id, _)| id),
            agent_id: None,
        })
    }
}

/// Who issued a request.
pub(crate) struct Caller {
    pub sink: ConnSink,
    pub conn: Option<Arc<ConnState>>,
    /// Who caused it (usage log); `None` for the daemon's own housekeeping.
    pub origin: Option<Origin>,
}

impl Caller {
    fn is_peer(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| c.peer.load(Ordering::Relaxed))
    }
}

/// A handler result: either a value, or a peer's raw JSON passed through without re-parsing.
pub(crate) enum Reply {
    Value(Value),
    Raw(Box<RawValue>),
}

impl Reply {
    pub fn into_value(self) -> ApiResult<Value> {
        match self {
            Reply::Value(v) => Ok(v),
            Reply::Raw(raw) => serde_json::from_str(raw.get()).map_err(ApiError::internal),
        }
    }
}

/// Encode the response frame for request `id`.
pub(crate) fn encode_reply(id: u64, reply: ApiResult<Reply>) -> Arc<str> {
    let text = match reply {
        Ok(Reply::Raw(raw)) => Ok(format!("{{\"id\":{id},\"result\":{}}}", raw.get())),
        Ok(Reply::Value(v)) => serde_json::to_string(&Response::from_result(id, Ok(v))),
        Err(e) => serde_json::to_string(&Response::from_result(id, Err(e))),
    }
    .unwrap_or_else(|_| format!("{{\"id\":{id},\"error\":{{\"code\":\"internal\",\"message\":\"unencodable result\"}}}}"));
    Arc::from(text)
}

pub(crate) struct Subsystems {
    pub workspace: Arc<Workspace>,
    pub stats: Arc<Stats>,
    pub terminals: Arc<Terminals>,
    pub agents: Arc<Agents>,
    pub browser: Arc<Browser>,
    pub insights: Arc<Insights>,
}

impl App {
    pub fn new(paths: Paths, config: Config, emitter: Emitter, subsystems: Subsystems) -> Arc<App> {
        let (detached_tx, _) = mpsc::channel(1);
        let resync = encode_event(events::SYSTEM_RESYNC, &config.host_id, &json!({})).expect("static event");
        let push = crate::push::Push::new(&paths);
        Arc::new(App {
            host_id: config.host_id.clone(),
            token: config.token.clone(),
            paths,
            config: Mutex::new(config),
            emitter,
            workspace: subsystems.workspace,
            stats: subsystems.stats,
            terminals: subsystems.terminals,
            agents: subsystems.agents,
            insights: subsystems.insights,
            browser: subsystems.browser,
            push,
            peers: RwLock::new(Vec::new()),
            shutdown: watch::Sender::new(false),
            resync,
            omp_version: OnceCell::new(),
            next_conn: AtomicU64::new(1),
            detached: ConnSink::new(0, detached_tx),
            input_logged: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    pub fn detached_caller(&self) -> Caller {
        Caller { sink: self.detached.clone(), conn: None, origin: None }
    }

    pub fn is_shutting_down(&self) -> bool {
        *self.shutdown.borrow()
    }

    /// Starts a link for every configured peer.
    pub fn start_peers(self: &Arc<Self>) {
        let configs = self.config.lock().peers.clone();
        let links: Vec<_> = configs.into_iter().map(|cfg| PeerLink::new(self, cfg, true)).collect();
        for link in &links {
            link.start();
        }
        self.peers.write().extend(links);
    }

    pub fn stop_peers(&self) {
        let links = std::mem::take(&mut *self.peers.write());
        for link in links {
            link.stop();
        }
    }

    pub fn peer(&self, id: &str) -> Option<Arc<PeerLink>> {
        self.peers.read().iter().find(|p| p.id() == id).cloned()
    }

    /// Persist the config with the current peer list.
    pub fn save_config(&self) {
        let peers: Vec<PeerConfig> = self.peers.read().iter().map(|p| p.config()).collect();
        let mut config = self.config.lock();
        config.peers = peers;
        if let Err(err) = config.save(&self.paths) {
            tracing::error!("saving config: {err:#}");
        }
    }

    /// Publish a fleet-view event that must not travel over peer links (it describes this
    /// daemon's peers, which are not the remote's business).
    pub fn emit_fleet<T: Serialize>(&self, event: &str, data: &T) {
        if let Some(json) = encode_event(event, &self.host_id, data) {
            self.emitter.relay(event.to_owned(), self.host_id.clone(), json);
        }
    }

    /// Cleanup when a client connection goes away.
    pub fn conn_closed(&self, conn_id: u64) {
        self.browser.conn_closed(conn_id);
        self.stats.conn_closed(conn_id);
        self.terminals.conn_closed(conn_id);
        for peer in self.peers.read().iter() {
            peer.conn_closed(conn_id);
        }
    }

    /// Route one request: locally, or over the peer link named by `host`.
    ///
    /// A request with an origin that is not nested in another one (a client's, a peer's on
    /// behalf of a client or agent, an agent tool's) runs with that [`ORIGIN`] in scope, so domain
    /// events it causes carry the same actor, and is recorded in the usage log by the host that
    /// executes it. Nested calls (a handler calling another method) are not recorded again.
    pub async fn call(self: &Arc<Self>, caller: &Caller, host: Option<String>, method: &str, params: Value) -> ApiResult<Reply> {
        match caller.origin.clone().filter(|_| current_origin().is_none()) {
            Some(origin) => ORIGIN.scope(origin.clone(), self.route(caller, host, method, params, Some(origin))).await,
            None => self.route(caller, host, method, params, None).await,
        }
    }

    async fn route(self: &Arc<Self>, caller: &Caller, host: Option<String>, method: &str, params: Value, origin: Option<Origin>) -> ApiResult<Reply> {
        match host {
            Some(host) if host != self.host_id => {
                if caller.is_peer() {
                    return Err(ApiError::new("forbidden", "requests from peer links are never forwarded"));
                }
                let link = self.peer(&host).ok_or_else(|| ApiError::not_found(format_args!("host {host}")))?;
                link.forward(&caller.sink, method, params, origin).await
            }
            _ => {
                let action = origin.and_then(|o| crate::insights::Action::begin(self, method, &params, o));
                let result = self.local(caller, method, params).await;
                if let Some(action) = action {
                    action.finish(self, &result);
                }
                result.map(Reply::Value)
            }
        }
    }

    async fn local(self: &Arc<Self>, caller: &Caller, method: &str, params: Value) -> ApiResult<Value> {
        let namespace = method.split_once('.').map_or(method, |(ns, _)| ns);
        match namespace {
            "system" => self.system(caller, method, params).await,
            "hosts" => self.hosts(method, params).await,
            "stats" => self.stats.dispatch(method, params, &caller.sink).await,
            "terminal" => self.terminals.dispatch(method, params, &caller.sink).await,
            "projects" | "git" | "fs" => self.workspace.dispatch(method, params).await,
            "actions" => self.workspace.dispatch_actions(&self.terminals, method, params).await,
            "agents" | "boards" | "schedules" | "secrets" => self.agents.dispatch(method, params).await,
            "browser" => self.browser.dispatch(method, params, &caller.sink).await,
            "push" => self.push.dispatch(method, params).await,
            "insights" => self.insights_method(caller, method, params).await,
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    async fn system(self: &Arc<Self>, caller: &Caller, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "system.hello" => {
                #[derive(Deserialize, Default)]
                struct Client {
                    id: String,
                    device: String,
                }
                #[derive(Deserialize, Default)]
                struct P {
                    client: Option<Client>,
                }
                let p: P = if p.is_null() { P::default() } else { params(p)? };
                if let (Some(conn), Some(client)) = (&caller.conn, p.client) {
                    *conn.client.lock() = Some((client.id.chars().take(32).collect(), crate::insights::device(&client.device).to_owned()));
                }
                ok(json!({
                    "protocol": PROTOCOL_VERSION,
                    "host": self.local_info().await,
                    "hosts": self.hosts_list().await,
                }))
            }
            "system.subscribe" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "lowercase")]
                enum Scope {
                    All,
                    Local,
                }
                #[derive(Deserialize)]
                struct P {
                    scope: Scope,
                }
                let p: P = params(p)?;
                if let Some(conn) = &caller.conn {
                    let local = matches!(p.scope, Scope::Local);
                    conn.scope_local.store(local, Ordering::Relaxed);
                    if local {
                        conn.peer.store(true, Ordering::Relaxed);
                    }
                }
                ok(json!({}))
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    async fn hosts(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "hosts.list" => ok(self.hosts_list().await),
            "hosts.add" => ok(self.add_host(params(p)?).await?),
            "hosts.remove" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                let P { id } = params(p)?;
                if id == self.host_id {
                    return Err(ApiError::invalid("the local host cannot be removed"));
                }
                let link = {
                    let mut peers = self.peers.write();
                    let idx = peers.iter().position(|l| l.id() == id).ok_or_else(|| ApiError::not_found(format_args!("host {id}")))?;
                    peers.remove(idx)
                };
                link.stop();
                self.save_config();
                self.emit_fleet(events::HOST_REMOVED, &json!({ "id": id }));
                ok(json!({}))
            }
            "hosts.rename" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    name: String,
                }
                let P { id, name } = params(p)?;
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(ApiError::invalid("name must not be empty"));
                }
                if id == self.host_id {
                    self.config.lock().name = name;
                    self.save_config();
                    let info = self.local_info().await;
                    self.emitter.emit(events::HOST_UPDATED, &info);
                    ok(info)
                } else {
                    let link = self.peer(&id).ok_or_else(|| ApiError::not_found(format_args!("host {id}")))?;
                    link.set_name(name);
                    self.save_config();
                    let info = link.info();
                    self.emit_fleet(events::HOST_UPDATED, &info);
                    ok(info)
                }
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    async fn add_host(self: &Arc<Self>, p: AddHost) -> ApiResult<HostInfo> {
        let cfg = match p {
            AddHost::Direct { url, token, name } => {
                let (url, token) = crate::peer::parse_direct(&url, token.as_deref()).map_err(ApiError::invalid)?;
                PeerConfig {
                    id: None,
                    name: clean_name(name),
                    kind: PeerKind::Direct,
                    url: Some(url),
                    token: Some(token),
                    target: None,
                    ssh_port: None,
                    remote_port: None,
                    install: false,
                }
            }
            AddHost::Ssh { target, ssh_port, remote_port, token, install, name } => {
                let target = target.trim().to_owned();
                if target.is_empty() || target.starts_with('-') || target.contains(char::is_whitespace) {
                    return Err(ApiError::invalid("invalid ssh target"));
                }
                PeerConfig {
                    id: None,
                    name: clean_name(name),
                    kind: PeerKind::Ssh,
                    url: None,
                    token: token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
                    target: Some(target),
                    ssh_port,
                    remote_port,
                    install,
                }
            }
        };
        let address = cfg.address();
        if self.peers.read().iter().any(|l| l.config().address() == address) {
            return Err(ApiError::conflict(format!("{address} is already in the fleet")));
        }
        let wait = match (cfg.kind, cfg.install) {
            (PeerKind::Direct, _) => Duration::from_secs(20),
            (PeerKind::Ssh, false) => Duration::from_secs(60),
            (PeerKind::Ssh, true) => Duration::from_secs(240),
        };
        let link = PeerLink::new(self, cfg, false);
        link.start();
        if let Err(err) = link.wait_first_outcome(wait).await {
            link.stop();
            return Err(ApiError::unavailable(err));
        }
        {
            let mut peers = self.peers.write();
            let id = link.id();
            if peers.iter().any(|l| l.id() == id) {
                drop(peers);
                link.stop();
                return Err(ApiError::conflict(format!("host {id} is already in the fleet")));
            }
            link.set_registered();
            peers.push(link.clone());
        }
        self.save_config();
        let info = link.info();
        self.emit_fleet(events::HOST_UPDATED, &info);
        Ok(info)
    }

    /// `HostInfo` of this daemon.
    pub async fn local_info(&self) -> HostInfo {
        let omp_version = self.omp_version().await;
        HostInfo {
            id: self.host_id.clone(),
            name: self.config.lock().name.clone(),
            kind: HostKind::Local,
            online: true,
            os: Some(std::env::consts::OS.to_owned()),
            arch: Some(std::env::consts::ARCH.to_owned()),
            version: Some(VERSION.to_owned()),
            address: None,
            error: None,
            omp_version,
        }
    }

    async fn hosts_list(&self) -> Vec<HostInfo> {
        let mut hosts = vec![self.local_info().await];
        hosts.extend(self.peers.read().iter().map(|p| p.info()));
        hosts
    }

    /// `omp --version`, probed once.
    pub async fn omp_version(&self) -> Option<String> {
        self.omp_version
            .get_or_init(|| {
                let configured = self.config.lock().agents.omp_path.clone();
                detect_omp_version(configured)
            })
            .await
            .clone()
    }
}

fn clean_name(name: Option<String>) -> Option<String> {
    name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
enum AddHost {
    /// `token` may be omitted when `url` is a pairing link carrying `#token=…`.
    Direct {
        url: String,
        token: Option<String>,
        name: Option<String>,
    },
    Ssh {
        target: String,
        ssh_port: Option<u16>,
        remote_port: Option<u16>,
        token: Option<String>,
        #[serde(default)]
        install: bool,
        name: Option<String>,
    },
}

async fn detect_omp_version(configured: Option<String>) -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut candidates: Vec<std::ffi::OsString> = configured.into_iter().map(Into::into).collect();
    candidates.push("omp".into());
    if let Some(home) = home {
        candidates.push(home.join(".local/bin/omp").into());
        candidates.push(home.join(".bun/bin/omp").into());
    }
    for candidate in candidates {
        let mut cmd = tokio::process::Command::new(&candidate);
        cmd.arg("--version").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(15), cmd.output()).await else { continue };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
        // `omp/18.12.3`, `omp 18.12.3` or `omp v18.12.3`.
        let version = line.strip_prefix("omp").unwrap_or(line).trim_start_matches(['/', ' ']);
        let version = version.strip_prefix('v').unwrap_or(version);
        if !version.is_empty() {
            return Some(version.to_owned());
        }
    }
    None
}

/// [`Orchestrator`] handed to the agents subsystem.
pub(crate) struct Orch {
    pub app: Weak<App>,
    pub host_id: String,
}

impl Orchestrator for Orch {
    fn call(&self, host: Option<String>, method: String, params: Value) -> BoxFuture<'_, ApiResult<Value>> {
        Box::pin(async move {
            let app = self.app.upgrade().ok_or_else(|| ApiError::unavailable("daemon is shutting down"))?;
            let mut caller = app.detached_caller();
            // An agent's host tool acts for that agent; anything else here is the daemon's own
            // housekeeping (or nested in a request whose origin is already in scope).
            caller.origin = TOOL_AGENT.try_with(|id| Origin::agent(id.clone())).ok();
            app.call(&caller, host, &method, params).await?.into_value()
        })
    }

    fn local_host_id(&self) -> String {
        self.host_id.clone()
    }
}
