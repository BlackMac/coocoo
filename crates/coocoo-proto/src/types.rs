//! Domain objects. All serialize as camelCase JSON; `ui/src/lib/protocol.ts` mirrors them 1:1.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── Hosts ────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKind {
    /// The daemon the client is connected to.
    Local,
    /// Peer reached over a direct WebSocket URL (LAN, Tailscale, WireGuard…).
    Direct,
    /// Peer reached through an `ssh -L` tunnel spawned by the home daemon.
    Ssh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// Stable daemon id (generated once, persisted in `config.toml`).
    pub id: String,
    /// Display name (defaults to the machine hostname).
    pub name: String,
    pub kind: HostKind,
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `ws://…` URL for direct peers, `user@host` for SSH peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Last connection error for offline peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `omp --version` on that host, if found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcStat {
    /// Agent id, or `browser:<profile>`, or `daemon`.
    pub key: String,
    pub pid: u32,
    /// Memory of the whole process tree in bytes: proportional set size (shared pages split
    /// between processes) on Linux, resident set size elsewhere.
    pub rss: u64,
    /// CPU usage of the whole process tree, percent of one core.
    pub cpu: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStats {
    pub host_id: String,
    pub ts: i64,
    /// Whole-machine CPU usage, 0–100.
    pub cpu: f32,
    pub cpu_count: u32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub load: [f64; 3],
    pub procs: Vec<ProcStat>,
}

/// A pseudo-terminal (`terminal.*`): a login shell, or the run of a project action
/// (`actions.run`). A shell keeps running while nobody is attached and ends when it exits or on
/// `terminal.kill`; an action's run stays listed after it exited (with its exit code and output)
/// until it is restarted or closed with `terminal.close`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub cwd: String,
    /// Shell executable (`$SHELL`, else `/bin/sh`).
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub created_at: i64,
    /// Set for the run of a project action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<TerminalAction>,
    /// When the process ended (Unix ms); only action runs are kept after that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exited_at: Option<i64>,
    /// Exit status once ended; `None` after a signal (stopped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Which action a terminal runs; `base` is the directory it was started for (project path or
/// worktree), before the action's own `cwd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAction {
    pub project_id: String,
    pub action_id: String,
    pub name: String,
    pub base: String,
}

// ─── Project actions (`.coocoo/actions.toml`) ───────────────────────────────────────────────

/// One entry of a project's `.coocoo/actions.toml`, validated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAction {
    /// Unique within the project (explicit `id`, else derived from the name).
    pub id: String,
    pub name: String,
    /// One of the icon names the UI ships; `None` = the kind's default icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub scope: ActionScope,
    /// The action the Run button starts with one click.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    /// Question shown in a confirmation dialog before it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(flatten)]
    pub kind: ActionKind,
}

/// Where an action is offered: in the project view (runs in the project directory, a prompt
/// starts a new agent), in agent views (runs in the agent's directory, a prompt goes to that
/// agent), or both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionScope {
    Project,
    Agent,
    #[default]
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActionKind {
    /// A shell command, run in its own terminal (`actions.run`).
    #[serde(rename_all = "camelCase")]
    Run {
        command: String,
        /// Relative to the project or worktree directory; never leaves it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// Text sent to the agent, or the first prompt of a new agent in the project view.
    #[serde(rename_all = "camelCase")]
    Prompt {
        prompt: String,
        /// Project view: start the new agent in its own worktree.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        worktree: bool,
    },
    #[serde(rename_all = "camelCase")]
    Url {
        url: String,
        /// `None` = the project's browser for loopback hosts, else the system browser.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<UrlTarget>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UrlTarget {
    /// The project's Chromium (Browser panel).
    Browser,
    /// The system browser of the client.
    External,
}

// ─── Projects / git ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub host_id: String,
    pub name: String,
    /// Absolute path on `host_id`.
    pub path: String,
    pub added_at: i64,
    /// `None` when the directory is not a git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
    /// URLs the user pinned for this project (dev server etc.), shown as quick tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dev_urls: Vec<String>,
    /// Icon file found in the project (favicon, app icon, logo); fetch it with `projects.icon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProjectIcon>,
    /// Actions from `.coocoo/actions.toml` in `path`, in file order (runtime state, like `git`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ProjectAction>,
    /// Problems in that file (`actions.toml:LINE:COL – …`); valid actions stay usable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIcon {
    /// Path relative to the project directory.
    pub path: String,
    /// Modification time (ms since epoch), so clients know when to refetch.
    pub modified: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: Option<String>,
    /// Short HEAD sha.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Number of changed (incl. untracked) files in the main worktree.
    pub changed: u32,
    pub last_commit: Option<CommitInfo>,
    pub worktrees: Vec<Worktree>,
    /// Newest `FETCH_HEAD` modification (Unix ms) over all worktrees: how current the
    /// upstream counts are. `None` when the repository was never fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<i64>,
    /// Default remote for pushing a branch without upstream: `origin`, else the only remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Present when that remote points to github.com.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubInfo>,
}

/// The GitHub repository behind the project's default remote, and what `gh` can do with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubInfo {
    /// `owner/name`.
    pub repo: String,
    /// Remote name, e.g. `origin`.
    pub remote: String,
    pub gh: GhState,
    /// Merge methods the repository allows; empty until known.
    #[serde(default)]
    pub merge_methods: Vec<MergeMethod>,
    #[serde(default)]
    pub delete_branch_on_merge: bool,
}

