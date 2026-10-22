// Fleet store: every host, project, agent, pending request and browser profile the home daemon
// can see, kept current from events. Entities from peers are keyed `${hostId}:${id}`.

import { Client, type ConnState } from './client';
import { notify } from './notify';
import type {
  AgentDetail,
  AgentSummary,
  BrowserProfileInfo,
  HostInfo,
  HostStats,
  Method,
  ModelInfo,
  ParamsOf,
  Project,
  ProviderUsage,
  ResultOf,
  TimelineItem,
  UiRequest,
} from './protocol';
import { href } from './router.svelte';
import { agentState } from './status';

export const key = (host: string, id: string) => `${host}:${id}`;

const TIMELINE_PAGE = 120;
const TIMELINE_CACHE = 8;

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
    if (item.seq < this.items[0].seq) return; // older than the loaded window; paging will fetch it
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
  native = false;
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
  /** `agents.usage` per `host:provider`. */
  usage = $state<Record<string, ProviderUsage>>({});
  /** Local receipt time per `usage` key (hosts' clocks may differ). */
  #usageAt = new Map<string, number>();
  #usageLoads = new Map<string, Promise<ProviderUsage>>();
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
    } catch (err) {
      console.warn('agents.get failed', err);
    }
  }

  async loadModels(host: string): Promise<ModelInfo[]> {
    const cached = this.models[host];
    if (cached) return cached;
    const list = await this.call('agents.models', undefined, host);
    this.models[host] = list;
    return list;
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
      const hello = await this.call('system.hello');
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
    const [projects, agents, profiles] = await Promise.all([
      this.call('projects.list', undefined, host).catch(() => null),
      this.call('agents.list', undefined, host).catch(() => null),
      this.call('browser.profiles', undefined, host).catch(() => null),
    ]);
    if (projects) this.#replace(this.projects, host, projects, (p) => p.id);
    if (agents) {
      this.#replace(this.agents, host, agents, (a) => a.id);
      for (const k of Object.keys(this.requests)) if (k.startsWith(`${host}:`) && !this.agents[k]) delete this.requests[k];
      const waiting = agents.filter((a) => a.pendingRequests > 0);
      await Promise.all(waiting.map((a) => this.loadDetail(host, a.id)));
      for (const a of agents) if (a.pendingRequests === 0) delete this.requests[key(host, a.id)];
    }
    if (profiles) this.#replace(this.profiles, host, profiles, (p) => p.id);
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
    for (const store of [this.projects, this.agents, this.profiles, this.requests, this.details] as Record<string, unknown>[]) {
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
      this.#scheduleDetail(h, a.id);
    });
    c.on('agent.removed', ({ id }, host) => {
      const k = key(host, id);
      delete this.agents[k];
      delete this.requests[k];
      delete this.details[k];
      this.#timelines.delete(k);
    });
    c.on('agent.timeline', ({ agentId, item }, host) => {
      this.#timelines.get(key(host, agentId))?.upsert(item);
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
    c.on('system.resync', () => void this.#hello());
  }
}

export function splitKey(k: string): [string, string] {
  const i = k.indexOf(':');
  return [k.slice(0, i), k.slice(i + 1)];
}

export const fleet = new Fleet();
