//! Agent supervisor. Every agent is one `omp --mode rpc-ui` process (the RPC protocol plus
//! omp's tool dialogs such as `ask`) driven through the vendored official client (`omp-rpc`).
//! Handles the `agents.*` methods.
//!
//! - Processes start lazily (on create/prompt) and are hibernated when idle; a hibernated agent
//!   resumes its omp session (`--resume <sessionFile>`) on the next prompt.
//! - Each live process has one dedicated thread that maps its events to timeline items and
//!   status changes ([`mapper`]); nothing polls while agents are idle.
//! - State survives daemon restarts: `<data_dir>/agents/<id>/meta.json` + `timeline.jsonl`,
//!   plus the images of user messages in `images/` (`<seq>-<index>.<ext>`, `agents.image`).
//!   A restarting daemon lets running agents pause first and tells them when it is back
//!   ([`restart`]).

mod api;
mod board;
mod cron;
mod image;
mod mapper;
mod process;
mod restart;
mod sandbox;
mod schedule;
mod slash;
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

use coocoo_proto::insights::{InsightEvent, InsightsSink, LedgerEntry};
use coocoo_proto::util::{now_ms, read_json, write_json_atomic};
use coocoo_proto::{
    AgentSummary, ApiError, ApiResult, Emitter, ItemKind, ModelInfo, Orchestrator, Paths, ProviderUsage, TimelineEvent,
    TimelineItem, UiRequest, events,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Handle;

use crate::state::{Agent, Life, Meta, RestartMark, State};

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
    /// On a restart, how long running agents get to pause before their processes are stopped.
    pub restart_grace_secs: u64,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            omp_path: None,
            hibernate_after_secs: 900,
            default_model: None,
            default_thinking: None,
            extra_args: Vec::new(),
            restart_grace_secs: 60,
        }
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
    /// Usage log and agent ledger; without one nothing is recorded.
    insights: OnceLock<Arc<dyn InsightsSink>>,
    runtime: OnceLock<Handle>,
    agents: RwLock<HashMap<String, Arc<Agent>>>,
    models: tokio::sync::Mutex<Option<(Instant, Vec<ModelInfo>)>>,
    /// `agents.usage` per provider; one fetch at a time so concurrent viewers share it.
    usage: tokio::sync::Mutex<HashMap<String, (Instant, ProviderUsage)>>,
    /// The hibernation sweep task is running.
    hibernator: AtomicBool,
    /// A restart is draining the agents ([`Agents::drain`]): new prompts and agents are refused.
    draining: AtomicBool,
    /// `<data_dir>/boards`: one Kanban board per project.
    boards_dir: PathBuf,
    /// Boards loaded so far, by project id.
    boards: parking_lot::Mutex<HashMap<String, coocoo_proto::Board>>,
    /// Every project's scheduled runs (`schedules.*`), persisted in `schedules_file`.
    schedules: parking_lot::Mutex<Vec<coocoo_proto::Schedule>>,
    schedules_file: PathBuf,
    /// Wakes the scheduler when the schedules change.
    schedule_wake: Arc<tokio::sync::Notify>,
}

impl Agents {
    /// Loads persisted agents from `<data_dir>/agents/`; previously live agents come back as
    /// `hibernated` (no process is spawned until they are prompted). Boards of earlier versions,
    /// kept per agent, move to their projects here.
    pub fn new(emitter: Emitter, paths: &Paths, config: AgentsConfig) -> Arc<Self> {
        let root = paths.data_dir.join("agents");
        let agents = load_agents(&root);
        let boards_dir = paths.data_dir.join("boards");
        let schedules_file = paths.data_dir.join("schedules.json");
        let schedules = schedule::load(&schedules_file);
        board::migrate_agent_boards(&root, &boards_dir, |id| {
            let summary = agents.get(id)?.summary();
            Some((summary.project_id?, summary.title))
        });
        let omp = process::resolve_omp(config.omp_path.as_deref());
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            emitter,
            root,
            config,
            omp,
            orchestrator: OnceLock::new(),
            insights: OnceLock::new(),
            runtime: OnceLock::new(),
            agents: RwLock::new(agents),
            models: tokio::sync::Mutex::new(None),
            usage: tokio::sync::Mutex::new(HashMap::new()),
            hibernator: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            boards_dir,
            boards: parking_lot::Mutex::new(HashMap::new()),
            schedules: parking_lot::Mutex::new(schedules),
            schedules_file,
            schedule_wake: Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Must be called once before the first `dispatch`, inside the runtime. Also starts the
    /// board source poller and the scheduler.
    pub fn set_orchestrator(self: &Arc<Self>, orchestrator: Arc<dyn Orchestrator>) {
        let _ = self.orchestrator.set(orchestrator);
        if let Ok(handle) = Handle::try_current() {
            let _ = self.runtime.set(handle);
            self.start_board_poller();
            self.start_scheduler();
        }
    }

    /// Where domain events and agent identity changes are reported; set once at startup.
    pub fn set_insights(&self, sink: Arc<dyn InsightsSink>) {
        let _ = self.insights.set(sink);
    }

    /// The omp executable agents run (its `stats` subcommand feeds the usage statistics).
    pub fn omp_path(&self) -> &Path {
        &self.omp
    }

    /// Summaries of every agent on this host.
    pub fn summaries(&self) -> Vec<AgentSummary> {
        self.agents.read().values().map(|agent| agent.summary()).collect()
    }

    /// Summary of agent `id`, if this host has it.
    pub fn summary(&self, id: &str) -> Option<AgentSummary> {
        self.agents.read().get(id).map(|agent| agent.summary())
    }

    /// `agents.*`, `boards.*` and `schedules.*`.
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

    /// Persist state and stop every omp process (sessions stay resumable). After
    /// [`Agents::drain`], agents still in a turn or dialog are marked to be told on the next
    /// start that the restart cut them off.
    pub async fn shutdown(&self) {
        let draining = self.draining.load(Ordering::Acquire);
        let agents: Vec<Arc<Agent>> = self.agents.read().values().cloned().collect();
        let mut teardown = Vec::new();
        for agent in agents {
            let (proc, requests) = {
                let mut st = agent.state.lock();
                if !matches!(st.life, Life::Live | Life::Starting) {
                    continue;
                }
                if draining && (st.busy() || !st.requests.is_empty()) {
                    st.restart = Some(RestartMark::Interrupted);
                }
                st.take_process(Life::Hibernated)
            };
            for request in &requests {
                self.request_closed(&agent, request, Some("process"));
            }
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

    /// The agent's summary changed: emit `agent.updated`, persist `meta.json` and report its
    /// identity to the agent ledger, coalesced.
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
            me.record_ledger(&summary, None);
            let _ = tokio::task::spawn_blocking(move || write_meta(&agent)).await;
        });
    }