/// Whether the `gh` CLI can be used on this host; `Unknown` until the first pull request call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GhState {
    #[default]
    Unknown,
    Ready,
    Missing,
    LoggedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

impl MergeMethod {
    /// The `gh pr merge` flag.
    pub fn flag(self) -> &'static str {
        match self {
            MergeMethod::Squash => "--squash",
            MergeMethod::Merge => "--merge",
            MergeMethod::Rebase => "--rebase",
        }
    }
}

/// A branch's pull request on GitHub, as of `fetched_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestInfo {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub draft: bool,
    /// Base branch name.
    pub base: String,
    pub checks: PrChecks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<PrReview>,
    /// GitHub's `mergeStateStatus` verbatim (`CLEAN`, `HAS_HOOKS`, `UNSTABLE`, `BLOCKED`,
    /// `BEHIND`, `DIRTY`, `DRAFT`, `UNKNOWN`); mergeable when `CLEAN` or `HAS_HOOKS`.
    pub merge_state: String,
    /// Unix ms.
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrReview {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrChecks {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    /// The failed checks, with their details page.
    #[serde(default)]
    pub failing: Vec<PrCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A pull request as remembered on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRef {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub summary: String,
    pub author: String,
    /// Unix ms.
    pub ts: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    /// Changed files; `None` when not computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<u32>,
    /// Upstream of the checked-out branch, with the ahead/behind counts against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// Linked worktrees on a branch: commits ahead of / behind the main worktree's branch
    /// (the target of `git.merge`). `None` for the main worktree and detached heads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ahead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_behind: Option<u32>,
    /// Unmerged (conflicted) paths.
    #[serde(default)]
    pub conflicts: u32,
    /// Operation in progress in this worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<GitOperation>,
    /// The branch's pull request (an open one before closed or merged ones), from the daemon's
    /// cache: refreshed after push, PR actions, fetch and `git.prStatus`, never in the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestInfo>,
    /// When the branch's pull request was last looked up (Unix ms), also when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_checked_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// `M`, `A`, `D`, `R`, `?` (untracked).
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    /// Unified diff hunks (starting at the first `@@`); empty for binary files.
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub cwd: String,
    /// What the diff is against: `HEAD`, or the merge-base sha with the worktree's base branch.
    pub base: String,
    pub files: Vec<DiffFile>,
    /// True when patches were cut to stay under the size budget.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Directory contains `.git`.
    pub is_git: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStat {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

// ─── Agents ───────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    /// omp process spawning / resuming.
    Starting,
    /// A turn is in progress.
    Running,
    /// Settled, process alive, ready for a prompt.
    Idle,
    /// Blocked on a `UiRequest` (needs the user).
    Waiting,
    /// Last turn failed or the process crashed.
    Error,
    /// Process stopped to save resources; resumes transparently on the next prompt.
    Hibernated,
    /// Archived by the user; read-only.
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
    /// Branch the worktree was created from (diffs are shown against its merge-base).
    pub base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSummary {
    pub done: u32,
    pub total: u32,
    /// Content of the in-progress task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
}

/// The agent's own short summary of the task it works on (host tool `coocoo_task_summary`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    /// One or two sentences, single line.
    pub text: String,
    /// When the user message it belongs to was sent (Unix ms): the newest one at the time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub id: String,
    pub host_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub title: String,
    /// Working directory of the omp process (project path or worktree path).
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeRef>,
    pub status: AgentStatus,
    /// `provider/modelId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    pub created_at: i64,
    pub last_activity_at: i64,
    /// Last assistant text, ≤ 240 chars, single line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// A turn finished since a client last opened this agent (`agents.markRead`).
    pub unread: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    pub pending_requests: u32,
    /// Agent that spawned this one through the orchestration tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todos: Option<TodoSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskSummary>,
    /// omp session file; used to resume after hibernation or daemon restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// Uncommitted + branch changes in `cwd` (vs worktree base), refreshed after each turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffStat>,
    /// omp subagents (task tool) currently running inside this agent.
    #[serde(default)]
    pub subagents: u32,
}

/// Image attached to a prompt (`agents.prompt.images`), forwarded to omp as `ImageContent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    pub mime_type: String,
    /// Base64 without the `data:` prefix.
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    User,
    Assistant,
    Thinking,
    Tool,
    /// Informational (compaction, retry, model change, hibernation…).
    Notice,
    Error,
    /// Output of a slash command omp ran itself (`/context`, `/todo`…), plain text.
    Command,
}

/// One entry of omp's slash-command catalog (`agents.commands`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// Without the leading `/`.
    pub name: String,
    /// `builtin`, `skill`, `extension`, `custom`, `mcp_prompt` or `file`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Argument hint, e.g. `[on|off|status]` or `<path>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<SlashSubcommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashSubcommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

/// Payload of `agent.commands`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandsEvent {
    pub agent_id: String,
    pub commands: Vec<SlashCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    /// The `i`/intent argument omp tools carry, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// JSON-encoded arguments, truncated to 4 KiB.
    pub args: String,
    pub status: ToolStatus,
    /// Result text, truncated to 8 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Subagents this call (omp's `task` tool) started, in start order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<Subagent>,
}

/// One omp subagent started by a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    /// omp's subagent id (also its name, e.g. `CoreSlice`).
    pub id: String,
    /// Agent type (`task`, `scout`, …).
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `running`, `completed`, `failed` or `aborted`.
    pub status: String,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// What it is doing right now (current tool and its intent); only while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// `provider/model` it runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Its omp session file: read the transcript with `agents.subagentTimeline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    /// Monotonic per agent; ordering key and pagination cursor.
    pub seq: u64,
    /// Stable id; later upserts with the same id replace the item (keeping the first `seq`).
    pub id: String,
    pub ts: i64,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCall>,
    /// Item still streaming (assistant/thinking text or running tool).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// Images of a user message, as the agent got them (fetch with `agents.image`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<TimelineImage>,
}

