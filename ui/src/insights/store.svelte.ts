// Shared filter state and cached reports of the Stats and Workflows tabs.
//
// Both tabs use the same filter (period, project, model, a clicked day, and the sidebar's host
// filter); every section of a tab reads one report fetched for that filter, so a filter chip set
// anywhere narrows all of them. Reports are cached per filter for 60 s (the daemon syncs omp's
// statistics at most once a minute as well).

import { fleet } from '../lib/fleet.svelte';
import type { InsightsQuery, UsageReport, WorkflowReport } from '../lib/protocol';

export type RangeId = 'today' | '7d' | '30d' | '90d' | 'all';

export const RANGES: { id: RangeId; label: string; days: number }[] = [
  { id: 'today', label: 'Today', days: 1 },
  { id: '7d', label: '7d', days: 7 },
  { id: '30d', label: '30d', days: 30 },
  { id: '90d', label: '90d', days: 90 },
  { id: 'all', label: 'All', days: 0 },
];

const CACHE_MS = 60_000;
/** omp's statistics may need a sync first; the daemon waits up to 60 s for it. */
const TIMEOUT_MS = 90_000;
const RANGE_KEY = 'coocoo.insights.range';

const DAY = 86_400_000;

/** Local midnight of the day containing `ts`. */
export function midnight(ts: number): number {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Local midnight `n` days after the one of `ts` (DST-safe). */
export function addDays(ts: number, n: number): number {
  const d = new Date(midnight(ts));
  d.setDate(d.getDate() + n);
  return d.getTime();
}

interface Cached<T> {
  key: string;
  at: number;
  data?: T;
  error?: string;
  loading: boolean;
}

class InsightsStore {
  range = $state<RangeId>((localStorage.getItem(RANGE_KEY) as RangeId | null) ?? '30d');
  /** Filter chips. */
  project = $state<{ hostId: string; id: string; name: string } | null>(null);
  model = $state<string | null>(null);
  /** A day picked in a chart (local midnight); overrides the period. */
  day = $state<number | null>(null);

  usage = $state<Cached<UsageReport>>({ key: '', at: 0, loading: false });
  workflows = $state<Cached<WorkflowReport>>({ key: '', at: 0, loading: false });

  /** The query for the current filter. */
  get query(): InsightsQuery {
    const today = midnight(Date.now());
    const to = this.day !== null ? addDays(this.day, 1) : addDays(today, 1);
    let from: number;
    if (this.day !== null) from = this.day;
    else {
      const days = RANGES.find((r) => r.id === this.range)?.days ?? 30;
      from = days ? addDays(today, 1 - days) : 0;
    }
    const q: InsightsQuery = { from, to, tzOffset: -new Date(from || Date.now()).getTimezoneOffset() };
    if (this.project) q.project = this.project.id;
    if (this.model) q.model = this.model;
    if (fleet.hostFilter) q.hosts = [fleet.hostFilter];
    else if (this.project) q.hosts = [this.project.hostId];
    if (this.range === 'all' && this.day === null) q.compare = false;
    return q;
  }

  /** Length of the period in days, for "vs. previous …" labels. */
  get periodLabel(): string {
    if (this.day !== null) return 'previous day';
    const r = RANGES.find((x) => x.id === this.range);
    return r?.id === 'today' ? 'yesterday' : `prev. ${r?.label ?? ''}`;
  }

  setRange(id: RangeId) {
    this.range = id;
    this.day = null;
    localStorage.setItem(RANGE_KEY, id);
  }

  clearFilters() {
    this.project = null;
    this.model = null;
    this.day = null;
  }

  get hasFilters(): boolean {
    return !!(this.project || this.model || this.day !== null);
  }

  loadUsage(force = false): void {
    void this.#load('insights.usage', this.usage, (c) => (this.usage = c), force);
  }

  loadWorkflows(force = false): void {
    void this.#load('insights.workflows', this.workflows, (c) => (this.workflows = c), force);
  }

  async #load<M extends 'insights.usage' | 'insights.workflows'>(
    method: M,
    current: Cached<M extends 'insights.usage' ? UsageReport : WorkflowReport>,
    set: (c: Cached<M extends 'insights.usage' ? UsageReport : WorkflowReport>) => void,
    force: boolean,
  ) {
    const query = this.query;
    const key = JSON.stringify(query);
    if (!force && current.key === key && (current.loading || Date.now() - current.at < CACHE_MS)) return;
    set({ ...current, key, loading: true, error: undefined });
    try {
      const data = (await fleet.call(method, query, undefined, TIMEOUT_MS)) as M extends 'insights.usage' ? UsageReport : WorkflowReport;
      if (JSON.stringify(this.query) !== key) return;
      set({ key, at: Date.now(), data, loading: false });
    } catch (err) {
      if (JSON.stringify(this.query) !== key) return;
      set({ key, at: Date.now(), data: current.key === key ? current.data : undefined, loading: false, error: (err as Error).message });
    }
  }
}

export const insights = new InsightsStore();

export { DAY };
