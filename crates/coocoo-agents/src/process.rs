//! omp process lifecycle: spawn (with the browser overlay and host tools), the per-process
//! event thread, post-turn state refresh, and crash handling.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use coocoo_proto::cgroup::Leaf;
use coocoo_proto::util::{now_ms, preview};
use coocoo_proto::{
    ApiError, ApiResult, CommandsEvent, DiffStat, ItemKind, Orchestrator, TimelineDelta, TimelineItem, TodoPhase, TodoTask,
    UiRequest, events,
};
use omp_rpc::{
    Client, ClientOptions, Command, Event, GetSessionStatsCommand, GetStateCommand, PromptCommand, SessionState,
    SessionStats, SetAskDialogCommand, SetSubagentSubscriptionCommand, SubagentSubscriptionLevel,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Agents;
use crate::mapper::{Effect, Mapper, Sink};
use crate::secret::Redactor;
use crate::state::{Agent, Life, PendingPrompt, Proc, State, Turn};

/// Session events the supervisor consumes; everything else (`agent_end` with the whole
/// transcript, tool output streaming, turn markers) stays inside omp.
const EVENTS: &[&str] = &[
    "agent_start",
    "message_update",
    "message_end",
    "tool_execution_start",
    "tool_execution_end",
    "auto_compaction_start",
    "auto_compaction_end",
    "auto_retry_start",
    "auto_retry_end",
    "retry_fallback_applied",
    "model_changed",
    "thinking_level_changed",
    "notice",
    "queue_update",
    "todo_auto_clear",
    "irc_message",
];

/// `set_event_filter` with `messageUpdates: "delta"`; the generated command lacks the field.
#[derive(Serialize)]
struct SetEventFilter {
    events: &'static [&'static str],
    #[serde(rename = "messageUpdates")]
    message_updates: &'static str,
}

impl Command for SetEventFilter {
    const NAME: &'static str = "set_event_filter";
    const TIMEOUT_MS: Option<u64> = None;
    type Output = Value;

    fn decode(data: Option<Value>) -> Result<Value, serde_json::Error> {
        Ok(data.unwrap_or(Value::Null))
    }
}

/// omp executable: explicit config, then `PATH`, then the usual install locations.
pub(crate) fn resolve_omp(configured: Option<&str>) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(path) = configured.filter(|p| !p.is_empty()) {
        return match (path.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(path),
        };
    }
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("omp"));
    let fallbacks = home.iter().flat_map(|h| [h.join(".local/bin/omp"), h.join(".bun/bin/omp")]);
    on_path.chain(fallbacks).find(|p| is_executable(p)).unwrap_or_else(|| PathBuf::from("omp"))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

pub(crate) fn client_error(err: omp_rpc::Error) -> ApiError {
    match err {
        omp_rpc::Error::Command { error, .. } => ApiError::conflict(error),
        omp_rpc::Error::Closed => ApiError::unavailable("the agent process exited"),
        omp_rpc::Error::Timeout { command } => ApiError::unavailable(format!("omp did not answer `{command}` in time")),
        other => ApiError::internal(other),
    }
}

fn join_error(err: tokio::task::JoinError) -> ApiError {
    ApiError::internal(format!("background task failed: {err}"))
}

/// Run a blocking omp call off the async runtime.
pub(crate) async fn call<C>(client: Arc<Client>, command: C) -> ApiResult<C::Output>
where
    C: Command + Send + 'static,
    C::Output: Send + 'static,
{
    tokio::task::spawn_blocking(move || client.call(&command)).await.map_err(join_error)?.map_err(client_error)
}

/// Everything needed to start one omp process.
struct SpawnSpec {
    omp: PathBuf,
    dir: PathBuf,
    cwd: String,
    resume: Option<String>,
    model: Option<String>,
    thinking: Option<String>,
    extra_args: Vec<String>,
    cdp_url: String,
    agent_id: String,
    host_id: String,
    tools: Vec<omp_rpc::HostTool>,
    /// The project's secrets (and their shadow variables, [`crate::secret::SHADOW_PREFIX`]).
    env: Vec<(String, String)>,
}

struct Spawned {
    client: Client,
    events: Receiver<Event>,
    state: Option<SessionState>,
    /// The recorded session file was gone, so a fresh session was started.
    lost_session: Option<String>,
    cgroup: Option<Leaf>,
}