/// One image of a user message, stored under the agent's data directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineImage {
    /// File name (`<seq>-<index>.<ext>`), the key for `agents.image`.
    pub name: String,
    pub mime: String,
    /// Bytes.
    pub size: u64,
    /// Pixel size from the image header; absent when it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Payload of `agent.timeline`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub agent_id: String,
    pub item: TimelineItem,
}

/// Payload of `agent.delta`: append `text` to item `item_id`'s text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDelta {
    pub agent_id: String,
    pub item_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePage {
    /// Oldest first.
    pub items: Vec<TimelineItem>,
    /// More items exist before `items[0].seq`.
    pub has_more: bool,
}

/// A dialog awaiting the user: an omp extension UI request, or a secret an agent asked for.
///
/// `method` is one of `select`, `confirm`, `input`, `editor`, `ask` (then `payload` is the omp
/// `extension_ui_request` frame verbatim) or `secret` (a [`SecretUiPayload`], answered with
/// `secrets.provide`). The client answers omp dialogs with `agents.respond`, whose `response`
/// is the matching omp reply body without `type`/`id`:
/// `{ "value": String }` (select/input/editor), `{ "confirmed": bool }` (confirm),
/// `{ "answers": [{ "id", "selectedOptions": [String], "customInput"? }] }` (ask),
/// or `{ "cancelled": true }` (any, also declines a `secret`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiRequest {
    pub id: String,
    pub agent_id: String,
    pub method: String,
    pub payload: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoTask {
    pub content: String,
    /// `pending`, `in_progress`, `completed`, `abandoned`, `blocked`.
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoPhase {
    pub name: String,
    pub tasks: Vec<TodoTask>,
}

/// Full agent state for the agent view (`agents.get`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetail {
    pub summary: AgentSummary,
    pub todos: Vec<TodoPhase>,
    pub queued_steering: Vec<String>,
    pub queued_follow_up: Vec<String>,
    pub requests: Vec<UiRequest>,
    /// Context window size in tokens, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

/// A project's Kanban board (`boards.*`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    /// Every card; a column's order is the order of its cards here (first = top).
    pub cards: Vec<Card>,
    /// External sources whose new entries land in the inbox.
    #[serde(default)]
    pub sources: Vec<BoardSource>,
    /// What `kanban_done` does after the commit.
    #[serde(default)]
    pub done_action: DoneAction,
}

