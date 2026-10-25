// TS mirror of crates/coocoo-proto/src/{types,wire}.rs and the method table in docs/PROTOCOL.md.
// All JSON is camelCase; Rust `Option` fields are optional properties here.

// ─── Envelopes ────────────────────────────────────────────────────────────────────────────────

export interface RequestFrame {
  id: number;
  method: string;
  params?: unknown;
  host?: string;
}

export interface ApiError {
  code: ApiErrorCode | string;
  message: string;
}

export type ApiErrorCode =
  | 'invalid_params'
  | 'not_found'
  | 'unknown_method'
  | 'conflict'
  | 'unauthorized'
  | 'unavailable'
  | 'forbidden'
  | 'internal';

export interface ResponseFrame {
  id: number;
  result?: unknown;
  error?: ApiError;
}

export interface EventFrame {
  event: string;
  host: string;
  data: unknown;
}

export type ServerFrame = ResponseFrame | EventFrame;

// ─── Hosts ────────────────────────────────────────────────────────────────────────────────────

export type HostKind = 'local' | 'direct' | 'ssh';

export interface HostInfo {
  id: string;
  name: string;
  kind: HostKind;
  online: boolean;
  os?: string;
  arch?: string;
  version?: string;
  /** `ws://…` for direct peers, `user@host` for SSH peers. */
  address?: string;
  error?: string;
  ompVersion?: string;
}

export interface ProcStat {
  /** Agent id, `browser:<profile>`, or `daemon`. */
  key: string;
  pid: number;
  rss: number;
  cpu: number;
}

export interface HostStats {
  hostId: string;
  ts: number;
  cpu: number;
  cpuCount: number;
  memUsed: number;
  memTotal: number;
  load: [number, number, number];
  procs: ProcStat[];
}

/**
 * A pseudo-terminal: a login shell (keeps running while nobody is attached), or the run of a
 * project action, which stays listed after it ended until restarted or closed.
 */
export interface TerminalInfo {
  id: string;
  cwd: string;
  shell: string;
  cols: number;
  rows: number;
  createdAt: number;
  action?: TerminalAction;
  /** When the process ended (ms); only action runs are kept after that. */
  exitedAt?: number;
  /** Exit status; missing after a signal (stopped). */
  exitCode?: number;
}

export interface TerminalAction {
  projectId: string;
  actionId: string;
  name: string;
  /** Directory it was started for (project path or agent cwd), before the action's own `cwd`. */
  base: string;
}

// ─── Projects / git ───────────────────────────────────────────────────────────────────────────

export type ActionScope = 'project' | 'agent' | 'both';

/** One entry of `.coocoo/actions.toml`. */
export type ProjectAction = {
  id: string;
  name: string;
  icon?: string;
  scope: ActionScope;
  default?: boolean;
  confirm?: string;
} & (
  | { kind: 'run'; command: string; cwd?: string }
  | { kind: 'prompt'; prompt: string; worktree?: boolean }
  | { kind: 'url'; url: string; target?: 'browser' | 'external' }
);

export interface Project {
  id: string;
  hostId: string;
  name: string;
  path: string;
  addedAt: number;
  git?: GitInfo;
  devUrls?: string[];
  /** Favicon/app icon/logo found in the project; fetch with `projects.icon`. */
  icon?: { path: string; modified: number };
  /** From `.coocoo/actions.toml`, in file order. */
  actions?: ProjectAction[];
  /** Problems in that file (`actions.toml:LINE:COL – …`). */
  actionErrors?: string[];
}

export interface GitInfo {
  branch?: string | null;
  head?: string | null;
  upstream?: string | null;
  ahead: number;
  behind: number;
  changed: number;
  lastCommit?: CommitInfo | null;
  worktrees: Worktree[];
  /** Newest `FETCH_HEAD` mtime (ms) over all worktrees: how current `ahead`/`behind` are. */
  fetchedAt?: number;
  /** Default remote for pushes of branches without upstream: `origin`, else the only remote. Absent when the repo has no (unambiguous) remote. */
  remote?: string;
  /** Present when that default remote's URL points to github.com. */
  github?: GithubInfo;
}

export interface GithubInfo {
  /** `owner/name`. */
  repo: string;
  /** Remote name, e.g. `origin`. */
  remote: string;
  /** `unknown` until the first PR call (`git.prStatus` etc.) on this daemon. */
  gh: 'unknown' | 'ready' | 'missing' | 'loggedOut';
  /** Allowed by the repo; `[]` until known. */
  mergeMethods: MergeMethod[];
  deleteBranchOnMerge: boolean;
}

export type MergeMethod = 'squash' | 'merge' | 'rebase';

export interface PullRequestInfo {
  number: number;
  url: string;
  title: string;
  state: 'open' | 'closed' | 'merged';
  draft: boolean;
  /** Base branch name. */
  base: string;
  checks: PrChecks;
  review?: 'approved' | 'changesRequested' | 'reviewRequired';
  /** GitHub mergeStateStatus verbatim: CLEAN, HAS_HOOKS, UNSTABLE, BLOCKED, BEHIND, DIRTY, DRAFT, UNKNOWN. Mergeable only when CLEAN or HAS_HOOKS. */
  mergeState: string;
  /** Unix ms. */
  fetchedAt: number;
}

export interface PrChecks {
  total: number;
  passed: number;
  failed: number;
  pending: number;
  failing: PrCheck[];
}

export interface PrCheck {
  name: string;
  url?: string;
}

export interface CommitInfo {
  sha: string;
  summary: string;
  author: string;
  ts: number;
}