const STDERR_LOG: &str = "omp-stderr.log";

/// Last non-empty lines of the process's stderr, for crash reports.
fn stderr_tail(dir: &Path) -> Option<String> {
    let mut file = std::fs::File::open(dir.join(STDERR_LOG)).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(2048))).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" | ");
    (!tail.is_empty()).then(|| preview(&tail, 600))
}

fn spawn_omp(spec: SpawnSpec) -> Result<Spawned, String> {
    std::fs::create_dir_all(&spec.dir).map_err(|e| format!("cannot create {}: {e}", spec.dir.display()))?;
    let overlay = spec.dir.join("omp-overlay.yml");
    // A JSON string is a valid YAML double-quoted scalar.
    let mut yaml = format!("browser:\n  cdpUrl: {}\n", serde_json::Value::String(spec.cdp_url.clone()));
    if !spec.env.is_empty() {
        // omp then replaces the values (collected from the env at startup) with placeholders in
        // everything it sends to the model.
        yaml.push_str("secrets:\n  enabled: true\n");
    }
    std::fs::write(&overlay, yaml).map_err(|e| format!("cannot write {}: {e}", overlay.display()))?;
    let stderr = std::fs::File::create(spec.dir.join(STDERR_LOG)).map_err(|e| format!("cannot create stderr log: {e}"))?;

    let mut process = Process::new(&spec.omp);
    // `rpc-ui` (same wire protocol as `rpc`) additionally enables the tool UI context, so the
    // `ask` tool and tool dialogs reach the user as `extension_ui_request`s.
    process
        .args(["--mode", "rpc-ui", "--allow-home", "--cwd"])
        .arg(&spec.cwd)
        .arg("--config")
        .arg(&overlay)
        .arg("--append-system-prompt")
        .arg(crate::tools::SYSTEM_APPENDIX)
        .current_dir(&spec.cwd)
        .env("COOCOO_AGENT_ID", &spec.agent_id)
        .env("COOCOO_HOST_ID", &spec.host_id)
        .stderr(stderr);
    for (name, value) in &spec.env {
        process.env(name, value);
    }
    let mut lost_session = None;
    if let Some(session) = spec.resume {
        if Path::new(&session).is_file() {
            process.arg("--resume").arg(&session);
        } else {
            lost_session = Some(session);
        }
    }
    if let Some(model) = &spec.model {
        process.arg("--model").arg(model);
    }
    if let Some(thinking) = &spec.thinking {
        process.arg("--thinking").arg(thinking);
    }
    process.args(&spec.extra_args);
    // Its own cgroup: under memory pressure oomd kills this agent and what it started, not the
    // daemon with every other agent.
    let cgroup = coocoo_proto::cgroup::leaf(&format!("agent-{}", spec.agent_id));
    #[cfg(unix)]
    if let Some(leaf) = &cgroup {
        // SAFETY: the hook only makes async-signal-safe syscalls.
        unsafe { std::os::unix::process::CommandExt::pre_exec(&mut process, leaf.joiner()) };
    }

    let options = ClientOptions { tools: spec.tools, ..ClientOptions::default() };
    let (client, events) = Client::spawn(process, options).map_err(|err| {
        let detail = stderr_tail(&spec.dir).map(|t| format!(" ({t})")).unwrap_or_default();
        match err {
            omp_rpc::Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                format!("omp not found at `{}`; install omp or set agents.ompPath", spec.omp.display())
            }
            other => format!("failed to start omp: {other}{detail}"),
        }
    })?;
    // Optional capabilities: older omp builds answer with an error, which is fine.
    for (what, result) in [
        ("set_ask_dialog", client.call(&SetAskDialogCommand { enabled: true }).map(drop)),
        (
            "set_subagent_subscription",
            client.call(&SetSubagentSubscriptionCommand { level: SubagentSubscriptionLevel::Progress }).map(drop),
        ),
        ("set_event_filter", client.call(&SetEventFilter { events: EVENTS, message_updates: "delta" }).map(drop)),
    ] {
        if let Err(err) = result {
            tracing::debug!(agent = %spec.agent_id, %err, "{what} unsupported");
        }
    }
    let state = client.call(&GetStateCommand {}).map_err(|e| tracing::debug!(%e, "get_state failed")).ok();
    Ok(Spawned { client, events, state, lost_session, cgroup })
}

