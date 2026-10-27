// Fleet store: every host, project, agent, pending request and browser profile the home daemon
// can see, kept current from events. Entities from peers are keyed `${hostId}:${id}`.

import { Client, type ConnState } from './client';
import { notify } from './notify';
import type {
  AgentDetail,
  AgentSummary,
  Board,
  BrowserProfileInfo,
  HostInfo,
  HostStats,
  ImageInput,
  Method,
  ModelInfo,
  ParamsOf,
  Project,
  ProviderUsage,
  ResultOf,
  Schedule,
  SecretInfo,
  SlashCommand,
  TerminalInfo,
  TimelineItem,
  UiRequest,
  UiResponse,
} from './protocol';
import { href } from './router.svelte';
import { agentState } from './status';
import { parseError } from './providerError';
import { clientId, device } from './track';
import type { Draft, ImageChip } from './ui.svelte';

export const key = (host: string, id: string) => `${host}:${id}`;

const TIMELINE_PAGE = 120;
const TIMELINE_CACHE = 8;
/** How long a delivered prompt may stay unconfirmed once its agent is idle again. */
const OUTBOX_GRACE = 2000;

export type PromptMode = 'auto' | 'steer' | 'followUp';

/**
 * A prompt sent from this client, shown provisionally until omp confirms it: in the transcript
 * (`auto`: a new turn) or in the queued strip (`steer`/`followUp` while the agent runs).
 */
export interface Outgoing {
  id: number;
  message: string;
  images: ImageInput[];
  /** One-line form: the message plus `[N images]`. omp's `user` item matches it with its `images`
   *  counted the same way (daemons before `TimelineItem.images` put the marker into the text). */
  text: string;
  /** What the composer held, for Edit. */
  draft: Draft;
  mode: PromptMode;
  target: 'transcript' | 'queue';
  /** `sent`: the daemon accepted it; its `user` item (or queue entry) has not arrived yet. */
  state: 'sending' | 'sent' | 'failed';
  error?: string;
  /** When it was sent (`sending`) or accepted (`sent`). */
  sentAt: number;
}

let outgoingSeq = 1;

/** One agent's transcript: loaded lazily, paginated backwards, live-updated from events. */
export class Timeline {
  items = $state<TimelineItem[]>([]);
  hasMore = $state(false);
  loading = $state(false);
  loaded = $state(false);
  error = $state<string | null>(null);

  readonly host: string;
  readonly id: string;
  #fleet: Fleet;
  #index = new Map<string, number>();
  /** Events that arrived while the first page was in flight. */
  #backlog: ((t: Timeline) => void)[] | null = null;

  constructor(fleet: Fleet, host: string, id: string) {
    this.#fleet = fleet;
    this.host = host;
    this.id = id;
  }

  async load(): Promise<void> {
    this.loading = true;
    this.error = null;
    this.#backlog = [];
    try {
      const page = await this.#fleet.call('agents.timeline', { id: this.id, limit: TIMELINE_PAGE }, this.host);
      this.items = page.items;
      this.hasMore = page.hasMore;
      this.#reindex();
      this.loaded = true;
      const backlog = this.#backlog;
      this.#backlog = null;
      for (const apply of backlog) apply(this);
    } catch (err) {
      this.#backlog = null;
      this.error = err instanceof Error ? err.message : String(err);
    } finally {
      this.loading = false;
    }
  }