export interface Worktree {
  path: string;
  branch?: string | null;
  head?: string | null;
  isMain: boolean;
  changed?: number;
  /** Upstream of the checked-out branch, with counts against it. */
  upstream?: string;
  ahead: number;
  behind: number;
  /** Linked worktrees on a branch: counts against the main worktree's branch. */
  baseAhead?: number;
  baseBehind?: number;
  /** Unmerged paths. */
  conflicts: number;
  operation?: GitOperation;
  /** The branch's pull request (open one preferred over closed/merged), from the daemon's cache. */
  pullRequest?: PullRequestInfo;
  /** When the PR status of this branch was last fetched (Unix ms); absent = never. Also set when no PR exists. */
  prCheckedAt?: number;
}

export type GitOperation = 'merge' | 'rebase' | 'cherryPick' | 'revert' | 'bisect';

export type DiffFileStatus = 'M' | 'A' | 'D' | 'R' | '?';

export interface DiffFile {
  path: string;
  oldPath?: string;
  status: DiffFileStatus | string;
  additions: number;
  deletions: number;
  binary: boolean;
  patch: string;
}

export interface DiffResult {
  cwd: string;
  base: string;
  files: DiffFile[];
  truncated: boolean;
}

export interface FsEntry {
  name: string;
  path: string;
  isDir: boolean;
  isGit: boolean;
}

export interface DiffStat {
  files: number;
  additions: number;
  deletions: number;
}

// ─── Agents ───────────────────────────────────────────────────────────────────────────────────

export type AgentStatus = 'starting' | 'running' | 'idle' | 'waiting' | 'error' | 'hibernated' | 'stopped';

export interface WorktreeRef {
  path: string;
  branch: string;
  base: string;
}

export interface TodoSummary {
  done: number;
  total: number;
  current?: string;
}

export interface TaskSummary {
  /** One or two sentences, single line. */
  text: string;
  /** When the user message it belongs to was sent (Unix ms). */
  since?: number;
}

export interface AgentSummary {
  id: string;
  hostId: string;
  projectId?: string;
  title: string;
  cwd: string;
  worktree?: WorktreeRef;
  status: AgentStatus;
  model?: string;
  thinking?: string;
  createdAt: number;
  lastActivityAt: number;
  preview?: string;
  unread: boolean;
  contextPercent?: number;
  costUsd?: number;
  tokens?: number;
  pendingRequests: number;
  parentId?: string;
  error?: string;
  todos?: TodoSummary;
  /** The agent's own summary of its current task (host tool `coocoo_task_summary`). */
  task?: TaskSummary;
  sessionFile?: string;
  diff?: DiffStat;
  subagents: number;
}

/** An image sent with a card (`boards.cardAdd`/`cardUpdate`): PNG, JPEG, GIF or WebP, ≤ 5 MiB. */
export interface ImageUpload extends ImageInput {
  name?: string;
}

export interface ImageInput {
  mimeType: string;
  /** Base64 without the `data:` prefix. */
  data: string;
}

/** `command`: output of a slash command omp ran itself (plain text). */
export type ItemKind = 'user' | 'assistant' | 'thinking' | 'tool' | 'notice' | 'error' | 'command';

/** One entry of omp's slash-command catalog. */
export interface SlashCommand {
  /** Without the leading `/`. */
  name: string;
  source: 'builtin' | 'skill' | 'extension' | 'custom' | 'mcp_prompt' | 'file';
  aliases?: string[];
  description?: string;
  /** Argument hint, e.g. `[on|off|status]`. */
  hint?: string;
  subcommands?: { name: string; description?: string; usage?: string }[];
}

export type ToolStatus = 'running' | 'ok' | 'error';

export interface ToolCall {
  name: string;
  intent?: string;
  /** JSON-encoded arguments, truncated to 4 KiB. */
  args: string;
  status: ToolStatus;
  output?: string;
  durationMs?: number;
  /** Subagents this call (omp's `task` tool) started. */
  subagents?: Subagent[];
}

export interface Subagent {
  id: string;
  /** Agent type (`task`, `scout`, …). */
  agent: string;
  description?: string;
  status: 'running' | 'completed' | 'failed' | 'aborted';
  startedAt: number;
  endedAt?: number;
  /** Current tool and intent, while running. */
  activity?: string;
  toolCount?: number;
  costUsd?: number;
  model?: string;
  sessionFile?: string;
}

export interface TimelineItem {
  seq: number;
  id: string;
  ts: number;
  kind: ItemKind;
  text?: string;
  tool?: ToolCall;
  partial?: boolean;
  /** Images of a user message, as the agent got them (fetch with `agents.image`). */
  images?: TimelineImage[];
}

/** One image of a user message, stored with the agent. */
export interface TimelineImage {
  /** `<seq>-<index>.<ext>`, the key for `agents.image`. */
  name: string;
  mime: string;
  size: number;
  /** Pixel size from the image header; absent when it could not be read. */
  width?: number;
  height?: number;
}

export interface TimelineEvent {
  agentId: string;
  item: TimelineItem;
}

export interface TimelineDelta {
  agentId: string;
  itemId: string;
  text: string;
}

export interface TimelinePage {
  items: TimelineItem[];
  hasMore: boolean;
}

export type UiRequestMethod = 'select' | 'confirm' | 'input' | 'editor' | 'ask' | 'secret';

export interface UiRequest {
  id: string;
  agentId: string;
  method: UiRequestMethod | string;
  /** The omp `extension_ui_request` frame verbatim. */
  payload: UiRequestPayload;
  createdAt: number;
}

export interface TodoTask {
  content: string;
  status: 'pending' | 'in_progress' | 'completed' | 'abandoned' | 'blocked' | string;
}