/// What finishing a card (`kanban_done`) does besides moving it to done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DoneAction {
    /// Commit the agent's directory.
    #[default]
    Commit,
    /// Commit, then push the branch.
    Push,
    /// Commit, push and open a pull request (worktree agents; others only push).
    PullRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CardColumn {
    /// From external sources; the user picks what moves on to `todo`.
    Inbox,
    Todo,
    Doing,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub column: CardColumn,
    /// Link to the source entry (feed item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// `BoardSource.id` the card came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// When the card last entered done (cleared when it moves back). Cards finished before this
    /// field existed have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<i64>,
    /// The agent's closing note (`kanban_done`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Commit made when the agent finished the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitInfo>,
    /// Agent the card was handed to (`boards.cardStart`/`boards.cardRefine`) or that finished it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// That agent's title at the time, for when it has been deleted since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// Webhook POSTed when work on the card starts (it enters doing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_url: Option<String>,
    /// Webhook POSTed when the card is finished (it enters done).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_url: Option<String>,
    /// Why the last webhook call failed; cleared by the next successful one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_error: Option<String>,
    /// The agent that put the card on the board (`kanban_add`), e.g. a suggestion in the inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<AgentRef>,
    /// Context images from the user (`boards.cardImage`); the agent gets their paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Attachment>,
    /// Its agent is working out a refinement (`boards.cardRefine`) instead of implementing it;
    /// cleared by the refinement, or when the agent's run ends without one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refining: bool,
    /// The latest refinement: the plan agreed before the work starts, handed to the agent that
    /// implements the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refinement: Option<Refinement>,
    /// Pull request opened when the card was finished (`DoneAction::PullRequest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestRef>,
    /// Why the push or pull request after finishing the card failed; cleared by a successful
    /// `boards.cardPublish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_error: Option<String>,
}

/// An agent as remembered on a card: its title at the time, for when it has been deleted since.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refinement {
    /// Markdown.
    pub text: String,
    /// UI mocks and other files (images, HTML); fetch with `boards.attachment`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// File name, unique within the refinement.
    pub name: String,
    pub mime: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSource {
    pub id: String,
    /// `feed`: RSS/Atom URL; `command`: shell command run in the project directory; `script`: a
    /// small program an agent wrote, run sandboxed in its own state directory. Commands and
    /// scripts print one item per line (plain title or a JSON object).
    pub kind: String,
    /// Feed URL or command (empty for scripts).
    #[serde(default)]
    pub target: String,
    /// Label, e.g. "Svelte changelog" (required for scripts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Source code of a `script` importer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    pub interval_mins: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether the last run of a script was sandboxed (Landlock on Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandboxed: Option<bool>,
    /// The agent that created or last changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// Entry keys already imported (feed guid/link, line or JSON `id`), newest last; deleting a
    /// card does not bring it back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<String>,
}

/// Payload of `project.board`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardEvent {
    pub project_id: String,
    pub board: Board,
}

/// A recurring or one-off agent run of a project (`schedules.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub project_id: String,
    /// Label, also the title of the agents it creates.
    pub name: String,
    /// The message the agent gets on every run.
    pub prompt: String,
    /// Recurring: a 5-field cron expression (or `@daily` etc.) read in `timezone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// One-off run time (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// IANA time zone the cron expression is read in.
    pub timezone: String,
    /// Existing agent of the project prompted on every run; `None`: a new agent per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// That agent's title when the schedule was saved, for when it has been deleted since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// New agents get their own git worktree.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worktree: bool,
    /// Model (`provider/id`) of new agents; `None`: the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub enabled: bool,
    /// When it runs next; `None` while disabled or once a one-off run is over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<ScheduleRun>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The agent that created it (`coocoo_schedule_save`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<AgentRef>,
}

/// What the latest run of a [`Schedule`] did.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRun {
    pub at: i64,
    pub outcome: RunOutcome,
    /// The agent that got (or would have got) the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// Why it was skipped or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Started with `schedules.run` instead of by the clock.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunOutcome {
    /// The agent got the prompt.
    Started,
    /// The target agent was busy; nothing was sent.
    Skipped,
    /// Creating or prompting the agent failed.
    Failed,
}

/// Payload of `project.schedules`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulesEvent {
    pub project_id: String,
    pub schedules: Vec<Schedule>,
}

