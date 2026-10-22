//! Agent supervisor. Every agent is one `omp --mode rpc-ui` process (the RPC protocol plus
//! omp's tool dialogs such as `ask`) driven through the vendored official client (`omp-rpc`).
//! Handles the `agents.*` methods.
//!
//! - Processes start lazily (on create/prompt) and are hibernated when idle; a hibernated agent
//!   resumes its omp session (`--resume <sessionFile>`) on the next prompt.
//! - Each live process has one dedicated thread that maps its events to timeline items and
//!   status changes ([`mapper`]); nothing polls while agents are idle.
//! - State survives daemon restarts: `<data_dir>/agents/<id>/meta.json` + `timeline.jsonl`.

mod api;
mod board;
mod mapper;
mod process;
mod state;
mod timeline;
mod tools;
mod transcript;
mod usage;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use coocoo_proto::util::{now_ms, read_json, write_json_atomic};
use coocoo_proto::{
    ApiError, ApiResult, Emitter, ItemKind, ModelInfo, Orchestrator, Paths, ProviderUsage, TimelineEvent, TimelineItem,
    events,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Handle;

use crate::state::{Agent, Life, Meta, State};

/// `[agents]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentsConfig {
    /// omp executable; default: `omp` on PATH, then `~/.local/bin/omp`, `~/.bun/bin/omp`.
    pub omp_path: Option<String>,
    /// Stop idle agent processes after this many seconds (session kept, resumed on demand).
    /// `0` disables hibernation.
    pub hibernate_after_secs: u64,
    /// Default `provider/modelId` for new agents; `None` = omp's configured default.
    pub default_model: Option<String>,
    pub default_thinking: Option<String>,
    /// Extra CLI args appended to every `omp --mode rpc` invocation.
    pub extra_args: Vec<String>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self { omp_path: None, hibernate_after_secs: 900, default_model: None, default_thinking: None, extra_args: Vec::new() }
    }
}

/// Delay that coalesces bursts of summary changes into one `agent.updated` + `meta.json` write.
const SUMMARY_COALESCE: Duration = Duration::from_millis(80);
/// How often the hibernation sweep runs while at least one process is alive.
const HIBERNATE_TICK: Duration = Duration::from_secs(30);

pub struct Agents {
    me: Weak<Agents>,
    emitter: Emitter,
    /// `<data_dir>/agents`.
    root: PathBuf,
    config: AgentsConfig,
    omp: PathBuf,
    orchestrator: OnceLock<Arc<dyn Orchestrator>>,
    runtime: OnceLock<Handle>,
    agents: RwLock<HashMap<String, Arc<Agent>>>,
    models: tokio::sync::Mutex<Option<(Instant, Vec<ModelInfo>)>>,
    /// `agents.usage` per provider; one fetch at a time so concurrent viewers share it.
    usage: tokio::sync::Mutex<HashMap<String, (Instant, ProviderUsage)>>,
    /// The hibernation sweep task is running.
    hibernator: AtomicBool,
}