export interface TodoPhase {
  name: string;
  tasks: TodoTask[];
}

/** A project's Kanban board: a column's order is the order of its cards (first = top). */
export interface Board {
  cards: Card[];
  sources: BoardSource[];
  /** What `kanban_done` does beyond committing. */
  doneAction: DoneAction;
}

export type DoneAction = 'commit' | 'push' | 'pullRequest';

export type CardColumn = 'inbox' | 'todo' | 'doing' | 'done';

export interface Card {
  id: string;
  title: string;
  body?: string;
  column: CardColumn;
  url?: string;
  /** `BoardSource.id` it came from. */
  source?: string;
  createdAt: number;
  updatedAt: number;
  /** When it last entered done (cleared when moved back); absent on cards finished before Coocoo recorded it. */
  doneAt?: number;
  /** The agent's closing note. */
  note?: string;
  commit?: CommitInfo;
  /** Agent the card was handed to, or that finished it. */
  agentId?: string;
  /** That agent's title at the time (it may have been deleted since). */
  agentTitle?: string;
  /** Webhooks POSTed when the card enters doing / done. */
  startUrl?: string;
  doneUrl?: string;
  /** Why the last webhook call failed. */
  hookError?: string;
  /** Its agent is working out a refinement (`boards.cardRefine`). */
  refining?: boolean;
  /** The agent that put the card on the board (`kanban_add`), e.g. a suggestion in the inbox. */
  addedBy?: { id: string; title: string };
  /** Context images from the user (`boards.cardImage`); the agent gets their paths. */
  images?: Attachment[];
  /** The latest refinement, handed to the agent that implements the card. */
  refinement?: Refinement;
  /** The pull request opened for it when it was done. */
  pullRequest?: { number: number; url: string };
  /** Why push / PR creation after `kanban_done` failed; cleared by a successful `boards.cardPublish`. */
  gitError?: string;
}

export interface Refinement {
  /** Markdown. */
  text: string;
  /** UI mocks (images, HTML); fetch with `boards.attachment`. */
  attachments?: Attachment[];
  agentId?: string;
  agentTitle?: string;
  at: number;
}

export interface Attachment {
  name: string;
  mime: string;
  size: number;
}

export type SourceKind = 'feed' | 'command' | 'script';

/** Schedules an importer can run on, in minutes. */
export const IMPORT_INTERVALS = [15, 60, 360, 1440] as const;

export interface BoardSource {
  id: string;
  /** `feed` (RSS/Atom URL), `command` (run in the project, one card per output line) or
   *  `script` (a program an agent wrote, run sandboxed in its own directory). */
  kind: SourceKind;
  /** Feed URL or command; empty for scripts. */
  target: string;
  name?: string;
  script?: string;
  intervalMins: number;
  checkedAt?: number;
  error?: string;
  /** Whether a script's last run was sandboxed. */
  sandboxed?: boolean;
  /** Agent that created or last changed it. */
  agentId?: string;
  agentTitle?: string;
}

/** Result of adding, changing or running a source. */
export interface SourceRun {
  source: BoardSource;
  imported: number;
}

/** Outcome of one run of a `Schedule`. */
export interface ScheduleRun {
  /** When it ran (Unix ms). */
  at: number;
  /** started: the agent got the prompt; skipped: the target agent was busy (or archived);
   *  failed: creating/prompting failed (`message` says why). */
  outcome: 'started' | 'skipped' | 'failed';
  /** The agent that got (or would have got) the prompt. */
  agentId?: string;
  agentTitle?: string;
  /** Skip reason or error. */
  message?: string;
  /** Started with `schedules.run` ("Run now") instead of by the clock. */
  manual?: boolean;
}

/** A scheduled agent run: on a cron schedule or once, a new agent or an existing one gets `prompt`. */
export interface Schedule {
  id: string;
  projectId: string;
  /** Label, also the title of the agents it creates. */
  name: string;
  /** The message the agent gets on every run (Markdown/plain text). */
  prompt: string;
  /** Recurring: 5-field cron `minute hour day-of-month month day-of-week` (lists `,`, ranges `-`,
   *  steps `/`, `*`, month names jan–dec, weekday names sun–sat, 0 and 7 = Sunday; day-of-month and
   *  day-of-week both restricted = either matches, as in Vixie cron) or one of `@hourly`, `@daily`,
   *  `@weekly`, `@monthly`, `@yearly`. Read in `timezone`. Absent for a one-off run. */
  cron?: string;
  /** One-off run time (Unix ms); absent for cron schedules. */
  at?: number;
  /** IANA time zone the cron expression is read in, e.g. "Europe/Berlin". */
  timezone: string;
  /** Prompt this existing agent of the project on every run; absent = create a new agent per run. */
  agentId?: string;
  /** That agent's title when the schedule was saved (for when it is deleted). */
  agentTitle?: string;
  /** New agents get their own git worktree (only without agentId). Omitted when false. */
  worktree?: boolean;
  /** Model (`provider/id`) for new agents; absent = default. */
  model?: string;
  enabled: boolean;
  /** Next time it runs (Unix ms); absent while disabled or when a one-off run is over. */
  nextRunAt?: number;
  lastRun?: ScheduleRun;
  createdAt: number;
  updatedAt: number;
  /** The agent that created it via its `coocoo_schedule_save` tool. */
  addedBy?: { id: string; title: string };
}

export interface AgentDetail {
  summary: AgentSummary;
  todos: TodoPhase[];
  queuedSteering: string[];
  queuedFollowUp: string[];
  requests: UiRequest[];
  contextWindow?: number;
}

export interface ModelInfo {
  provider: string;
  id: string;
  name?: string;
  contextWindow?: number;
  reasoning: boolean;
}