/// Format of a file Coocoo writes a secret into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretFormat {
    /// One `NAME=value` line in a dotenv file; other lines stay as they are.
    Dotenv,
    /// The value is the whole file.
    Raw,
}

/// A file Coocoo wrote a secret into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFile {
    /// Absolute path on the agent's host.
    pub path: String,
    /// Path relative to the directory it was written in (an agent's directory or the project).
    pub rel_path: String,
    pub format: SecretFormat,
    /// Git branch of that directory when it was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// A project's secret without its value (`secrets.*`, `project.secrets`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInfo {
    /// Env var name every agent of the project gets.
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<SecretFile>,
    /// The agent that asked for it (`coocoo_secret_request`); `None`: added by the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<AgentRef>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Payload of `project.secrets`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsEvent {
    pub project_id: String,
    pub secrets: Vec<SecretInfo>,
}

/// `payload` of a [`UiRequest`] with method `secret`: an agent asks for a secret. Never holds
/// a value; the client answers with `secrets.provide`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretUiPayload {
    /// Always `secret`.
    pub method: String,
    pub name: String,
    /// Why the agent needs it.
    pub description: String,
    pub project_id: String,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiline: bool,
    /// A secret of this name exists; saving replaces it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<SecretFileTarget>,
    /// Branch of the agent's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The agent works in its own worktree.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worktree: bool,
}

/// The file a requested secret is also written into.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFileTarget {
    /// Relative to the agent's directory.
    pub path: String,
    pub format: SecretFormat,
    /// Already ignored by git.
    pub git_ignored: bool,
    /// Not ignored: Coocoo adds it to `.git/info/exclude`.
    pub exclude: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub provider: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub reasoning: bool,
}

/// `agents.usage`: one provider's subscription quota on a host, normalized from
/// `omp usage --json` (every account of the provider that omp is logged in with).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub provider: String,
    /// When the quota was read (ms since epoch).
    pub fetched_at: i64,
    /// Quota per window and pool summed over the accounts, shortest window first.
    pub windows: Vec<UsageCapacity>,
    pub accounts: Vec<UsageAccount>,
}

/// Quota left in one window of one pool across all accounts of a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCapacity {
    /// `5h`, `7d`, …; windows without a duration use the provider's label (`extra`).
    pub window: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Name of a separate pool that only some models draw from (an Anthropic model tier, Codex
    /// Spark, an Antigravity counter such as `Gemini`); absent for the provider-wide pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// omp's ids of the quota counters this pool covers (`fable`, `spark`, `anthropic`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pools: Vec<String>,
    /// The requested model draws from this window (always true for provider-wide pools).
    pub gating: bool,
    /// Accounts reporting a limit in this window.
    pub accounts: u32,
    /// Accounts' worth of quota used: each account adds its binding limit's used fraction.
    pub used: f64,
    /// Earliest reset among the window's limits (ms since epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAccount {
    /// Email, organization or account id, whichever the provider reports.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub limits: Vec<UsageLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimit {
    pub id: String,
    pub label: String,
    /// Window label (`5 Hour`, `7 Day`); absent for quotas without a window (extra usage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// `percent`, `usd`, `tokens`, `requests`, `credits`, `minutes`, `bytes` or `unknown`.
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    /// 0–1; above 1 means overage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_fraction: Option<f64>,
    /// `ok`, `warning`, `exhausted` or `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

// ─── Browser ──────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub target_id: String,
    pub url: String,
    pub title: String,
    /// Tab most recently created/navigated/focused (agent or user).
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileInfo {
    /// Profile id (= project id; `default` for agents without a project).
    pub id: String,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub tabs: Vec<BrowserTab>,
    /// Loopback CDP discovery root handed to omp (`browser.cdpUrl`), e.g. `http://127.0.0.1:41234`
    /// (bound by `browser.cdpEndpoint`); empty while no listener is bound for this profile.
    pub cdp_url: String,
}

/// Payload of `browser.frame`. `data` is the base64 JPEG straight from CDP (no re-encoding).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFrame {
    pub profile: String,
    pub target_id: String,
    pub data: String,
    /// CSS-pixel size of the page viewport the frame represents.
    pub width: u32,
    pub height: u32,
    /// Monotonic frame counter per watch.
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConsoleEntry {
    pub profile: String,
    pub target_id: String,
    /// `log`, `info`, `warning`, `error`, `debug`, `exception`, `network`.
    pub level: String,
    pub text: String,
    pub ts: i64,
}
