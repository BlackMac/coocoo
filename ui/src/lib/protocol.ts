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

/** A login shell on a pseudo-terminal; keeps running while nobody is attached. */
export interface TerminalInfo {
  id: string;
  cwd: string;
  shell: string;
  cols: number;
  rows: number;
  createdAt: number;
}

// ─── Projects / git ───────────────────────────────────────────────────────────────────────────

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
}

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
  sessionFile?: string;
  diff?: DiffStat;
  subagents: number;
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

export type UiRequestMethod = 'select' | 'confirm' | 'input' | 'editor' | 'ask';

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
}

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
  /** The latest refinement, handed to the agent that implements the card. */
  refinement?: Refinement;
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

export type UiRequestPayload = SelectUiPayload | ConfirmUiPayload | InputUiPayload | EditorUiPayload | AskUiPayload;

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

export interface Methods {
  'system.hello': [Empty | undefined, { protocol: number; host: HostInfo; hosts: HostInfo[] }];
  'system.subscribe': [{ scope: 'all' | 'local' }, Ok];
  'hosts.list': [Empty | undefined, HostInfo[]];
  'hosts.add': [AddHostParams, HostInfo];
  'hosts.remove': [{ id: string }, Ok];
  'hosts.rename': [{ id: string; name: string }, HostInfo];
  'stats.watch': [{ enabled: boolean }, Ok];
  'stats.get': [Empty | undefined, HostStats];

  'terminal.list': [Empty | undefined, TerminalInfo[]];
  'terminal.open': [{ cwd: string; cols?: number; rows?: number }, TerminalInfo];
  'terminal.attach': [{ id: string; cols?: number; rows?: number }, { data: string }];
  'terminal.detach': [{ id: string }, Ok];
  'terminal.input': [{ id: string; data: string }, Ok];
  'terminal.resize': [{ id: string; cols: number; rows: number }, Ok];
  'terminal.kill': [{ id: string }, Ok];

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
  'boards.get': [{ projectId: string }, Board];
  'boards.cardAdd': [{ projectId: string; title: string; body?: string; column?: CardColumn; top?: boolean; startUrl?: string; doneUrl?: string }, Card];
  'boards.cardUpdate': [{ projectId: string; cardId: string; title?: string; body?: string; column?: CardColumn; index?: number; startUrl?: string; doneUrl?: string }, Card];
  'boards.cardDelete': [{ projectId: string; cardId: string }, Ok];
  /** Without `agentId` a new agent is created for the card (optionally in a worktree). */
  'boards.cardStart': [{ projectId: string; cardId: string; agentId?: string; worktree?: boolean; model?: string }, Card];
  'boards.cardRefine': [{ projectId: string; cardId: string; agentId?: string; worktree?: boolean; model?: string }, Card];
  'boards.attachment': [{ projectId: string; cardId: string; name: string }, { mime: string; data: string }];
  'boards.cardDone': [{ projectId: string; cardId: string; agentId?: string; commitMessage?: string; note?: string }, { card: Card; commit?: CommitInfo; commitSkipped?: string }];
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
  'agents.hibernate': [{ id: string }, AgentSummary];
  'agents.archive': [{ id: string }, AgentSummary];
  'agents.delete': [{ id: string; removeWorktree?: boolean }, Ok];

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
  'agent.commands': { agentId: string; commands: SlashCommand[] };
  'browser.updated': BrowserProfileInfo;
  'browser.frame': BrowserFrame;
  'browser.console': BrowserConsoleEntry;
  'terminal.output': { id: string; data: string };
  'terminal.exit': { id: string; code?: number | null };
  'system.resync': Empty;
}

export type EventName = keyof Events;

export const PROTOCOL_VERSION = 1;