/** `agents.usage`: one provider's subscription quota on a host (normalized `omp usage --json`). */
export interface ProviderUsage {
  provider: string;
  fetchedAt: number;
  /** Quota per window summed over the accounts, shortest window first. */
  windows: UsageCapacity[];
  accounts: UsageAccount[];
}

export interface UsageCapacity {
  /** `5h`, `7d`, …; windows without a duration use the provider's label (`extra`). */
  window: string;
  durationMs?: number;
  /** Separate pool only some models draw from (Anthropic tier, Codex Spark, Antigravity counter such as `Gemini`). */
  name?: string;
  /** omp's ids of the quota counters this pool covers. */
  pools?: string[];
  /** The requested model draws from this window. */
  gating: boolean;
  accounts: number;
  /** Accounts' worth of quota used (0…accounts). */
  used: number;
  resetsAt?: number;
}

export interface UsageAccount {
  label: string;
  org?: string;
  plan?: string;
  limits: UsageLimit[];
}

export interface UsageLimit {
  id: string;
  label: string;
  window?: string;
  resetsAt?: number;
  unit: string;
  used?: number;
  limit?: number;
  /** 0–1; above 1 means overage. */
  usedFraction?: number;
  status?: 'ok' | 'warning' | 'exhausted' | 'unknown';
}

// ─── omp extension_ui_request payloads (crates/omp-rpc/src/wire.rs) ─────────────────────────

export interface SelectOptionDetail {
  description?: string;
  [key: string]: unknown;
}

export interface SelectUiPayload {
  type: 'extension_ui_request';
  method: 'select';
  id: string;
  title: string;
  options: string[];
  optionDetails?: SelectOptionDetail[];
  timeout?: number;
}

export interface ConfirmUiPayload {
  type: 'extension_ui_request';
  method: 'confirm';
  id: string;
  title: string;
  message: string;
  timeout?: number;
}

export interface InputUiPayload {
  type: 'extension_ui_request';
  method: 'input';
  id: string;
  title: string;
  placeholder?: string;
  timeout?: number;
}

export interface EditorUiPayload {
  type: 'extension_ui_request';
  method: 'editor';
  id: string;
  title: string;
  prefill?: string;
  promptStyle?: boolean;
}

export interface AskOption {
  label: string;
  description?: string;
  preview?: string;
}

export interface AskQuestion {
  id: string;
  question: string;
  options: AskOption[];
  header?: string;
  multi?: boolean;
  /** Index into `options` of the recommended choice. */
  recommended?: number;
}

export interface AskUiPayload {
  type: 'extension_ui_request';
  method: 'ask';
  id: string;
  questions: AskQuestion[];
  timeout?: number;
}

/** A file Coocoo wrote a secret into. */
export interface SecretFile {
  /** Absolute path on the agent's host. */
  path: string;
  /** Path relative to the directory it was written in (agent's cwd/worktree or the project), shown in lists. */
  relPath: string;
  format: 'dotenv' | 'raw';
  /** Git branch of that directory when written (worktree branch or the project's branch). */
  branch?: string;
}

/** A project's secret without its value (`secrets.*`, `project.secrets`). */
export interface SecretInfo {
  /** Env var name, `^[A-Z_][A-Z0-9_]*$`. Every agent of the project gets `$NAME`. */
  name: string;
  files: SecretFile[];
  /** The agent that asked for it via coocoo_secret_request; absent = added by the user. */
  addedBy?: { id: string; title: string };
  createdAt: number;
  updatedAt: number;
}

/** Payload of a `UiRequest` with `method: 'secret'` (created by Coocoo, not omp). Never contains a value. */
export interface SecretUiPayload {
  method: 'secret';
  name: string;
  /** Why the agent needs it (agent-written text, show as a quote). */
  description: string;
  projectId: string;
  projectName: string;
  /** PEM/JSON etc.: use a (masked) textarea instead of a password input. */
  multiline?: boolean;
  /** A secret with this name exists already; saving replaces it. */
  replace?: boolean;
  /** Also written to this file in the agent's directory. */
  file?: {
    /** Relative to the agent's directory, as the agent gave it. */
    path: string;
    format: 'dotenv' | 'raw';
    /** Already ignored by git. */
    gitIgnored: boolean;
    /** Not ignored: Coocoo adds it to .git/info/exclude so no commit picks it up. */
    exclude: boolean;
  };
  /** Branch of the agent's directory. */
  branch?: string;
  /** The agent works in its own worktree. */
  worktree?: boolean;
}

export type UiRequestPayload =
  | SelectUiPayload
  | ConfirmUiPayload
  | InputUiPayload
  | EditorUiPayload
  | AskUiPayload
  | SecretUiPayload;

export interface AskAnswer {
  id: string;
  selectedOptions: string[];
  customInput?: string;
}

/** `agents.respond.response` body (omp reply without `type`/`id`). */
export type UiResponse =
  | { value: string }
  | { confirmed: boolean }
  | { answers: AskAnswer[] }
  | { cancelled: true };

// ─── Browser ──────────────────────────────────────────────────────────────────────────────────

export interface BrowserTab {
  targetId: string;
  url: string;
  title: string;
  active: boolean;
}

export interface BrowserProfileInfo {
  id: string;
  running: boolean;
  pid?: number;
  tabs: BrowserTab[];
  /** Loopback CDP discovery URL handed to omp (`browser.cdpUrl`); empty until first requested. */
  cdpUrl: string;
}

export interface BrowserFrame {
  profile: string;
  targetId: string;
  /** Base64 JPEG. */
  data: string;
  width: number;
  height: number;
  seq: number;
}

