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
    /// Commands the user pinned for this project (dev server etc.), shown as quick actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dev_urls: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    /// Changed files; `None` when not computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<u32>,
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

/// An omp extension UI request awaiting the user.
///
/// `payload` is the omp `extension_ui_request` frame verbatim; `method` is one of
/// `select`, `confirm`, `input`, `editor`, `ask`. The client answers with `agents.respond`,
/// whose `response` is the matching omp reply body without `type`/`id`:
/// `{ "value": String }` (select/input/editor), `{ "confirmed": bool }` (confirm),
/// `{ "answers": [{ "id", "selectedOptions": [String], "customInput"? }] }` (ask),
/// or `{ "cancelled": true }` (any).
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
    /// Quota per window summed over the accounts, shortest window first (omp's `capacity`).
    pub windows: Vec<UsageCapacity>,
    pub accounts: Vec<UsageAccount>,
}

/// Quota left in one window across all accounts of a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCapacity {
    /// `5h`, `7d`, …; windows without a duration use the provider's label (`extra`).
    pub window: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Separate pool inside the window that only some models draw from (e.g. a model tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<String>,
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