  /** Loads the page before the oldest loaded item. Resolves to the number of items added. */
  async loadOlder(): Promise<number> {
    if (!this.hasMore || this.loading || !this.items.length) return 0;
    this.loading = true;
    try {
      const before = this.items[0].seq;
      const page = await this.#fleet.call('agents.timeline', { id: this.id, before, limit: TIMELINE_PAGE }, this.host);
      const fresh = page.items.filter((i) => !this.#index.has(i.id));
      this.items.unshift(...fresh);
      this.hasMore = page.hasMore;
      this.#reindex();
      return fresh.length;
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
      return 0;
    } finally {
      this.loading = false;
    }
  }

  upsert(item: TimelineItem) {
    if (this.#backlog) return void this.#backlog.push((t) => t.upsert(item));
    if (!this.loaded) return;
    const idx = this.#index.get(item.id);
    if (idx !== undefined) {
      item.seq = this.items[idx].seq;
      this.items[idx] = item;
      return;
    }
    const last = this.items.at(-1);
    if (!last || item.seq > last.seq) {
      this.#index.set(item.id, this.items.length);
      this.items.push(item);
      return;
    }
    // Older than the loaded window: paging fetches it (unless everything is loaded already).
    if (item.seq < this.items[0].seq && this.hasMore) return;
    const at = this.items.findIndex((i) => i.seq > item.seq);
    this.items.splice(at, 0, item);
    this.#reindex();
  }

  delta(itemId: string, text: string) {
    if (this.#backlog) return void this.#backlog.push((t) => t.delta(itemId, text));
    const idx = this.#index.get(itemId);
    if (idx === undefined) return;
    const item = this.items[idx];
    item.text = (item.text ?? '') + text;
  }

  #reindex() {
    this.#index.clear();
    this.items.forEach((it, i) => this.#index.set(it.id, i));
  }
}

export interface AttentionItem {
  kind: 'request' | 'error' | 'review';
  agentKey: string;
  agent: AgentSummary;
  request?: UiRequest;
  ts: number;
}

export class Fleet {
  client: Client | null = null;
  native = $state(false);
  conn = $state<ConnState>('connecting');
  /** True once the first hello completed (initial data present). */
  ready = $state(false);
  homeId = $state('');
  protocol = $state(0);

  hosts = $state<Record<string, HostInfo>>({});
  stats = $state<Record<string, HostStats>>({});
  projects = $state<Record<string, Project>>({});
  agents = $state<Record<string, AgentSummary>>({});
  requests = $state<Record<string, UiRequest[]>>({});
  profiles = $state<Record<string, BrowserProfileInfo>>({});
  details = $state<Record<string, AgentDetail>>({});
  models = $state<Record<string, ModelInfo[]>>({});
  /** Kanban boards per `host:projectId`, loaded on demand and kept current by `project.board`. */
  boards = $state<Record<string, Board>>({});
  /** Scheduled runs per `host:projectId`, loaded on demand and kept current by `project.schedules`. */
  schedules = $state<Record<string, Schedule[]>>({});
  /** Secrets (names and files, never values) per `host:projectId`, kept current by `project.secrets`. */
  secrets = $state<Record<string, SecretInfo[]>>({});
  /** Slash-command catalogs per `host:agentId`, loaded on demand and kept current by `agent.commands`. */
  commands = $state<Record<string, SlashCommand[]>>({});
  /** `agents.usage` per `host:provider`. */
  usage = $state<Record<string, ProviderUsage>>({});
  /** Terminals per `host:id` (shells and action runs), kept current by `terminal.updated/removed`. */
  terminals = $state<Record<string, TerminalInfo>>({});
  /** Prompts sent from this client that omp has not confirmed yet, per `host:agentId`, oldest first. */
  outbox = $state<Record<string, Outgoing[]>>({});
  /** Items with images (`agents.images`) per `host:agentId`, loaded by the Images tab, kept current by `agent.timeline`. */
  images = $state<Record<string, TimelineItem[]>>({});
  /** The latest picture an agent produced while this client watched: a tool result image by
   *  `name`, or `''` for a refinement (its mocks land on the board). `n` counts arrivals. */
  imageArrived = $state<{ key: string; name: string; n: number } | null>(null);
  /** Items that already announced a picture (a finished call is upserted again on subagent progress). */
  #announced = new Set<string>();
  #outboxTimers = new Map<string, number>();
  /** `user` item ids seen per agent: only a new one confirms a prompt (not a re-sent update). */
  #seenUser = new Map<string, Set<string>>();
  /** Local receipt time per `usage` key (hosts' clocks may differ). */
  #usageAt = new Map<string, number>();
  #usageLoads = new Map<string, Promise<ProviderUsage>>();
  /** Project icon and persona avatar data URLs per `host:id:path:modified` / `persona:host:id:…`. */
  projectIcons = $state<Record<string, string>>({});
  #iconLoads = new Set<string>();
  /** Sidebar host filter; `null` = all hosts. */
  hostFilter = $state<string | null>(null);

  hostList = $derived(
    Object.values(this.hosts).sort((a, b) => {
      if (a.kind === 'local' !== (b.kind === 'local')) return a.kind === 'local' ? -1 : 1;
      return a.name.localeCompare(b.name);
    }),
  );

  projectList = $derived(
    Object.values(this.projects)
      .filter((p) => !this.hostFilter || p.hostId === this.hostFilter)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );

  agentList = $derived(
    Object.values(this.agents)
      .filter((a) => !this.hostFilter || a.hostId === this.hostFilter)
      .sort((a, b) => b.lastActivityAt - a.lastActivityAt),
  );

  attention = $derived.by(() => {
    const out: AttentionItem[] = [];
    for (const a of this.agentList) {
      const k = key(a.hostId, a.id);
      const reqs = this.requests[k] ?? [];
      for (const r of reqs) out.push({ kind: 'request', agentKey: k, agent: a, request: r, ts: r.createdAt });
      if (reqs.length) continue;
      const st = agentState(a);
      if (st === 'error') out.push({ kind: 'error', agentKey: k, agent: a, ts: a.lastActivityAt });
      else if (st === 'review') out.push({ kind: 'review', agentKey: k, agent: a, ts: a.lastActivityAt });
    }
    const rank = { request: 0, error: 1, review: 2 };
    return out.sort((x, y) => rank[x.kind] - rank[y.kind] || (x.kind === 'review' ? y.ts - x.ts : x.ts - y.ts));
  });

  needsCount = $derived(this.attention.filter((i) => i.kind !== 'review').length);

  #timelines = new Map<string, Timeline>();
  #statsWatchers = 0;
  #watchingStats = new Set<string>();
  #detailTimers = new Map<string, number>();

  start(wsUrl: string, token: string, native: boolean) {
    this.native = native;
    const c = new Client(wsUrl, token);
    this.client = c;
    c.onState((s) => {
      this.conn = s;
      if (s !== 'open') this.#watchingStats.clear();
    });
    c.onOpen(() => void this.#hello());
    document.addEventListener('visibilitychange', () => this.#syncStatsWatch());
    this.#wire(c);
    c.connect();
  }

  call<M extends Method>(method: M, params?: ParamsOf<M>, host?: string, timeout?: number): Promise<ResultOf<M>> {
    if (!this.client) return Promise.reject(new Error('not connected'));
    return this.client.call(method, params, { host: host && host !== this.homeId ? host : undefined, timeout });
  }

  /** Fire-and-forget call (high-rate input forwarding); failures are ignored. */
  send<M extends Method>(method: M, params: ParamsOf<M>, host?: string): void {
    this.call(method, params, host).catch(() => {});
  }

  /** Answers a pending request and takes it off the list at once (`agent.requestResolved` follows). */
  async respond(host: string, request: UiRequest, response: UiResponse): Promise<void> {
    await this.call('agents.respond', { id: request.agentId, requestId: request.id, response }, host);
    this.dropRequest(host, request);
  }

  dropRequest(host: string, request: UiRequest): void {
    const list = this.requests[key(host, request.agentId)];
    const i = list?.findIndex((r) => r.id === request.id) ?? -1;
    if (list && i >= 0) list.splice(i, 1);
  }

  // ─── Lookups ────────────────────────────────────────────────────────────────────────────────

  host(id: string): HostInfo | undefined {
    return this.hosts[id];
  }

  project(host: string, id: string | undefined): Project | undefined {
    return id ? this.projects[key(host, id)] : undefined;
  }

  agent(host: string, id: string): AgentSummary | undefined {
    return this.agents[key(host, id)];
  }

  agentsOf(host: string, projectId: string): AgentSummary[] {
    return this.agentList.filter((a) => a.hostId === host && a.projectId === projectId);
  }

  /** The run of project action `actionId` started for directory `base` on `host`, if any. */
  actionRun(host: string, projectId: string, actionId: string, base: string): TerminalInfo | undefined {
    let found: TerminalInfo | undefined;
    for (const [k, t] of Object.entries(this.terminals)) {
      const a = t.action;
      if (!a || a.projectId !== projectId || a.actionId !== actionId || a.base !== base || !k.startsWith(`${host}:`)) continue;
      if (!found || t.createdAt > found.createdAt) found = t;
    }
    return found;
  }

  childrenOf(host: string, id: string): AgentSummary[] {
    return Object.values(this.agents).filter((a) => a.hostId === host && a.parentId === id);
  }

  /** Timeline for an agent; created and loaded on first access, LRU-evicted beyond a few. */
  timeline(host: string, id: string): Timeline {
    const k = key(host, id);
    let tl = this.#timelines.get(k);
    if (tl) {
      this.#timelines.delete(k);
      this.#timelines.set(k, tl);
      return tl;
    }
    tl = new Timeline(this, host, id);
    this.#timelines.set(k, tl);
    if (this.#timelines.size > TIMELINE_CACHE) {
      const oldest = this.#timelines.keys().next().value;
      if (oldest !== undefined) this.#timelines.delete(oldest);
    }
    // Deferred: callers may be inside a `$derived`, where state writes are not allowed.
    queueMicrotask(() => void tl.load());
    return tl;
  }

  /** Fetches (or refreshes) the full agent detail: todos, queued messages, requests. */
  async loadDetail(host: string, id: string): Promise<void> {
    try {
      const d = await this.call('agents.get', { id }, host);
      const k = key(host, id);
      this.details[k] = d;
      this.agents[k] = d.summary;
      this.requests[k] = d.requests;
      this.#settleQueued(k, d);
    } catch (err) {
      console.warn('agents.get failed', err);
    }
  }

  /** Fetches (or refreshes) the items of an agent's timeline that carry images. */
  async loadImages(host: string, id: string): Promise<void> {
    this.images[key(host, id)] = await this.call('agents.images', { id }, host);
  }

  /** Keeps a loaded image list current and announces pictures the agent produced. */
  #noteImages(k: string, item: TimelineItem) {
    if (item.partial) return;
    if (item.images?.length) {
      const list = this.images[k];
      if (list) {
        const i = list.findIndex((x) => x.id === item.id);
        if (i >= 0) list[i] = item;
        else list.push(item);
      }
    }
    const tool = item.tool;
    // A refinement counts when it attaches mocks (its arguments may be cut off before them).
    const mocks = tool?.name === 'kanban_refine' && tool.status === 'ok' && /"attachments"\s*:\s*\[\s*"/.test(tool.args);
    const produced = tool && (item.images?.length || mocks);
    const id = `${k}:${item.id}`;
    if (!produced || this.#announced.has(id)) return;
    this.#announced.add(id);
    this.imageArrived = { key: k, name: item.images?.at(-1)?.name ?? '', n: (this.imageArrived?.n ?? 0) + 1 };
  }

  // ─── Outbox (optimistic prompts) ────────────────────────────────────────────────────────────

  /** Sends a prompt, shown provisionally until omp confirms it; failures stay for Retry/Edit. */
  sendPrompt(host: string, agentId: string, out: { message: string; images: ImageInput[]; draft: Draft }, mode: PromptMode) {
    const entry = this.#addOutgoing(key(host, agentId), { ...out, mode, state: 'sending' });
    void this.#deliver(host, agentId, entry);
  }

  /** Records the first prompt of an agent `agents.create` started (already accepted). */
  noteSent(host: string, agentId: string, message: string, images: ImageChip[] = []) {
    const k = key(host, agentId);
    const draft = { text: message, images, elements: [] };
    this.#addOutgoing(k, { message, images: images.map((i) => i.image), draft, mode: 'auto', state: 'sent' });
    this.#sweepSoon(k);
  }

  retry(agentKey: string, id: number) {
    const e = this.outbox[agentKey]?.find((x) => x.id === id);
    if (!e || e.state !== 'failed') return;
    e.state = 'sending';
    e.error = undefined;
    e.sentAt = Date.now();
    const [host, agentId] = splitKey(agentKey);
    void this.#deliver(host, agentId, id);
  }

  discard(agentKey: string, id: number) {
    this.#dropWhere(agentKey, (e) => e.id === id);
  }

  #addOutgoing(k: string, e: Omit<Outgoing, 'id' | 'text' | 'target' | 'sentAt'>): number {
    const id = outgoingSeq++;
    const n = e.images.length;
    const text = [e.message.trim(), n ? `[${n} image${n === 1 ? '' : 's'}]` : ''].filter(Boolean).join('\n');
    // Through the store, so the push hits the reactive proxy rather than a raw literal.
    if (!this.outbox[k]) this.outbox[k] = [];
    this.outbox[k].push({ ...e, id, text, target: e.mode === 'auto' ? 'transcript' : 'queue', sentAt: Date.now() });
    return id;
  }

  async #deliver(host: string, agentId: string, id: number) {
    const k = key(host, agentId);
    const e = this.outbox[k]?.find((x) => x.id === id);
    if (!e) return;
    const params = { id: agentId, message: e.message, mode: e.mode, images: e.images.length ? $state.snapshot(e.images) : undefined };
    const queue = e.target === 'queue';
    try {
      await this.call('agents.prompt', params, host, 60_000);
      // Already confirmed, discarded or failed (the agent errored) meanwhile: nothing to do.
      const cur = this.outbox[k]?.find((x) => x.id === id);
      if (cur?.state === 'sending') {
        cur.state = 'sent';
        cur.sentAt = Date.now();
        this.#sweepSoon(k);
      }
      if (queue) this.#scheduleDetail(host, agentId);
    } catch (err) {
      const cur = this.outbox[k]?.find((x) => x.id === id);
      if (cur?.state !== 'sending') return;
      cur.state = 'failed';
      cur.error = `Not sent: ${err instanceof Error ? err.message : String(err)}`;
    }
  }

  /** Confirms a provisional prompt with a `user` item omp just produced. */
  #settleItem(k: string, item: TimelineItem) {
    if (item.kind !== 'user') return;
    // Ids, not seqs: the echo of a slash command has a seq below the output it precedes.
    let seen = this.#seenUser.get(k);
    if (!seen) this.#seenUser.set(k, (seen = new Set()));
    if (seen.has(item.id)) return;
    seen.add(item.id);
    const list = this.outbox[k];
    if (!list?.length) return;
    // The same text first (also a failed entry: its call timed out, but the daemon took it);
    // else the oldest open turn prompt (omp may have expanded a skill or template).
    const n = item.images?.length ?? 0;
    const text = [(item.text ?? '').trim(), n ? `[${n} image${n === 1 ? '' : 's'}]` : ''].filter(Boolean).join('\n');
    let i = list.findIndex((e) => e.text === text);
    if (i < 0) i = list.findIndex((e) => e.target === 'transcript' && e.state !== 'failed');
    if (i < 0) return;
    list.splice(i, 1);
    if (!list.length) delete this.outbox[k];
  }

  /** Drops provisional prompts the agent's queue now shows. */
  #settleQueued(k: string, d: AgentDetail) {
    if (!this.outbox[k]?.length) return;
    const queued = new Set([...d.queuedSteering, ...d.queuedFollowUp].map((t) => t.trim()));
    if (queued.size) this.#dropWhere(k, (e) => queued.has(e.text) || (!!e.message && queued.has(e.message.trim())));
  }

  #dropWhere(k: string, pred: (e: Outgoing) => boolean) {
    const list = this.outbox[k];
    if (!list) return;
    const keep = list.filter((e) => !pred(e));
    if (keep.length === list.length) return;
    if (keep.length) this.outbox[k] = keep;
    else delete this.outbox[k];
  }

  /** Safety net: accepted prompts still unconfirmed once the agent idled for a grace period go. */
  #sweepSoon(k: string) {
    clearTimeout(this.#outboxTimers.get(k));
    if (!this.outbox[k]?.some((e) => e.state === 'sent')) return;
    this.#outboxTimers.set(
      k,
      setTimeout(() => {
        this.#outboxTimers.delete(k);
        const status = this.agents[k]?.status;
        if (status !== 'idle' && status !== 'hibernated') return;
        const cutoff = Date.now() - OUTBOX_GRACE;
        this.#dropWhere(k, (e) => e.state === 'sent' && e.sentAt <= cutoff);
        this.#sweepSoon(k);
      }, OUTBOX_GRACE),
    );
  }

