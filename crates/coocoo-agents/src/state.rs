//! In-memory agent state and the status machine.
//!
//! `AgentSummary.status` is never set directly: it is derived from the process lifecycle
//! ([`Life`]), outstanding prompts, the streaming flag, pending dialogs and the last turn's
//! error, so concurrent updates from the event thread and API calls cannot disagree.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::time::Instant;

use coocoo_proto::util::now_ms;
use coocoo_proto::{AgentDetail, AgentStatus, AgentSummary, TodoPhase, TodoSummary, UiRequest};
use omp_rpc::{Client, ImageContent, PromptStatus, StreamingBehavior};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::timeline::Timeline;

/// Process lifecycle of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Life {
    /// No process; resumes on the next prompt.
    Hibernated,
    /// Archived: no process, read-only.
    Stopped,
    /// The process exited unexpectedly or failed to start; resumable.
    Crashed,
    Starting,
    Live,
}

/// The running omp process of an agent.
pub(crate) struct Proc {
    pub client: Arc<Client>,
    pub pid: u32,
}

/// A prompt accepted before the process was ready.
pub(crate) struct PendingPrompt {
    pub message: String,
    pub images: Option<Vec<ImageContent>>,
    pub behavior: StreamingBehavior,
}

/// `meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meta {
    pub summary: AgentSummary,
    /// Full text of the last assistant message (`agents.output`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_output: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<TodoPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

pub(crate) struct State {
    pub summary: AgentSummary,
    pub last_output: Option<String>,
    pub todos: Vec<TodoPhase>,
    pub queued_steering: Vec<String>,
    pub queued_follow_up: Vec<String>,
    pub requests: Vec<UiRequest>,
    pub context_window: Option<u64>,
    pub life: Life,
    pub proc: Option<Proc>,
    /// Incremented per spawn; the event thread of an older process must not touch the state.
    pub generation: u64,
    /// Prompts sent (or queued for sending) whose `prompt_result` has not arrived.
    pub outstanding: u32,
    /// Between `agent_start` and the end of the run.
    pub streaming: bool,
    pub subagents: HashSet<String>,
    pub idle_since: Instant,
    pub pending: Vec<PendingPrompt>,
    pub deleted: bool,
}

impl State {
    pub fn new(summary: AgentSummary, life: Life) -> Self {
        let mut state = Self {
            summary,
            last_output: None,
            todos: Vec::new(),
            queued_steering: Vec::new(),
            queued_follow_up: Vec::new(),
            requests: Vec::new(),
            context_window: None,
            life,
            proc: None,
            generation: 0,
            outstanding: 0,
            streaming: false,
            subagents: HashSet::new(),
            idle_since: Instant::now(),
            pending: Vec::new(),
            deleted: false,
        };
        state.sync();
        state
    }

    /// Restore from `meta.json`: anything that had a process comes back hibernated.
    pub fn from_meta(meta: Meta) -> Self {
        let life = match meta.summary.status {
            AgentStatus::Stopped => Life::Stopped,
            AgentStatus::Error => Life::Crashed,
            _ => Life::Hibernated,
        };
        let mut state = Self::new(meta.summary, life);
        state.last_output = meta.last_output;
        state.todos = meta.todos;
        state.context_window = meta.context_window;
        state.sync();
        state
    }

    pub fn meta(&self) -> Meta {
        Meta {
            summary: self.summary.clone(),
            last_output: self.last_output.clone(),
            todos: self.todos.clone(),
            context_window: self.context_window,
        }
    }

    pub fn detail(&self) -> AgentDetail {
        AgentDetail {
            summary: self.summary.clone(),
            todos: self.todos.clone(),
            queued_steering: self.queued_steering.clone(),
            queued_follow_up: self.queued_follow_up.clone(),
            requests: self.requests.clone(),
            context_window: self.context_window,
        }
    }

    pub fn busy(&self) -> bool {
        self.outstanding > 0 || self.streaming
    }

    pub fn is_live(&self) -> bool {
        self.life == Life::Live
    }

    fn derive(&self) -> AgentStatus {
        match self.life {
            Life::Stopped => AgentStatus::Stopped,
            Life::Hibernated => AgentStatus::Hibernated,
            Life::Crashed => AgentStatus::Error,
            Life::Starting => AgentStatus::Starting,
            Life::Live if !self.requests.is_empty() => AgentStatus::Waiting,
            Life::Live if self.busy() => AgentStatus::Running,
            Life::Live if self.summary.error.is_some() => AgentStatus::Error,
            Life::Live => AgentStatus::Idle,
        }
    }

    /// Recompute derived summary fields.
    pub fn sync(&mut self) {
        self.summary.status = self.derive();
        self.summary.pending_requests = self.requests.len() as u32;
        self.summary.subagents = self.subagents.len() as u32;
        self.summary.todos = todo_summary(&self.todos);
    }

    pub fn touch(&mut self) {
        self.summary.last_activity_at = now_ms();
        self.idle_since = Instant::now();
    }