    fn emit_item(&self, agent_id: &str, item: TimelineItem) {
        self.emitter.emit(events::AGENT_TIMELINE, &TimelineEvent { agent_id: agent_id.to_owned(), item });
    }

    /// Report a usage-log event (no-op without a sink).
    fn record(&self, build: impl FnOnce() -> InsightEvent) {
        if let Some(sink) = self.insights.get() {
            sink.event(build());
        }
    }

    /// Report a domain event about `agent`, tagged with its id and project. Locks the agent's
    /// state, so never call it while holding that lock.
    fn record_agent(&self, agent: &Agent, kind: &str, build: impl FnOnce(InsightEvent) -> InsightEvent) {
        self.record(|| {
            let project = agent.state.lock().summary.project_id.clone();
            build(InsightEvent::new(kind).agent(agent.id.as_str()).project(project))
        });
    }

    /// Report the agent's identity to the ledger, which skips entries that did not change.
    fn record_ledger(&self, summary: &AgentSummary, deleted_at: Option<i64>) {
        if let Some(sink) = self.insights.get() {
            sink.agent(LedgerEntry {
                ts: now_ms(),
                agent_id: summary.id.clone(),
                title: summary.title.clone(),
                project_id: summary.project_id.clone(),
                cwd: summary.cwd.clone(),
                worktree: summary.worktree.as_ref().map(|w| w.path.clone()),
                session_file: summary.session_file.clone(),
                parent_id: summary.parent_id.clone(),
                created_at: summary.created_at,
                deleted_at,
            });
        }
    }

    /// A dialog was answered (`cancel` = `None`) or cancelled (`user`, `omp`, `process`).
    fn request_closed(&self, agent: &Agent, request: &UiRequest, cancel: Option<&'static str>) {
        let kind = if cancel.is_some() { "request.cancel" } else { "request.answer" };
        self.record_agent(agent, kind, |event| {
            let event = event.ms(now_ms() - request.created_at).meta("method", request.method.as_str());
            match cancel {
                Some(reason) => event.meta("reason", reason),
                None => event,
            }
        });
    }

    /// Dialogs dropped with the agent's process: gone for the clients, cancelled in the log.
    fn requests_dropped(&self, agent: &Agent, requests: &[UiRequest]) {
        for request in requests {
            self.emit_resolved(&agent.id, &request.id);
            self.request_closed(agent, request, Some("process"));
        }
    }

    /// Report a model change (`cause`: user, switch, fallback, respawn); `from` is `None` when
    /// the previous model was not known.
    fn record_model_change(&self, agent: &Agent, from: Option<&str>, to: &str, cause: &'static str) {
        self.record_agent(agent, "model.change", |event| {
            let event = event.meta("to", to).meta("cause", cause);
            match from {
                Some(from) => event.meta("from", from),
                None => event,
            }
        });
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
            let item = TimelineItem { seq, id: seq.to_string(), ts: now_ms(), kind, text: Some(text), tool: None, partial: false, images: Vec::new() };
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
            if let Err(err) = self.hibernate(&agent, note).await {
                tracing::warn!(agent = %agent.id, %err, "hibernation failed");
            }
        }
    }

    /// Hibernate a live agent (idle sweep, `agents.hibernate`).
    async fn hibernate(&self, agent: &Arc<Agent>, note: String) -> ApiResult<()> {
        if self.stop_process(agent, Life::Hibernated, Some(note)).await? {
            self.record_agent(agent, "agent.hibernate", |event| event);
        }
        Ok(())
    }

    /// Stop the agent's process (if any) and move it to `life`. Pending dialogs are resolved.
    /// Returns `false` when the agent was in that state already.
    async fn stop_process(&self, agent: &Arc<Agent>, life: Life, note: Option<String>) -> ApiResult<bool> {
        let _lifecycle = agent.lifecycle.lock().await;
        let (proc, requests) = {
            let mut st = agent.state.lock();
            if st.deleted {
                return Err(ApiError::not_found(format_args!("agent {}", agent.id)));
            }
            if st.life == life || (life == Life::Hibernated && !st.is_live()) {
                return Ok(false);
            }
            st.take_process(life)
        };
        self.requests_dropped(agent, &requests);
        if let Some(proc) = proc {
            // Teardown (SIGTERM, wait, SIGKILL) blocks for up to ~2 s.
            let _ = tokio::task::spawn_blocking(move || drop(proc)).await;
        }
        if let Some(note) = note {
            self.push_item(agent, ItemKind::Notice, note).await;
        }
        self.changed(agent);
        Ok(true)
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