/// Fold `get_state` into the agent state. Returns the previous and the new model when it changed.
fn apply_session_state(st: &mut State, state: SessionState) -> Option<(String, String)> {
    let mut model_change = None;
    if let Some(model) = state.model {
        let model = format!("{}/{}", model.provider, model.id);
        if let Some(previous) = st.summary.model.as_ref().filter(|m| **m != model) {
            model_change = Some((previous.clone(), model.clone()));
        }
        st.summary.model = Some(model);
    }
    if let Some(level) = state.thinking_level {
        st.summary.thinking = Some(level.as_str().to_owned());
    }
    if let Some(file) = state.session_file {
        st.summary.session_file = Some(file);
    }
    if let Some(usage) = state.context_usage {
        st.summary.context_percent = Some(usage.percent as f32);
        st.context_window = u64::try_from(usage.context_window).ok();
    }
    st.queued_steering = state.queued_messages.steering;
    st.queued_follow_up = state.queued_messages.follow_up;
    st.todos = state
        .todo_phases
        .into_iter()
        .map(|phase| TodoPhase {
            name: phase.name,
            tasks: phase
                .tasks
                .into_iter()
                .map(|t| TodoTask { content: t.content, status: t.status.as_str().to_owned() })
                .collect(),
        })
        .collect();
    st.sync();
    model_change
}

fn apply_stats(st: &mut State, stats: SessionStats) {
    st.summary.cost_usd = Some(stats.cost);
    st.summary.tokens = u64::try_from(stats.tokens.total).ok();
    if let Some(usage) = stats.context_usage {
        st.summary.context_percent = Some(usage.percent as f32);
        st.context_window = u64::try_from(usage.context_window).ok();
    }
    if st.summary.session_file.is_none() {
        st.summary.session_file = stats.session_file;
    }
}

/// The daemon's CDP discovery URL for `profile` (`browser.cdpEndpoint`), handed to omp as
/// `browser.cdpUrl`.
async fn cdp_endpoint(orchestrator: &dyn Orchestrator, profile: &str) -> Result<String, String> {
    let fail = |e: &dyn std::fmt::Display| format!("cannot get the browser CDP endpoint for profile `{profile}`: {e}");
    let res = orchestrator
        .call(None, "browser.cdpEndpoint".into(), json!({ "profile": profile }))
        .await
        .map_err(|e| fail(&e.message))?;
    res["url"].as_str().map(str::to_owned).ok_or_else(|| fail(&"response has no `url`"))
}

impl Agents {
    /// The agent's client, spawning (or resuming) its process first when needed.
    pub(crate) async fn ensure_live(self: &Arc<Self>, agent: &Arc<Agent>) -> ApiResult<Arc<Client>> {
        let _lifecycle = agent.lifecycle.lock().await;
        {
            let st = agent.state.lock();
            if st.deleted {
                return Err(ApiError::not_found(format_args!("agent {}", agent.id)));
            }
            if let Some(proc) = &st.proc {
                return Ok(proc.client.clone());
            }
            if st.life == Life::Stopped {
                return Err(ApiError::conflict("the agent is archived"));
            }
        }
        self.spawn_locked(agent).await
    }