    pub fn on_agent_start(&mut self) {
        self.streaming = true;
        self.summary.error = None;
        self.touch();
        self.sync();
    }

    /// Returns `true` when the agent finished its work (turn end).
    pub fn on_prompt_result(&mut self, status: PromptStatus, settled: bool, error: Option<String>) -> bool {
        self.outstanding = self.outstanding.saturating_sub(1);
        if settled {
            self.streaming = false;
        }
        if status == PromptStatus::Error {
            self.summary.error = Some(error.unwrap_or_else(|| "the turn failed".to_owned()));
        }
        let finished = !self.busy();
        if finished {
            self.finish_turn();
        }
        self.sync();
        finished
    }

    /// Returns `true` when the agent finished its work (turn end).
    pub fn on_settled(&mut self) -> bool {
        let was_streaming = self.streaming;
        self.streaming = false;
        let finished = was_streaming && !self.busy();
        if finished {
            self.finish_turn();
        }
        self.sync();
        finished
    }

    fn finish_turn(&mut self) {
        self.summary.unread = true;
        self.touch();
    }

    /// A prompt that will produce exactly one `prompt_result` is about to be sent.
    pub fn begin_prompt(&mut self) {
        self.outstanding += 1;
        self.touch();
        self.sync();
    }

    /// A prompt failed to send or finished without invoking the agent (no `prompt_result`).
    pub fn cancel_prompt(&mut self) {
        self.outstanding = self.outstanding.saturating_sub(1);
        self.sync();
    }

    pub fn add_request(&mut self, request: UiRequest) {
        self.requests.retain(|r| r.id != request.id);
        self.requests.push(request);
        self.sync();
    }

    pub fn resolve_request(&mut self, id: &str) -> Option<UiRequest> {
        let at = self.requests.iter().position(|r| r.id == id)?;
        let request = self.requests.remove(at);
        self.sync();
        Some(request)
    }

    /// Process gone (hibernate, archive, crash, shutdown): drop everything tied to it.
    /// Returns the process (to tear down off the async runtime) and the dropped dialogs.
    pub fn take_process(&mut self, life: Life) -> (Option<Proc>, Vec<UiRequest>) {
        self.generation += 1;
        self.life = life;
        self.outstanding = 0;
        self.streaming = false;
        self.subagents.clear();
        self.pending.clear();
        self.queued_steering.clear();
        self.queued_follow_up.clear();
        let requests = std::mem::take(&mut self.requests);
        let proc = self.proc.take();
        self.sync();
        (proc, requests)
    }
}

fn todo_summary(phases: &[TodoPhase]) -> Option<TodoSummary> {
    let tasks = phases.iter().flat_map(|p| p.tasks.iter());
    let mut total = 0;
    let mut done = 0;
    let mut current = None;
    for task in tasks {
        match task.status.as_str() {
            "abandoned" => continue,
            "completed" => done += 1,
            "in_progress" if current.is_none() => current = Some(task.content.clone()),
            _ => {}
        }
        total += 1;
    }
    (total > 0).then_some(TodoSummary { done, total, current })
}

/// One agent: state, timeline, and the locks that serialize process lifecycle and persistence.
pub(crate) struct Agent {
    pub id: String,
    pub dir: PathBuf,
    pub state: Mutex<State>,
    pub timeline: Mutex<Timeline>,
    /// Serializes spawn / hibernate / archive / delete.
    pub lifecycle: tokio::sync::Mutex<()>,
    /// An `agent.updated` emit + `meta.json` write is scheduled.
    pub flush_scheduled: AtomicBool,
    /// Serializes `meta.json` writes against each other and against deletion.
    pub meta_write: Mutex<()>,
    /// A state refresh task is running.
    pub refresh_running: AtomicBool,
    /// Requested refresh kinds (`process::REFRESH_*` bits) not yet picked up by that task.
    pub refresh_pending: AtomicU8,
    /// Kanban board (`board.json`), loaded on first use.
    pub board: Mutex<Option<coocoo_proto::Board>>,
}

impl Agent {
    pub fn new(dir: PathBuf, state: State) -> Self {
        Self {
            id: state.summary.id.clone(),
            timeline: Mutex::new(Timeline::new(dir.join("timeline.jsonl"))),
            dir,
            state: Mutex::new(state),
            lifecycle: tokio::sync::Mutex::new(()),
            flush_scheduled: AtomicBool::new(false),
            meta_write: Mutex::new(()),
            refresh_running: AtomicBool::new(false),
            refresh_pending: AtomicU8::new(0),
            board: Mutex::new(None),
        }
    }

    pub fn summary(&self) -> AgentSummary {
        self.state.lock().summary.clone()
    }

    pub fn client(&self) -> Option<Arc<Client>> {
        self.state.lock().proc.as_ref().map(|p| p.client.clone())
    }
}

#[cfg(test)]
mod tests {
    use coocoo_proto::TodoTask;

    use super::*;