impl Agents {
    /// Loads persisted agents from `<data_dir>/agents/`; previously live agents come back as
    /// `hibernated` (no process is spawned until they are prompted).
    pub fn new(emitter: Emitter, paths: &Paths, config: AgentsConfig) -> Arc<Self> {
        let root = paths.data_dir.join("agents");
        let agents = load_agents(&root);
        let omp = process::resolve_omp(config.omp_path.as_deref());
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            emitter,
            root,
            config,
            omp,
            orchestrator: OnceLock::new(),
            runtime: OnceLock::new(),
            agents: RwLock::new(agents),
            models: tokio::sync::Mutex::new(None),
            usage: tokio::sync::Mutex::new(HashMap::new()),
            hibernator: AtomicBool::new(false),
        })
    }

    /// Must be called once before the first `dispatch`, inside the runtime. Also starts the
    /// board source poller.
    pub fn set_orchestrator(self: &Arc<Self>, orchestrator: Arc<dyn Orchestrator>) {
        let _ = self.orchestrator.set(orchestrator);
        if let Ok(handle) = Handle::try_current() {
            let _ = self.runtime.set(handle);
            self.start_board_poller();
        }
    }

    /// `agents.*`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value) -> ApiResult<Value> {
        self.runtime.get_or_init(Handle::current);
        self.handle_method(method, params).await
    }

    /// `(agent id, omp pid)` for every live process.
    pub fn pids(&self) -> Vec<(String, u32)> {
        self.agents
            .read()
            .values()
            .filter_map(|agent| agent.state.lock().proc.as_ref().map(|p| (agent.id.clone(), p.pid)))
            .collect()
    }

    /// Persist state and stop every omp process (sessions stay resumable).
    pub async fn shutdown(&self) {
        let agents: Vec<Arc<Agent>> = self.agents.read().values().cloned().collect();
        let mut teardown = Vec::new();
        for agent in agents {
            let (proc, _) = {
                let mut st = agent.state.lock();
                if !matches!(st.life, Life::Live | Life::Starting) {
                    continue;
                }
                st.take_process(Life::Hibernated)
            };
            teardown.push(tokio::task::spawn_blocking(move || {
                drop(proc);
                write_meta(&agent);
            }));
        }
        for task in teardown {
            let _ = task.await;
        }
    }

    // ─── shared plumbing ──────────────────────────────────────────────────────────────────

    fn handle(&self) -> Handle {
        self.runtime.get_or_init(Handle::current).clone()
    }

    fn orch(&self) -> ApiResult<Arc<dyn Orchestrator>> {
        self.orchestrator.get().cloned().ok_or_else(|| ApiError::unavailable("orchestrator not initialized"))
    }

    fn get(&self, id: &str) -> ApiResult<Arc<Agent>> {
        self.agents.read().get(id).cloned().ok_or_else(|| ApiError::not_found(format_args!("agent {id}")))
    }

    fn host_id(&self) -> &str {
        self.emitter.host_id()
    }

    /// The agent's summary changed: emit `agent.updated` and persist `meta.json`, coalesced.
    fn changed(&self, agent: &Arc<Agent>) {
        if agent.flush_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        let agent = agent.clone();
        self.handle().spawn(async move {
            tokio::time::sleep(SUMMARY_COALESCE).await;
            agent.flush_scheduled.store(false, Ordering::Release);
            let summary = {
                let st = agent.state.lock();
                if st.deleted {
                    return;
                }
                st.summary.clone()
            };
            me.emitter.emit(events::AGENT_UPDATED, &summary);
            let _ = tokio::task::spawn_blocking(move || write_meta(&agent)).await;
        });
    }

    fn emit_item(&self, agent_id: &str, item: TimelineItem) {
        self.emitter.emit(events::AGENT_TIMELINE, &TimelineEvent { agent_id: agent_id.to_owned(), item });
    }

    fn emit_resolved(&self, agent_id: &str, request_id: &str) {
        self.emitter.emit(events::AGENT_REQUEST_RESOLVED, &json!({ "agentId": agent_id, "requestId": request_id }));
    }

    /// Append a finished notice/error item. Blocking (file append): call off the async runtime.
    fn push_item_blocking(&self, agent: &Agent, kind: ItemKind, text: String) {
        let live = agent.state.lock().is_live();
        let item = {
            let mut tl = agent.timeline.lock();
            let seq = tl.alloc();
            let item = TimelineItem { seq, id: seq.to_string(), ts: now_ms(), kind, text: Some(text), tool: None, partial: false };
            if let Err(err) = tl.upsert(&item) {
                tracing::warn!(agent = %agent.id, %err, "timeline write failed");
            }
            if !live {
                // No more writes expected until the next spawn; don't hold the descriptor.
                tl.close();
            }
            item
        };
        self.emit_item(&agent.id, item);
    }

    async fn push_item(&self, agent: &Arc<Agent>, kind: ItemKind, text: String) {
        let Some(me) = self.me.upgrade() else { return };
        let agent = agent.clone();
        let _ = tokio::task::spawn_blocking(move || me.push_item_blocking(&agent, kind, text)).await;
    }

    /// Start the hibernation sweep unless it runs already (or hibernation is disabled). The
    /// sweep ends by itself once no process is alive.
    fn ensure_hibernator(&self) {
        if self.config.hibernate_after_secs == 0 || self.hibernator.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.me.clone();
        self.handle().spawn(async move {
            let mut tick = tokio::time::interval(HIBERNATE_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(this) = me.upgrade() else { return };
                this.sweep().await;
                if !this.any_live() {
                    this.hibernator.store(false, Ordering::Release);
                    // A process may have started between the check and the store.
                    if !this.any_live() || this.hibernator.swap(true, Ordering::AcqRel) {
                        return;
                    }
                }
            }
        });
    }

    fn any_live(&self) -> bool {
        self.agents.read().values().any(|a| matches!(a.state.lock().life, Life::Live | Life::Starting))
    }

    async fn sweep(&self) {
        let after = Duration::from_secs(self.config.hibernate_after_secs);
        let idle: Vec<Arc<Agent>> = self
            .agents
            .read()
            .values()
            .filter(|agent| {
                let st = agent.state.lock();
                st.is_live()
                    && !st.busy()
                    && st.requests.is_empty()
                    && st.subagents.is_empty()
                    && st.idle_since.elapsed() >= after
            })
            .cloned()
            .collect();
        for agent in idle {
            let minutes = after.as_secs().div_ceil(60);
            let note = format!("Hibernated after {minutes} min idle; the session resumes on the next message");
            if let Err(err) = self.stop_process(&agent, Life::Hibernated, Some(note)).await {
                tracing::warn!(agent = %agent.id, %err, "hibernation failed");
            }
        }
    }

    /// Stop the agent's process (if any) and move it to `life`. Pending dialogs are resolved.
    async fn stop_process(&self, agent: &Arc<Agent>, life: Life, note: Option<String>) -> ApiResult<()> {
        let _lifecycle = agent.lifecycle.lock().await;
        let (proc, requests) = {
            let mut st = agent.state.lock();
            if st.deleted {
                return Err(ApiError::not_found(format_args!("agent {}", agent.id)));
            }
            if st.life == life || (life == Life::Hibernated && !st.is_live()) {
                return Ok(());
            }
            st.take_process(life)
        };
        for request in &requests {
            self.emit_resolved(&agent.id, &request.id);
        }
        if let Some(proc) = proc {
            // Teardown (SIGTERM, wait, SIGKILL) blocks for up to ~2 s.
            let _ = tokio::task::spawn_blocking(move || drop(proc)).await;
        }
        if let Some(note) = note {
            self.push_item(agent, ItemKind::Notice, note).await;
        }
        self.changed(agent);
        Ok(())
    }
}

/// Write `meta.json` from the current state (blocking).
fn write_meta(agent: &Agent) {
    let _guard = agent.meta_write.lock();
    let meta = {
        let st = agent.state.lock();
        if st.deleted {
            return;
        }
        st.meta()
    };
    if let Err(err) = write_json_atomic(&agent.dir.join("meta.json"), &meta) {
        tracing::warn!(agent = %agent.id, %err, "failed to persist agent state");
    }
}

fn load_agents(root: &Path) -> HashMap<String, Arc<Agent>> {
    let mut agents = HashMap::new();
    let Ok(entries) = std::fs::read_dir(root) else { return agents };
    for entry in entries.flatten() {
        let dir = entry.path();
        let meta = match read_json::<Meta>(&dir.join("meta.json")) {
            Ok(Some(meta)) => meta,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(dir = %dir.display(), %err, "skipping unreadable agent");
                continue;
            }
        };
        let before = meta.summary.status;
        let state = State::from_meta(meta);
        let rewrite = state.summary.status != before;
        let agent = Arc::new(Agent::new(dir, state));
        if rewrite {
            write_meta(&agent);
        }
        agents.insert(agent.id.clone(), agent);
    }
    agents
}