    /// Spawn the process. Caller holds `agent.lifecycle`.
    async fn spawn_locked(self: &Arc<Self>, agent: &Arc<Agent>) -> ApiResult<Arc<Client>> {
        let orchestrator = self.orch()?;
        let project_id = agent.state.lock().summary.project_id.clone();
        // Told to the agent in the `kanban_done` description.
        let done_action =
            project_id.as_deref().and_then(|pid| self.edit_board(pid, |b| Ok((b.done_action, false))).ok()).unwrap_or_default();
        let secrets = self.spawn_secrets(project_id.as_deref());
        let profile = project_id.unwrap_or_else(|| "default".to_owned());
        let cdp_url = match cdp_endpoint(orchestrator.as_ref(), &profile).await {
            Ok(url) => url,
            Err(message) => return Err(self.spawn_failed(agent, message).await),
        };
        let spec = {
            let mut st = agent.state.lock();
            st.life = Life::Starting;
            st.summary.error = None;
            st.spawn_versions = secrets.versions;
            // This process gets every secret saved so far.
            st.secret_restart.clear();
            st.sync();
            SpawnSpec {
                omp: self.omp.clone(),
                dir: agent.dir.clone(),
                cwd: st.summary.cwd.clone(),
                resume: st.summary.session_file.clone(),
                model: st.summary.model.clone(),
                thinking: st.summary.thinking.clone(),
                extra_args: self.config.extra_args.clone(),
                cdp_url,
                agent_id: agent.id.clone(),
                host_id: self.host_id().to_owned(),
                tools: crate::tools::host_tools(self, &agent.id, orchestrator.clone(), done_action),
                env: secrets.env,
            }
        };
        let resume = spec.resume.is_some();
        self.changed(agent);

        let spawned = tokio::task::spawn_blocking(move || spawn_omp(spec)).await.map_err(join_error)?;
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(message) => return Err(self.spawn_failed(agent, message).await),
        };

        let client = Arc::new(spawned.client);
        let (generation, pending, model_change) = {
            let mut st = agent.state.lock();
            st.generation += 1;
            st.proc = Some(Proc { client: client.clone(), pid: client.pid().unwrap_or(0), _cgroup: spawned.cgroup });
            st.life = Life::Live;
            st.idle_since = Instant::now();
            if spawned.lost_session.is_some() {
                st.summary.session_file = None;
            }
            let model_change = spawned.state.map(|s| apply_session_state(&mut st, s)).unwrap_or_default();
            st.sync();
            (st.generation, std::mem::take(&mut st.pending), model_change)
        };
        let me = self.clone();
        let thread_agent = agent.clone();
        let events = spawned.events;
        let spawn_redactor = secrets.redactor;
        let started = std::thread::Builder::new()
            .name(format!("omp-{}", agent.id))
            .spawn(move || me.run_events(&thread_agent, generation, events, spawn_redactor));
        if let Err(err) = started {
            // Without its event thread the process is unusable; tear it down again.
            let (proc, _) = agent.state.lock().take_process(Life::Crashed);
            let _ = tokio::task::spawn_blocking(move || drop(proc)).await;
            return Err(ApiError::internal(format!("cannot start event thread: {err}")));
        }
        let resumed = resume && spawned.lost_session.is_none();
        if let Some(lost) = spawned.lost_session {
            self.push_item(agent, ItemKind::Notice, format!("Session file {lost} is gone; started a new session")).await;
        }
        if resumed {
            self.record_agent(agent, "agent.resume", |event| event);
        }
        if let Some((from, to)) = model_change {
            self.record_model_change(agent, Some(&from), &to, "respawn");
            self.push_item(agent, ItemKind::Notice, format!("Model: {to}")).await;
        }
        self.changed(agent);
        self.ensure_hibernator();