export interface BrowserConsoleEntry {
  profile: string;
  targetId: string;
  level: 'log' | 'info' | 'warning' | 'error' | 'debug' | 'exception' | 'network' | string;
  text: string;
  ts: number;
}

export type Modifiers = number; // CDP bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8

export type BrowserInputEvent =
  | {
      type: 'mouse';
      action: 'down' | 'up' | 'move' | 'wheel';
      x: number;
      y: number;
      button?: 'left' | 'middle' | 'right' | 'none';
      clickCount?: number;
      deltaX?: number;
      deltaY?: number;
      modifiers?: Modifiers;
    }
  | { type: 'key'; action: 'down' | 'up'; key: string; code: string; text?: string; keyCode?: number; modifiers?: Modifiers }
  | { type: 'text'; text: string };

export interface InspectResult {
  selector: string;
  tag: string;
  html: string;
  text: string;
  styles: Record<string, string>;
  box: { x: number; y: number; width: number; height: number };
  /** Base64 PNG of the element. */
  screenshot: string;
}

// ─── Methods ──────────────────────────────────────────────────────────────────────────────────

type Empty = Record<string, never>;
type Ok = Record<string, never>;

export type CreateWorktree = boolean | { branch?: string; base?: string };

export type AddHostParams =
  | { kind: 'direct'; url: string; token: string; name?: string }
  | { kind: 'ssh'; target: string; sshPort?: number; remotePort?: number; token?: string; install?: boolean; name?: string };

// ─── Insights (usage statistics and workflows) ────────────────────────────────────────────────

export type Device = 'desktop' | 'phone' | 'browser';

export interface InsightsQuery {
  /** Unix ms, inclusive. */
  from: number;
  /** Unix ms, exclusive. */
  to: number;
  /** `-new Date().getTimezoneOffset()`: days and the heatmap use it. */
  tzOffset: number;
  project?: string;
  /** Model name as omp records it (`claude-opus-5-5`). */
  model?: string;
  /** Host ids; absent = every host. */
  hosts?: string[];
  /** Compare with the equally long period before `from` (default true). */
  compare?: boolean;
}

export interface InsightHostSource {
  hostId: string;
  name: string;
  /** The host answered. */
  ok: boolean;
  /** Why it could not answer (offline, timeout). */
  error?: string;
  statsDb?: string;
  syncedAt?: number;
  schema?: string;
  /** Why token/cost numbers are missing (no database, unknown schema, sync failed). */
  statsError?: string;
  /** Oldest usage-log entry; older workflow data is reconstructed. */
  logSince?: number;
  logEnabled: boolean;
}