  /** The agent failed before confirming: open prompts become failed (Retry/Edit/Discard). */
  #failOpen(k: string, error: string | undefined) {
    for (const e of this.outbox[k] ?? []) {
      if (e.state === 'failed') continue;
      e.state = 'failed';
      e.error = `Agent failed${error ? `: ${parseError(error).title}` : ''}`;
    }
  }

  /** An agent's slash commands, fetched once; updates arrive as `agent.commands` events. */
  async loadCommands(host: string, agentId: string): Promise<SlashCommand[]> {
    const k = key(host, agentId);
    const cached = this.commands[k];
    if (cached) return cached;
    const list = await this.call('agents.commands', { id: agentId }, host, 30_000);
    this.commands[k] = list;
    return list;
  }

  /** Load a project's board; later changes arrive as `project.board` events. */
  async loadBoard(host: string, projectId: string): Promise<Board> {
    const k = key(host, projectId);
    const board = await this.call('boards.get', { projectId }, host);
    this.boards[k] = board;
    return board;
  }

  /** Load a project's schedules; later changes arrive as `project.schedules` events. */
  async loadSchedules(host: string, projectId: string): Promise<Schedule[]> {
    const list = await this.call('schedules.list', { projectId }, host);
    this.schedules[key(host, projectId)] = list;
    return list;
  }

  /** Load a project's secrets (without values); later changes arrive as `project.secrets` events. */
  async loadSecrets(host: string, projectId: string): Promise<SecretInfo[]> {
    const list = await this.call('secrets.list', { projectId }, host);
    this.secrets[key(host, projectId)] = list;
    return list;
  }

  /** Puts a schedule a call returned into its project's list (the `project.schedules` event follows). */
  putSchedule(host: string, s: Schedule) {
    const list = this.schedules[key(host, s.projectId)];
    if (!list) return;
    const i = list.findIndex((x) => x.id === s.id);
    if (i >= 0) list[i] = s;
    else list.push(s);
  }

  async loadModels(host: string): Promise<ModelInfo[]> {
    const cached = this.models[host];
    if (cached) return cached;
    const list = await this.call('agents.models', undefined, host);
    this.models[host] = list;
    return list;
  }

  /**
   * Data URL of a project's icon, fetched once per icon file version; `undefined` until loaded or
   * when it cannot be read. Safe to call while rendering (state is only written asynchronously).
   */
  projectIcon(p: Project): string | undefined {
    if (!p.icon) return undefined;
    const k = `${key(p.hostId, p.id)}:${p.icon.path}:${p.icon.modified}`;
    const cached = this.projectIcons[k];
    if (cached === undefined && !this.#iconLoads.has(k)) {
      this.#iconLoads.add(k);
      this.call('projects.icon', { id: p.id }, p.hostId)
        .then(({ mime, data }) => (this.projectIcons[k] = `data:${mime};base64,${data}`))
        .catch(() => {});
    }
    return cached;
  }

  /**
   * Data URL of a project's persona avatar, fetched once per persona version (any change of
   * `persona.toml`); `undefined` without one, until loaded or when it cannot be read.
   */
  personaAvatar(p: Project): string | undefined {
    if (!p.persona?.hasAvatar) return undefined;
    const k = `persona:${key(p.hostId, p.id)}:${JSON.stringify(p.persona)}`;
    const cached = this.projectIcons[k];
    if (cached === undefined && !this.#iconLoads.has(k)) {
      this.#iconLoads.add(k);
      this.call('projects.personaAvatar', { id: p.id }, p.hostId)
        .then(({ mime, data }) => (this.projectIcons[k] = `data:${mime};base64,${data}`))
        .catch(() => {});
    }
    return cached;
  }

  /**
   * A provider's quota on `host` with the windows `model` (id within the provider) draws from
   * marked `gating`; refetched once older than `maxAge` (the daemon caches 60 s per provider).
   */
  loadUsage(host: string, provider: string, model: string, maxAge = 60_000): Promise<ProviderUsage> {
    const k = key(host, `${provider}/${model}`);
    const cached = this.usage[k];
    if (cached && Date.now() - (this.#usageAt.get(k) ?? 0) < maxAge) return Promise.resolve(cached);
    let load = this.#usageLoads.get(k);
    if (!load) {
      load = this.call('agents.usage', { provider, model }, host, 40_000)
        .then((u) => {
          this.usage[k] = u;
          this.#usageAt.set(k, Date.now());
          return u;
        })
        .finally(() => this.#usageLoads.delete(k));
      this.#usageLoads.set(k, load);
    }
    return load;
  }

  /** Ref-counted `stats.watch`; views showing live stats hold a watch while visible. */
  watchStats(): () => void {
    this.#statsWatchers++;
    this.#syncStatsWatch();
    return () => {
      this.#statsWatchers--;
      this.#syncStatsWatch();
    };
  }

  // ─── Loading ────────────────────────────────────────────────────────────────────────────────

  async #hello() {
    try {
      const hello = await this.call('system.hello', { client: { id: clientId(), device: device() } });
      this.protocol = hello.protocol;
      this.homeId = hello.host.id;
      const hosts: Record<string, HostInfo> = {};
      for (const h of hello.hosts) hosts[h.id] = h;
      hosts[hello.host.id] = hello.host;
      this.hosts = hosts;
      for (const id of Object.keys(this.stats)) if (!hosts[id]) delete this.stats[id];
      await this.call('system.subscribe', { scope: 'all' }).catch(() => {});
      await Promise.all(hello.hosts.filter((h) => h.online).map((h) => this.#loadHost(h.id)));
      this.#pruneHosts();
      for (const tl of this.#timelines.values()) if (tl.loaded || tl.error) void tl.load();
      for (const k of Object.keys(this.details)) {
        const [host, id] = splitKey(k);
        void this.loadDetail(host, id);
      }
      this.ready = true;
      this.#syncStatsWatch();
    } catch (err) {
      console.error('hello failed', err);
    }
  }

  async #loadHost(host: string) {
    const [projects, agents, profiles, terminals] = await Promise.all([
      this.call('projects.list', undefined, host).catch(() => null),
      this.call('agents.list', undefined, host).catch(() => null),
      this.call('browser.profiles', undefined, host).catch(() => null),
      this.call('terminal.list', undefined, host).catch(() => null),
    ]);
    if (projects) this.#replace(this.projects, host, projects, (p) => p.id);
    if (agents) {
      this.#replace(this.agents, host, agents, (a) => a.id);
      for (const k of Object.keys(this.requests)) if (k.startsWith(`${host}:`) && !this.agents[k]) delete this.requests[k];
      const waiting = agents.filter((a) => a.pendingRequests > 0);
      await Promise.all(waiting.map((a) => this.loadDetail(host, a.id)));
      for (const a of agents) if (a.pendingRequests === 0) delete this.requests[key(host, a.id)];
      // Confirmations may have been missed while disconnected.
      for (const k of Object.keys(this.outbox)) if (k.startsWith(`${host}:`)) this.#sweepSoon(k);
    }
    if (profiles) this.#replace(this.profiles, host, profiles, (p) => p.id);
    if (terminals) this.#replace(this.terminals, host, terminals, (t) => t.id);
    this.call('stats.get', undefined, host)
      .then((s) => (this.stats[host] = s))
      .catch(() => {});
  }

  #replace<T>(store: Record<string, T>, host: string, list: T[], id: (v: T) => string) {
    const prefix = `${host}:`;
    const keep = new Set<string>();
    for (const v of list) {
      const k = prefix + id(v);
      keep.add(k);
      store[k] = v;
    }
    for (const k of Object.keys(store)) if (k.startsWith(prefix) && !keep.has(k)) delete store[k];
  }

  /** Drops data of hosts that are gone from the fleet. */
  #pruneHosts() {
    for (const store of [this.projects, this.agents, this.profiles, this.requests, this.details, this.terminals, this.outbox] as Record<string, unknown>[]) {
      for (const k of Object.keys(store)) if (!this.hosts[splitKey(k)[0]]) delete store[k];
    }
  }

  #syncStatsWatch() {
    if (this.conn !== 'open') return;
    // Nobody looks at a hidden tab: let every daemon stop sampling.
    const want = this.#statsWatchers > 0 && document.visibilityState === 'visible';
    for (const h of Object.values(this.hosts)) {
      const on = this.#watchingStats.has(h.id);
      if (want && h.online && !on) {
        this.#watchingStats.add(h.id);
        this.call('stats.watch', { enabled: true }, h.id).catch(() => this.#watchingStats.delete(h.id));
      } else if ((!want || !h.online) && on) {
        this.#watchingStats.delete(h.id);
        if (h.online) this.call('stats.watch', { enabled: false }, h.id).catch(() => {});
      }
    }
  }

  #scheduleDetail(host: string, id: string) {
    const k = key(host, id);
    if (!this.details[k]) return;
    clearTimeout(this.#detailTimers.get(k));
    this.#detailTimers.set(
      k,
      setTimeout(() => {
        this.#detailTimers.delete(k);
        void this.loadDetail(host, id);
      }, 400),
    );
  }

  // ─── Events ─────────────────────────────────────────────────────────────────────────────────

  #wire(c: Client) {
    c.on('host.updated', (h) => {
      const prev = this.hosts[h.id];
      this.hosts[h.id] = h;
      if (h.online && !prev?.online) void this.#loadHost(h.id);
      if (!h.online) delete this.stats[h.id];
      this.#syncStatsWatch();
    });
    c.on('host.removed', ({ id }) => {
      delete this.hosts[id];
      delete this.stats[id];
      this.#watchingStats.delete(id);
      if (this.hostFilter === id) this.hostFilter = null;
      this.#pruneHosts();
    });
    c.on('host.stats', (s, host) => {
      this.stats[s.hostId || host] = s;
    });
    c.on('project.updated', (p, host) => {
      this.projects[key(p.hostId || host, p.id)] = p;
    });
    c.on('project.removed', ({ id }, host) => {
      delete this.projects[key(host, id)];
    });
    c.on('agent.updated', (a, host) => {
      const h = a.hostId || host;
      const k = key(h, a.id);
      const prev = this.agents[k];
      this.agents[k] = a;
      if (prev && (prev.status === 'running' || prev.status === 'starting') && a.unread && !prev.unread) {
        const ok = a.status !== 'error';
        notify(ok ? 'Ready for review' : 'Agent failed', `${a.title}${a.preview ? ` — ${a.preview}` : ''}`, href({ name: 'agent', host: h, id: a.id }), k);
      }
      if (a.pendingRequests === 0 && this.requests[k]?.length) delete this.requests[k];
      if (a.status === 'error' && prev?.status !== 'error') this.#failOpen(k, a.error);
      else if (a.status === 'idle' || a.status === 'hibernated') this.#sweepSoon(k);
      this.#scheduleDetail(h, a.id);
    });
    c.on('agent.removed', ({ id }, host) => {
      const k = key(host, id);
      delete this.agents[k];
      delete this.requests[k];
      delete this.details[k];
      delete this.outbox[k];
      this.#seenUser.delete(k);
      this.#timelines.delete(k);
      delete this.images[k];
    });
    c.on('agent.timeline', ({ agentId, item }, host) => {
      const k = key(host, agentId);
      this.#timelines.get(k)?.upsert(item);
      this.#settleItem(k, item);
      this.#noteImages(k, item);
    });
    c.on('agent.delta', ({ agentId, itemId, text }, host) => {
      this.#timelines.get(key(host, agentId))?.delta(itemId, text);
    });
    c.on('agent.request', (r, host) => {
      const k = key(host, r.agentId);
      const list = this.requests[k];
      if (!list) this.requests[k] = [r];
      else if (!list.some((x) => x.id === r.id)) list.push(r);
      const a = this.agents[k];
      notify('Agent needs you', a ? a.title : 'An agent is waiting for input', href({ name: 'agent', host, id: r.agentId }), `req:${r.id}`);
    });
    c.on('agent.requestResolved', ({ agentId, requestId }, host) => {
      const k = key(host, agentId);
      const list = this.requests[k];
      if (!list) return;
      const i = list.findIndex((r) => r.id === requestId);
      if (i >= 0) list.splice(i, 1);
      if (!list.length) delete this.requests[k];
    });
    c.on('browser.updated', (p, host) => {
      this.profiles[key(host, p.id)] = p;
    });
    c.on('project.board', ({ projectId, board }, host) => {
      this.boards[key(host, projectId)] = board;
    });
    c.on('project.schedules', ({ projectId, schedules }, host) => {
      this.schedules[key(host, projectId)] = schedules;
    });
    c.on('project.secrets', ({ projectId, secrets }, host) => {
      this.secrets[key(host, projectId)] = secrets;
    });
    c.on('agent.commands', ({ agentId, commands }, host) => {
      this.commands[key(host, agentId)] = commands;
    });
    c.on('terminal.updated', (t, host) => {
      this.terminals[key(host, t.id)] = t;
    });
    c.on('terminal.removed', ({ id }, host) => {
      delete this.terminals[key(host, id)];
    });
    c.on('system.resync', () => void this.#hello());
  }
}

export function splitKey(k: string): [string, string] {
  const i = k.indexOf(':');
  return [k.slice(0, i), k.slice(i + 1)];
}

export const fleet = new Fleet();