        for prompt in pending {
            if let Err(err) = self.send_prompt(agent, client.clone(), prompt).await {
                self.push_item(agent, ItemKind::Error, format!("Prompt not delivered: {}", err.message)).await;
            }
        }
        Ok(client)
    }

    /// Record a failed start: the agent shows as crashed with `message`.
    async fn spawn_failed(self: &Arc<Self>, agent: &Arc<Agent>, message: String) -> ApiError {
        tracing::warn!(agent = %agent.id, %message, "omp failed to start");
        {
            let mut st = agent.state.lock();
            st.take_process(Life::Crashed);
            st.summary.error = Some(message.clone());
            st.sync();
        }
        self.run_ended(agent);
        self.push_item(agent, ItemKind::Error, message.clone()).await;
        self.changed(agent);
        ApiError::unavailable(message)
    }

    /// Send one prompt whose `outstanding` slot was already reserved (`State::begin_prompt`).
    ///
    /// A slash command omp runs itself (`/context`) never reaches the transcript as a user
    /// message, so it is echoed here; its seq is reserved before sending so the echo sorts before
    /// the `command_output` that arrives ahead of the response.
    pub(crate) async fn send_prompt(&self, agent: &Arc<Agent>, client: Arc<Client>, prompt: PendingPrompt) -> ApiResult<()> {
        let slash = prompt.message.trim_start().starts_with('/').then(|| (agent.timeline.lock().alloc(), prompt.message.trim().to_owned()));
        let command = PromptCommand {
            message: prompt.message,
            images: prompt.images,
            streaming_behavior: Some(prompt.behavior),
        };
        let result = call(client, command).await;
        match &result {
            Ok(ack) if ack.agent_invoked == Some(false) => {
                agent.state.lock().cancel_prompt();
                if let Some((seq, text)) = slash {
                    let item = TimelineItem {
                        seq,
                        id: seq.to_string(),
                        ts: now_ms(),
                        kind: ItemKind::User,
                        text: Some(text),
                        tool: None,
                        partial: false,
                        images: Vec::new(),
                    };
                    if let Err(err) = agent.timeline.lock().upsert(&item) {
                        tracing::warn!(agent = %agent.id, %err, "timeline write failed");
                    }
                    self.emit_item(&agent.id, item);
                }
            }
            Ok(_) => {}
            Err(_) => agent.state.lock().cancel_prompt(),
        }
        self.changed(agent);
        result.map(drop)
    }

    /// Body of the per-process event thread: map frames until the process is gone.
    /// `spawn_redactor` covers the secrets the process started with (also once they are
    /// deleted or replaced, as its env still has them).
    fn run_events(self: Arc<Self>, agent: &Arc<Agent>, generation: u64, events: Receiver<Event>, spawn_redactor: Arc<Redactor>) {
        let mut mapper = Mapper::new();
        let secrets = SinkSecrets { project_id: agent.summary().project_id, spawn: spawn_redactor, cached: None, held: None };
        let mut sink = LiveSink { agents: &self, agent, generation, unsaved: Vec::new(), secrets };
        loop {
            let event = match mapper.deadline() {
                Some(deadline) => match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => {
                        mapper.flush(&mut sink);
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match events.recv() {
                    Ok(event) => event,
                    Err(_) => break,
                },
            };
            match event {
                Event::Notification(notification) => mapper.handle(notification, &mut sink),
                Event::Undecodable { raw, error } => {
                    let kind = raw.get("type").and_then(Value::as_str).unwrap_or("?").to_owned();
                    tracing::debug!(agent = %agent.id, kind, error, "undecodable omp frame");
                }
                Event::UnmatchedResponse(_) => {}
            }
        }
        mapper.finish(&mut sink);
        agent.timeline.lock().close();
        self.on_process_exit(agent, generation);
    }

    /// The event stream ended. Unless the process was stopped on purpose, it crashed.
    fn on_process_exit(&self, agent: &Arc<Agent>, generation: u64) {
        let detail = stderr_tail(&agent.dir);
        let message = match &detail {
            Some(detail) => format!("omp exited unexpectedly: {detail}"),
            None => "omp exited unexpectedly".to_owned(),
        };
        let (proc, requests) = {
            let mut st = agent.state.lock();
            if st.generation != generation || st.deleted {
                return;
            }
            let taken = st.take_process(Life::Crashed);
            st.summary.error = Some(message.clone());
            st.summary.unread = true;
            st.summary.last_activity_at = now_ms();
            st.sync();
            taken
        };
        tracing::warn!(agent = %agent.id, %message, "agent process crashed");
        // Reaps the process group; this is a plain thread, so blocking is fine.
        drop(proc);
        self.record_agent(agent, "agent.crash", |event| event);
        self.requests_dropped(agent, &requests);
        self.push_item_blocking(agent, ItemKind::Error, format!("{message}. Send a message to resume the session."));
        self.changed(agent);
        self.run_ended(agent);
    }

    /// Re-read session state after a turn (or a todo/model change). Coalesced per agent.
    pub(crate) fn schedule_refresh(&self, agent: &Arc<Agent>, full: bool) {
        agent.refresh_pending.fetch_or(if full { REFRESH_FULL } else { REFRESH_STATE }, Ordering::AcqRel);
        if agent.refresh_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        let agent = agent.clone();
        self.handle().spawn(async move {
            loop {
                let bits = agent.refresh_pending.swap(0, Ordering::AcqRel);
                if bits == 0 {
                    agent.refresh_running.store(false, Ordering::Release);
                    if agent.refresh_pending.load(Ordering::Acquire) != 0
                        && !agent.refresh_running.swap(true, Ordering::AcqRel)
                    {
                        continue;
                    }
                    return;
                }
                me.refresh(&agent, bits & REFRESH_FULL == REFRESH_FULL).await;
            }
        });
    }

    async fn refresh(&self, agent: &Arc<Agent>, full: bool) {
        if let Some(client) = agent.client() {
            let snapshot = tokio::task::spawn_blocking(move || {
                (client.call(&GetStateCommand {}).ok(), client.call(&GetSessionStatsCommand {}).ok())
            })
            .await;
            if let Ok((state, stats)) = snapshot {
                let (model_change, fallback) = {
                    let mut st = agent.state.lock();
                    if !st.is_live() {
                        return;
                    }
                    let change = state.and_then(|s| apply_session_state(&mut st, s));
                    let fallback = change.is_some() && std::mem::take(&mut st.fallback_pending);
                    if let Some(stats) = stats {
                        apply_stats(&mut st, stats);
                    }
                    st.sync();
                    (change, fallback)
                };
                if let Some((from, to)) = model_change {
                    self.record_model_change(agent, Some(&from), &to, if fallback { "fallback" } else { "switch" });
                    self.push_item(agent, ItemKind::Notice, format!("Model: {to}")).await;
                }
                self.changed(agent);
            }
        }
        if !full {
            return;
        }
        let Ok(orchestrator) = self.orch() else { return };
        let (cwd, base, project_id) = {
            let st = agent.state.lock();
            (st.summary.cwd.clone(), st.summary.worktree.as_ref().map(|w| w.base.clone()), st.summary.project_id.clone())
        };
        let mut params = json!({ "cwd": cwd });
        if let Some(base) = base {
            params["base"] = Value::String(base);
        }
        match orchestrator.call(None, "git.diffStat".to_owned(), params).await {
            Ok(value) => {
                if let Ok(diff) = serde_json::from_value::<DiffStat>(value) {
                    agent.state.lock().summary.diff = Some(diff);
                    self.changed(agent);
                }
            }
            Err(err) => tracing::debug!(agent = %agent.id, %err, "diff stat unavailable"),
        }
        if let Some(project_id) = project_id
            && let Err(err) = orchestrator.call(None, "projects.refresh".to_owned(), json!({ "id": project_id })).await
        {
            tracing::debug!(agent = %agent.id, %err, "project refresh failed");
        }
    }
}