export interface InsightCost {
  total: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface InsightTokens {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface InsightTotals {
  cost: InsightCost;
  tokens: InsightTokens;
  requests: number;
  failed: number;
  aborted: number;
  prompts: number;
  commits: number;
  linesAdded: number;
  linesDeleted: number;
  agents: number;
  subagents: number;
  activeMs: number;
  activeDaySet: number[];
  activeDays: number;
  /** Longest run of consecutive active days in the period. */
  streak: number;
  cardsDone: number;
}

export interface InsightDay {
  /** Days since 1970-01-01 in the query's time zone. */
  index: number;
  /** `YYYY-MM-DD`. */
  date: string;
  /** Unix ms of the local midnight starting the day. */
  start: number;
  models: Record<string, { cost: number; tokens: number; requests: number }>;
  commits: number;
  prompts: number;
  flagged: number;
  cardsDone: number;
  cardsDoneApprox: number;
}

export interface InsightModelRow {
  model: string;
  provider: string;
  requests: number;
  failed: number;
  cost: InsightCost;
  tokens: InsightTokens;
  /** `cacheRead / (input + cacheRead + cacheWrite)`. */
  cacheHit: number;
  /** Mean time to first token, ms. */
  ttft?: number;
  /** Mean output tokens per second of generation. */
  tps?: number;
}

export interface InsightProjectRow {
  hostId: string;
  projectId: string;
  name: string;
  agents: number;
  cost: number;
  commits: number;
  linesAdded: number;
  linesDeleted: number;
  cardsDone: number;
}

export type CommitMatch = 'exact' | 'turn' | 'shared';

export interface InsightCommit {
  sha: string;
  ts: number;
  summary: string;
  linesAdded: number;
  linesDeleted: number;
  match: CommitMatch;
}

export interface InsightAgentRow {
  hostId: string;
  id: string;
  title: string;
  projectId?: string;
  parentId?: string;
  createdAt: number;
  deleted?: boolean;
  archived?: boolean;
  /** Most requests first. */
  models: string[];
  prompts: number;
  requests: number;
  failed: number;
  toolCalls: number;
  activeMs: number;
  cost: InsightCost;
  costBy: { main: number; subagents: number; advisor: number };
  /** Requests per omp agent type (`main`, `subagent`, `advisor`). */
  requestsBy: Record<string, number>;
  subagentSessions: number;
  sessionFiles: string[];
  commits: InsightCommit[];
  linesAdded: number;
  linesDeleted: number;
  cardsDone: number;
  /** Cost per day, aligned with `UsageReport.days`. */
  dayCost: number[];
}

export interface FlaggedPrompt {
  ts: number;
  hostId: string;
  agentId?: string;
  text: string;
  signals: string[];
}

export interface FrustrationSignals {
  yelling: number;
  profanity: number;
  anguish: number;
  negation: number;
  repetition: number;
  blame: number;
}

export interface UsageReport {
  query?: InsightsQuery;
  hosts: InsightHostSource[];
  totals: InsightTotals;
  /** The equally long period before `from`. */
  prev?: InsightTotals;
  /** Every day of the period, oldest first. */
  days: InsightDay[];
  models: InsightModelRow[];
  projects: InsightProjectRow[];
  agents: InsightAgentRow[];
  tools: { name: string; calls: number; errors: number }[];
  /** Prompts by local weekday (0 = Monday) × hour (0–23). */
  heatmap: number[][];
  frustration: { prompts: number; flagged: number; signals: FrustrationSignals; examples: FlaggedPrompt[] };
  /** Models with requests in the period (ignores the model filter). */
  allModels: string[];
}

/** A concrete example behind a number: an agent (and when), or a card. */
export interface ExampleRef {
  hostId: string;
  agentId?: string;
  projectId?: string;
  cardId?: string;
  label?: string;
  ts: number;
}

export interface WorkflowKpi {
  value: number;
  prev?: number;
  extra?: number;
  n: number;
}

export interface FunnelStage {
  id: string;
  label: string;
  count: number;
  medianToNextMs?: number;
  /** Reached this stage but not the next, newest first. */
  stuck: ExampleRef[];
}

export interface WorkflowShape {
  id: string;
  label: string;
  /** The rule, in words. */
  rule: string;
  agents: number;
  promptsMedian: number;
  turnMedianMs: number;
  costAvg: number;
  commitRate?: number;
  examples: ExampleRef[];
}

export interface WorkflowSequence {
  /** Step ids (`card.refine`, `request.answer`, `ui.leave`…). */
  steps: string[];
  count: number;
  medianMs: number;
  commits: number;
  friction: number;
  phone: number;
  examples: ExampleRef[];
}

export interface WorkflowFeature {
  id: string;
  label: string;
  uses: number;
  days: number;
  lastUsed?: number;
  desktop: number;
  phone: number;
  browser: number;
}

export interface WorkflowFriction {
  id: string;
  label: string;
  count: number;
  prev?: number;
  examples: ExampleRef[];
}

export interface WorkflowReport {
  query?: InsightsQuery;
  hosts: InsightHostSource[];
  logSince?: number;
  events: number;
  reconstructed: number;
  kpis: {
    /** `extra` = from Kanban cards. */
    agentsStarted: WorkflowKpi;
    agentsByAgents: number;
    /** Median; `extra` = p90. */
    promptsPerAgent: WorkflowKpi;
    /** Tool calls per prompt. */
    autonomy: WorkflowKpi;
    /** Median ms; `extra` = p90. */
    needsYouWait: WorkflowKpi;
    /** Share of turns; `extra` = aborts followed by a steer. */
    abortRate: WorkflowKpi;
    /** Share of agents with ≥ 1 commit. */
    agentCommit: WorkflowKpi;
  };
  journey: FunnelStage[];
  kanban: {
    stages: FunnelStage[];
    backMoves: number;
    /** [finished in one run, finished]. */
    refinedOneRun: [number, number];
    unrefinedOneRun: [number, number];
  };
  shapes: WorkflowShape[];
  sequences: WorkflowSequence[];
  features: WorkflowFeature[];
  friction: WorkflowFriction[];
  lastAnalyze?: { hostId: string; projectId: string; agentId: string; at: number; cards: number; accepted: number };
}

export interface InsightEvent {
  ts: number;
  kind: string;
  actor: 'user' | 'agent' | 'daemon';
  device?: Device;
  clientId?: string;
  host?: string;
  agentId?: string;
  projectId?: string;
  cardId?: string;
  ok: boolean;
  ms?: number;
  meta?: Record<string, unknown>;
  reconstructed?: boolean;
}

/** A client event for `insights.track` (`kind` starts with `ui.`). */
export interface TrackEvent {
  ts: number;
  kind: string;
  agentId?: string;
  projectId?: string;
  cardId?: string;
  meta?: Record<string, string | number | boolean>;
}

export interface InsightsSettings {
  enabled: boolean;
  retentionDays: number;
  logSince?: number;
  /** Size of the log files. */
  bytes: number;
}

export interface Methods {
  'system.hello': [{ client?: { id: string; device: Device } } | undefined, { protocol: number; host: HostInfo; hosts: HostInfo[] }];
  'system.subscribe': [{ scope: 'all' | 'local' }, Ok];
  'hosts.list': [Empty | undefined, HostInfo[]];
  'hosts.add': [AddHostParams, HostInfo];
  'hosts.remove': [{ id: string }, Ok];
  'hosts.rename': [{ id: string; name: string }, HostInfo];
  'stats.watch': [{ enabled: boolean }, Ok];
  'stats.get': [Empty | undefined, HostStats];

  'insights.usage': [InsightsQuery, UsageReport];
  'insights.workflows': [InsightsQuery, WorkflowReport];
  /** Content-free usage-log events of every host, oldest first. */
  'insights.events': [InsightsQuery & { limit?: number }, { events: InsightEvent[]; truncated: boolean; hosts: InsightHostSource[] }];
  'insights.track': [{ clientId: string; device: Device; events: TrackEvent[] }, Ok];
  'insights.settings': [{ enabled?: boolean; retentionDays?: number } | undefined, InsightsSettings];
  'insights.clear': [Empty | undefined, InsightsSettings];
  /** Starts the pattern-analysis agent in `projectId` (on that project's host). */
  'insights.analyze': [{ projectId: string; from: number; to: number; tzOffset: number; focus?: string }, AgentSummary];

  'terminal.list': [Empty | undefined, TerminalInfo[]];
  'terminal.open': [{ cwd: string; cols?: number; rows?: number }, TerminalInfo];
  'terminal.attach': [{ id: string; cols?: number; rows?: number }, { data: string }];
  'terminal.detach': [{ id: string }, Ok];
  'terminal.input': [{ id: string; data: string }, Ok];
  'terminal.resize': [{ id: string; cols: number; rows: number }, Ok];
  'terminal.kill': [{ id: string }, Ok];
  'terminal.close': [{ id: string }, Ok];

  'actions.run': [{ projectId: string; actionId: string; cwd: string; cols?: number; rows?: number }, TerminalInfo];

  'projects.list': [Empty | undefined, Project[]];
  'projects.get': [{ id: string }, Project];
  'projects.add': [{ path: string; name?: string }, Project];
  'projects.update': [{ id: string; name?: string; devUrls?: string[] }, Project];
  'projects.remove': [{ id: string }, Ok];
  'projects.refresh': [{ id: string }, Project];
  'projects.icon': [{ id: string }, { mime: string; data: string }];
  'git.diff': [{ cwd: string; base?: string }, DiffResult];
  'git.diffStat': [{ cwd: string; base?: string }, DiffStat];
  'git.log': [{ cwd: string; limit?: number }, CommitInfo[]];
  'git.commit': [{ cwd: string; message: string }, CommitInfo];
  'git.merge': [{ projectId: string; branch: string }, { ok: boolean; output: string }];
  'git.fetch': [{ projectId: string }, { ok: boolean; output: string }];
  'git.fastForward': [{ projectId: string; path: string }, { ok: boolean; output: string }];
  /** `rejected`: non-fast-forward. Never forces; without upstream pushes with `-u`. */
  'git.push': [{ projectId: string; path: string }, { ok: boolean; output: string; rejected: boolean }];
  /** Commits a `git.push` would send, newest first (≤ 200). */
  'git.outgoing': [{ projectId: string; path: string }, CommitInfo[]];
  'git.prCreate': [{ projectId: string; path: string; title: string; body: string; draft?: boolean; base?: string }, PullRequestInfo];
  'git.prStatus': [{ projectId: string; path: string }, PullRequestInfo | null];
  'git.prMerge': [{ projectId: string; path: string; method: MergeMethod; deleteBranch: boolean }, { ok: boolean; output: string }];
  'git.worktreeCreate': [{ projectId: string; branch?: string; base?: string }, WorktreeRef];
  'git.worktreeRemove': [{ projectId: string; path: string; deleteBranch?: boolean }, Ok];
  'fs.list': [{ path?: string }, { path: string; parent?: string; entries: FsEntry[] }];

  'agents.list': [{ projectId?: string } | undefined, AgentSummary[]];
  'agents.get': [{ id: string }, AgentDetail];
  'agents.create': [
    {
      projectId?: string;
      cwd?: string;
      title?: string;
      prompt?: string;
      worktree?: CreateWorktree;
      model?: string;
      thinking?: string;
      parentId?: string;
    },
    AgentSummary,
  ];
  'agents.prompt': [{ id: string; message: string; mode?: 'auto' | 'steer' | 'followUp'; images?: ImageInput[] }, Ok];
  'agents.abort': [{ id: string }, Ok];
  'agents.respond': [{ id: string; requestId: string; response: UiResponse }, Ok];
  'agents.timeline': [{ id: string; before?: number; limit?: number }, TimelinePage];
  'agents.output': [{ id: string }, { status: AgentStatus; text?: string }];
  'agents.markRead': [{ id: string }, Ok];
  'agents.rename': [{ id: string; title: string }, AgentSummary];
  'agents.setModel': [{ id: string; model: string }, AgentSummary];
  'agents.setThinking': [{ id: string; level: string }, AgentSummary];
  'agents.models': [Empty | undefined, ModelInfo[]];
  'agents.commands': [{ id: string }, SlashCommand[]];
  'agents.usage': [{ provider: string; model?: string }, ProviderUsage];
  'agents.subagentTimeline': [{ id: string; sessionFile: string; live?: boolean }, TimelinePage];
  'agents.image': [{ id: string; name: string }, { mime: string; data: string }];
  'boards.get': [{ projectId: string }, Board];
  'boards.cardAdd': [{ projectId: string; title: string; body?: string; column?: CardColumn; top?: boolean; startUrl?: string; doneUrl?: string; images?: ImageUpload[] }, Card];
  'boards.cardUpdate': [{ projectId: string; cardId: string; title?: string; body?: string; column?: CardColumn; index?: number; startUrl?: string; doneUrl?: string; addImages?: ImageUpload[]; removeImages?: string[] }, Card];
  'boards.cardDelete': [{ projectId: string; cardId: string }, Ok];
  /** Without `agentId` a new agent is created for the card (optionally in a worktree). */
  'boards.cardStart': [{ projectId: string; cardId: string; agentId?: string; worktree?: boolean; model?: string }, Card];
  'boards.cardRefine': [{ projectId: string; cardId: string; agentId?: string; worktree?: boolean; model?: string }, Card];
  'boards.attachment': [{ projectId: string; cardId: string; name: string }, { mime: string; data: string }];
  'boards.cardImage': [{ projectId: string; cardId: string; name: string }, { mime: string; data: string }];
  'boards.cardDone': [
    { projectId: string; cardId: string; agentId?: string; commitMessage?: string; note?: string; prTitle?: string; prBody?: string },
    { card: Card; commit?: CommitInfo; commitSkipped?: string; pushed?: boolean; pullRequest?: { number: number; url: string }; gitError?: string },
  ];
  'boards.update': [{ projectId: string; doneAction: DoneAction }, Board];
  /** Retries push (+ PR) for a done card. */
  'boards.cardPublish': [{ projectId: string; cardId: string }, { card: Card; pushed: boolean; pullRequest?: { number: number; url: string }; error?: string }];
  'boards.sourceAdd': [
    { projectId: string; kind: SourceKind; target?: string; name?: string; script?: string; intervalMins?: number; agentId?: string },
    SourceRun,
  ];
  'boards.sourceUpdate': [
    { projectId: string; sourceId: string; name?: string; target?: string; script?: string; intervalMins?: number; agentId?: string },
    SourceRun,
  ];
  'boards.sourceRemove': [{ projectId: string; sourceId: string }, Board];
  'boards.sourceRefresh': [{ projectId: string; sourceId: string }, SourceRun];
  /** Without `projectId` all schedules of the host, oldest first. */
  'schedules.list': [{ projectId?: string }, Schedule[]];
  /** Exactly one of `cron`/`at`; `at` must lie in the future; `timezone` defaults to the host's zone. */
  'schedules.add': [
    { projectId: string; name: string; prompt: string; cron?: string; at?: number; timezone?: string; agentId?: string; worktree?: boolean; model?: string; enabled?: boolean },
    Schedule,
  ];
  /** Giving `cron` drops `at` and vice versa; `agentId: ""` = new agent per run, `model: ""` = default. */
  'schedules.update': [
    { projectId: string; scheduleId: string; name?: string; prompt?: string; cron?: string; at?: number; timezone?: string; agentId?: string; worktree?: boolean; model?: string; enabled?: boolean },
    Schedule,
  ];
  'schedules.remove': [{ projectId: string; scheduleId: string }, Ok];
  /** Runs it now (also when disabled); `nextRunAt` stays. */
  'schedules.run': [{ projectId: string; scheduleId: string }, Schedule];
  /** Next run times (Unix ms) of a cron expression; `invalid_params` with the parse error. */
  'schedules.preview': [{ cron: string; timezone?: string; count?: number }, { next: number[] }];
  /** Sorted by name; values are never returned. */
  'secrets.list': [{ projectId: string }, SecretInfo[]];
  /** Create or replace; replacing rewrites every listed file; `file` (relative to the project) is written too. */
  'secrets.set': [{ projectId: string; name: string; value: string; file?: { path: string; format: 'dotenv' | 'raw' } }, SecretInfo];
  /** Removes it from the store and the `NAME=` line / raw file Coocoo wrote. */
  'secrets.remove': [{ projectId: string; name: string }, Ok];
  /** Answers a `secret` UiRequest of agent `id`; decline with `agents.respond` `{ cancelled: true }`. */
  'secrets.provide': [{ id: string; requestId: string; value: string }, Ok];
  'agents.hibernate': [{ id: string }, AgentSummary];
  'agents.archive': [{ id: string }, AgentSummary];
  'agents.delete': [{ id: string; removeWorktree?: boolean; deleteBranch?: boolean }, Ok];

  'browser.profiles': [Empty | undefined, BrowserProfileInfo[]];
  'browser.start': [{ profile: string }, BrowserProfileInfo];
  'browser.stop': [{ profile: string }, BrowserProfileInfo];
  'browser.open': [{ profile: string; url: string }, BrowserTab];
  'browser.navigate': [{ profile: string; targetId: string; url?: string; action?: 'back' | 'forward' | 'reload' }, Ok];
  'browser.activate': [{ profile: string; targetId: string }, Ok];
  'browser.closeTab': [{ profile: string; targetId: string }, Ok];
  'browser.watch': [{ profile: string; targetId: string; maxWidth?: number; maxHeight?: number; quality?: number }, Ok];
  'browser.unwatch': [{ profile: string; targetId: string }, Ok];
  'browser.resize': [{ profile: string; targetId: string; width: number; height: number }, Ok];
  'browser.input': [{ profile: string; targetId: string; event: BrowserInputEvent }, Ok];
  'browser.console': [{ profile: string; targetId: string }, BrowserConsoleEntry[]];
  'browser.screenshot': [{ profile: string; targetId: string }, { data: string }];
  'browser.inspect': [{ profile: string; targetId: string; x: number; y: number }, InspectResult];
  'browser.highlight': [{ profile: string; targetId: string; x?: number; y?: number }, Ok];
  'browser.cdpEndpoint': [{ profile: string }, { url: string }];
}

export type Method = keyof Methods;
export type ParamsOf<M extends Method> = Methods[M][0];
export type ResultOf<M extends Method> = Methods[M][1];

// ─── Events ───────────────────────────────────────────────────────────────────────────────────

export interface Events {
  'host.updated': HostInfo;
  'host.removed': { id: string };
  'host.stats': HostStats;
  'project.updated': Project;
  'project.removed': { id: string };
  'agent.updated': AgentSummary;
  'agent.removed': { id: string };
  'agent.timeline': TimelineEvent;
  'agent.delta': TimelineDelta;
  'agent.request': UiRequest;
  'agent.requestResolved': { agentId: string; requestId: string };
  'project.board': { projectId: string; board: Board };
  'project.schedules': { projectId: string; schedules: Schedule[] };
  'project.secrets': { projectId: string; secrets: SecretInfo[] };
  'agent.commands': { agentId: string; commands: SlashCommand[] };
  'browser.updated': BrowserProfileInfo;
  'browser.frame': BrowserFrame;
  'browser.console': BrowserConsoleEntry;
  'terminal.output': { id: string; data: string };
  'terminal.exit': { id: string; code?: number | null };
  'terminal.updated': TerminalInfo;
  'terminal.removed': { id: string };
  'system.resync': Empty;
}

export type EventName = keyof Events;

export const PROTOCOL_VERSION = 1;