    fn summary(status: AgentStatus) -> AgentSummary {
        AgentSummary {
            id: "a1".into(),
            host_id: "h".into(),
            project_id: None,
            title: "t".into(),
            cwd: "/tmp".into(),
            worktree: None,
            status,
            model: None,
            thinking: None,
            created_at: 0,
            last_activity_at: 0,
            preview: None,
            unread: false,
            context_percent: None,
            cost_usd: None,
            tokens: None,
            pending_requests: 0,
            parent_id: None,
            error: None,
            todos: None,
            session_file: None,
            diff: None,
            subagents: 0,
        }
    }

    fn request(id: &str) -> UiRequest {
        UiRequest {
            id: id.into(),
            agent_id: "a1".into(),
            method: "confirm".into(),
            payload: serde_json::Value::Null,
            created_at: 0,
        }
    }

    #[test]
    fn prompt_lifecycle() {
        let mut st = State::new(summary(AgentStatus::Starting), Life::Starting);
        st.begin_prompt();
        assert_eq!(st.summary.status, AgentStatus::Starting, "still spawning");
        st.life = Life::Live;
        st.sync();
        assert_eq!(st.summary.status, AgentStatus::Running, "a sent prompt counts as running before agent_start");
        st.on_agent_start();
        assert_eq!(st.summary.status, AgentStatus::Running);
        st.add_request(request("ui1"));
        assert_eq!(st.summary.status, AgentStatus::Waiting);
        assert_eq!(st.summary.pending_requests, 1);
        assert!(st.resolve_request("ui1").is_some());
        assert_eq!(st.summary.status, AgentStatus::Running);
        assert!(!st.summary.unread);
        assert!(st.on_prompt_result(PromptStatus::Completed, true, None));
        assert_eq!(st.summary.status, AgentStatus::Idle);
        assert!(st.summary.unread);
        // The trailing session_settled of the same run is not a second turn end.
        assert!(!st.on_settled());
    }

    #[test]
    fn idle_only_after_settled_when_background_work_remains() {
        let mut st = State::new(summary(AgentStatus::Idle), Life::Live);
        st.begin_prompt();
        st.on_agent_start();
        assert!(!st.on_prompt_result(PromptStatus::Completed, false, None));
        assert_eq!(st.summary.status, AgentStatus::Running, "agent_end/prompt_result alone is not idle");
        assert!(st.on_settled());
        assert_eq!(st.summary.status, AgentStatus::Idle);

        // A background job wakes the agent without a prompt.
        st.summary.unread = false;
        st.on_agent_start();
        assert_eq!(st.summary.status, AgentStatus::Running);
        assert!(st.on_settled());
        assert!(st.summary.unread);
    }

    #[test]
    fn failed_turn_then_recovery_and_crash() {
        let mut st = State::new(summary(AgentStatus::Idle), Life::Live);
        st.begin_prompt();
        st.on_agent_start();
        st.on_prompt_result(PromptStatus::Error, true, Some("rate limited".into()));
        assert_eq!(st.summary.status, AgentStatus::Error);
        assert_eq!(st.summary.error.as_deref(), Some("rate limited"));
        st.begin_prompt();
        st.on_agent_start();
        assert_eq!(st.summary.status, AgentStatus::Running);
        assert!(st.summary.error.is_none());

        st.add_request(request("ui9"));
        st.subagents.insert("Sub".into());
        let (_, dropped) = st.take_process(Life::Crashed);
        assert_eq!(dropped.len(), 1);
        assert_eq!(st.summary.status, AgentStatus::Error);
        assert_eq!((st.summary.pending_requests, st.summary.subagents, st.outstanding), (0, 0, 0));
    }

    #[test]
    fn restored_agents_come_back_hibernated() {
        for (before, after) in [
            (AgentStatus::Running, AgentStatus::Hibernated),
            (AgentStatus::Waiting, AgentStatus::Hibernated),
            (AgentStatus::Idle, AgentStatus::Hibernated),
            (AgentStatus::Starting, AgentStatus::Hibernated),
            (AgentStatus::Error, AgentStatus::Error),
            (AgentStatus::Stopped, AgentStatus::Stopped),
        ] {
            let mut s = summary(before);
            s.pending_requests = 2;
            let st = State::from_meta(Meta { summary: s, last_output: None, todos: Vec::new(), context_window: None });
            assert_eq!(st.summary.status, after);
            assert_eq!(st.summary.pending_requests, 0);
        }
    }

    #[test]
    fn todo_summary_counts() {
        let task = |content: &str, status: &str| TodoTask { content: content.into(), status: status.into() };
        let phases = vec![TodoPhase {
            name: "P".into(),
            tasks: vec![task("a", "completed"), task("b", "in_progress"), task("c", "pending"), task("d", "abandoned")],
        }];
        let s = todo_summary(&phases).unwrap();
        assert_eq!((s.done, s.total, s.current.as_deref()), (1, 3, Some("b")));
        assert!(todo_summary(&[]).is_none());
    }
}