pub(crate) const REFRESH_STATE: u8 = 1;
pub(crate) const REFRESH_FULL: u8 = 3;

/// Applies mapper output to a live agent (on its event thread).
struct LiveSink<'a> {
    agents: &'a Agents,
    agent: &'a Arc<Agent>,
    generation: u64,
    /// Images whose `SaveImage` failed; their item is upserted without them.
    unsaved: Vec<String>,
    secrets: SinkSecrets,
}

/// Keeps secret values out of what the event thread stores and sends ([`crate::secret`]).
struct SinkSecrets {
    project_id: Option<String>,
    /// The secrets the process started with.
    spawn: Arc<Redactor>,
    /// The project's redactor seen last, and its union with `spawn`.
    cached: Option<(Arc<Redactor>, Arc<Redactor>)>,
    /// Streaming text held back as it may be the start of a value: `(item id, text)`.
    held: Option<(String, String)>,
}

impl SinkSecrets {
    /// The project's current secrets plus those the process started with.
    fn redactor(&mut self, agents: &Agents) -> Arc<Redactor> {
        let current = agents.project_redactor(self.project_id.as_deref());
        if let Some((seen, merged)) = &self.cached
            && Arc::ptr_eq(seen, &current)
        {
            return merged.clone();
        }
        let merged = match (current.is_empty(), self.spawn.is_empty()) {
            (_, true) => current.clone(),
            (true, false) => self.spawn.clone(),
            (false, false) => Arc::new(Redactor::merged(&current, &self.spawn)),
        };
        self.cached = Some((current, merged.clone()));
        merged
    }
}

impl LiveSink<'_> {
    /// Mutate state only while this thread's process is still the agent's current one; frames
    /// still queued from a stopped process must not leak into its successor.
    fn update(&self, f: impl FnOnce(&mut State) -> bool) -> bool {
        let mut st = self.agent.state.lock();
        if st.generation != self.generation {
            return false;
        }
        f(&mut st)
    }

    fn turn_ended(&self, turn: Turn) {
        self.agents.record_agent(self.agent, "turn.end", |event| {
            event.ms(now_ms() - turn.started_at).meta("stop", turn.stop.as_str()).meta("tools", turn.tools)
        });
    }
}

impl Sink for LiveSink<'_> {
    fn seq(&mut self) -> u64 {
        self.agent.timeline.lock().alloc()
    }

    fn effect(&mut self, effect: Effect) {
        let (agents, agent) = (self.agents, self.agent);
        match effect {
            Effect::SaveImage { name, bytes } => {
                let dir = agent.dir.join(crate::image::DIR);
                if let Err(err) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(&name), bytes)) {
                    tracing::warn!(agent = %agent.id, image = %name, %err, "image write failed");
                    self.unsaved.push(name);
                }
            }
            Effect::Upsert(mut item) => {
                if !self.unsaved.is_empty() {
                    item.images.retain(|image| !self.unsaved.contains(&image.name));
                    self.unsaved.clear();
                }
                // The item carries the whole text; anything held back from its stream is in it.
                if self.secrets.held.as_ref().is_some_and(|(id, _)| *id == item.id) {
                    self.secrets.held = None;
                }
                self.secrets.redactor(agents).redact_item(&mut item);
                if let Err(err) = agent.timeline.lock().upsert(&item) {
                    tracing::warn!(agent = %agent.id, %err, "timeline write failed");
                }
                agents.emit_item(&agent.id, item);
            }
            Effect::Progress(mut item) => {
                self.secrets.redactor(agents).redact_item(&mut item);
                agent.timeline.lock().overlay(item.clone());
                agents.emit_item(&agent.id, item);
            }
            Effect::Delta { item_id, mut text } => {
                let redactor = self.secrets.redactor(agents);
                if !redactor.is_empty() || self.secrets.held.is_some() {
                    match self.secrets.held.take() {
                        Some((id, held)) if id == item_id => text.insert_str(0, &held),
                        Some((id, held)) => {
                            let held = redactor.redact(&held).unwrap_or(held);
                            agent.timeline.lock().append_text(&id, &held);
                            agents.emitter.emit(events::AGENT_DELTA, &TimelineDelta { agent_id: agent.id.clone(), item_id: id, text: held });
                        }
                        None => {}
                    }
                    // A value split across chunks must not go out half by half.
                    let rest = text.split_off(text.len() - redactor.held_len(&text));
                    if !rest.is_empty() {
                        self.secrets.held = Some((item_id.clone(), rest));
                    }
                    redactor.redact_string(&mut text);
                    if text.is_empty() {
                        return;
                    }
                }
                agent.timeline.lock().append_text(&item_id, &text);
                agents.emitter.emit(events::AGENT_DELTA, &TimelineDelta { agent_id: agent.id.clone(), item_id, text });
            }
            Effect::AgentStart => {
                let mut started = false;
                if self.update(|st| {
                    started = st.on_agent_start();
                    true
                }) {
                    if started {
                        agents.record_agent(agent, "turn.start", |event| event);
                    }
                    agents.changed(agent);
                }
            }
            Effect::PromptResult { status, settled, error } => {
                let (mut started, mut finished, mut ended) = (false, false, None);
                if self.update(|st| {
                    // A prompt that failed before the agent started opens (and ends) its turn here.
                    started = st.turn.is_none();
                    finished = st.on_prompt_result(status, settled, error);
                    ended = if finished { st.turn.take() } else { None };
                    true
                }) {
                    if started {
                        agents.record_agent(agent, "turn.start", |event| event);
                    }
                    if let Some(turn) = ended {
                        self.turn_ended(turn);
                    }
                    if finished {
                        agents.schedule_refresh(agent, true);
                        agents.run_ended(agent);
                        agents.restart_for_secrets_when_idle(agent);
                    }
                    agents.changed(agent);
                }
            }
            Effect::Settled => {
                let (mut finished, mut ended) = (false, None);
                if self.update(|st| {
                    finished = st.on_settled();
                    ended = if finished { st.turn.take() } else { None };
                    true
                }) {
                    if let Some(turn) = ended {
                        self.turn_ended(turn);
                    }
                    if finished {
                        agents.schedule_refresh(agent, true);
                        agents.run_ended(agent);
                        agents.restart_for_secrets_when_idle(agent);
                    }
                    agents.changed(agent);
                }
            }
            Effect::Request { id, method, mut payload } => {
                self.secrets.redactor(agents).redact_value(&mut payload);
                let questions = payload.get("questions").and_then(Value::as_array).map_or(1, Vec::len);
                let request = UiRequest { id, agent_id: agent.id.clone(), method, payload, created_at: now_ms() };
                let added = self.update(|st| {
                    st.add_request(request.clone());
                    true
                });
                if added {
                    agents.emitter.emit(events::AGENT_REQUEST, &request);
                    agents.record_agent(agent, "request.ask", |event| {
                        event.meta("method", request.method.as_str()).meta("questions", questions)
                    });
                    agents.changed(agent);
                }
            }
            Effect::Resolve { id } => {
                let mut resolved = None;
                if self.update(|st| {
                    resolved = st.resolve_request(&id);
                    resolved.is_some()
                }) {
                    agents.emit_resolved(&agent.id, &id);
                    if let Some(request) = &resolved {
                        agents.request_closed(agent, request, Some("omp"));
                    }
                    agents.changed(agent);
                }
            }
            Effect::SubagentStarted(id) => {
                if self.update(|st| {
                    let added = st.subagents.insert(id);
                    st.sync();
                    added
                }) {
                    agents.changed(agent);
                }
            }
            Effect::SubagentEnded(id) => {
                if self.update(|st| {
                    let removed = st.subagents.remove(&id);
                    st.sync();
                    removed
                }) {
                    agents.changed(agent);
                    agents.restart_for_secrets_when_idle(agent);
                }
            }
            Effect::Queue { steering, follow_up } => {
                if self.update(|st| {
                    st.queued_steering = steering;
                    st.queued_follow_up = follow_up;
                    true
                }) {
                    agents.changed(agent);
                }
            }
            Effect::Thinking(level) => {
                if self.update(|st| {
                    let changed = st.summary.thinking.as_deref() != Some(level.as_str());
                    st.summary.thinking = Some(level);
                    changed
                }) {
                    agents.changed(agent);
                }
            }
            Effect::Output(mut text) => {
                self.secrets.redactor(agents).redact_string(&mut text);
                if self.update(|st| {
                    st.summary.preview = Some(preview(&text, 240));
                    st.last_output = Some(text);
                    true
                }) {
                    agents.changed(agent);
                }
            }
            Effect::Refresh => agents.schedule_refresh(agent, false),
            Effect::TodosCleared => {
                if self.update(|st| {
                    st.todos.clear();
                    st.sync();
                    true
                }) {
                    agents.changed(agent);
                }
            }
            Effect::Title(title) => {
                if self.update(|st| {
                    let changed = st.summary.title != title;
                    st.summary.title = title;
                    changed
                }) {
                    agents.changed(agent);
                }
            }
            Effect::Commands(commands) => {
                if self.update(|st| {
                    st.commands = Some(commands.clone());
                    true
                }) {
                    agents.emitter.emit(events::AGENT_COMMANDS, &CommandsEvent { agent_id: agent.id.clone(), commands });
                    agents.changed(agent);
                }
            }
            Effect::ToolStarted => {
                self.update(|st| {
                    if let Some(turn) = st.turn.as_mut() {
                        turn.tools += 1;
                    }
                    false
                });
            }
            Effect::RetryStart { attempt } => {
                if self.update(|_| true) {
                    agents.record_agent(agent, "retry.start", |event| event.meta("attempt", attempt));
                }
            }
            Effect::RetryEnd { ok } => {
                if self.update(|_| true) {
                    agents.record_agent(agent, "retry.end", |event| event.meta("ok", ok));
                }
            }
            Effect::Compaction { ok } => {
                if self.update(|_| true) {
                    agents.record_agent(agent, "compaction", |event| event.meta("ok", ok));
                }
            }
            Effect::ModelFallback => {
                self.update(|st| {
                    st.fallback_pending = true;
                    false
                });
            }
        }
    }
}
