// Number formatting for the Stats and Workflows tabs.

/** `$995.79`, `$12.4k`; API-equivalent dollars. */
export function usd(n: number | undefined): string {
  if (n == null) return '—';
  if (n === 0) return '$0';
  if (n < 0.01) return '<$0.01';
  if (n < 10_000) return `$${n.toFixed(2)}`;
  return `$${(n / 1000).toFixed(1)}k`;
}

/** `950`, `12.4k`, `25.1M`, `2.09B`. */
export function count(n: number | undefined): string {
  if (n == null) return '—';
  const abs = Math.abs(n);
  if (abs < 1000) return String(Math.round(n));
  if (abs < 1e6) return `${(n / 1e3).toFixed(abs < 1e4 ? 1 : 0)}k`;
  if (abs < 1e9) return `${(n / 1e6).toFixed(abs < 1e7 ? 2 : 1)}M`;
  return `${(n / 1e9).toFixed(2)}B`;
}

/** `0.42` → `42%` (one decimal below 10 %). */
export function pct(ratio: number | undefined, digits?: number): string {
  if (ratio == null || !Number.isFinite(ratio)) return '—';
  const v = ratio * 100;
  return `${v.toFixed(digits ?? (v !== 0 && Math.abs(v) < 10 ? 1 : 0))}%`;
}

/** `45s`, `12m`, `1h 05m`, `2d 4h`. */
export function span(ms: number | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return '—';
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 && m < 10 ? `${m}m ${String(s % 60).padStart(2, '0')}s` : `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

export interface Delta {
  text: string;
  /** `good`/`bad` colour the arrow; `flat` for no change or no comparison. */
  tone: 'good' | 'bad' | 'flat';
}

/**
 * Change against the previous period: `▲ 18%`. `higherIsBetter` decides the colour
 * (more cost is not "good", more commits is).
 */
export function delta(cur: number, prev: number | undefined, higherIsBetter = true): Delta | null {
  if (prev == null) return null;
  if (prev === 0) return cur === 0 ? { text: '±0', tone: 'flat' } : { text: 'new', tone: 'flat' };
  const r = (cur - prev) / prev;
  if (Math.abs(r) < 0.005) return { text: '±0%', tone: 'flat' };
  const up = r > 0;
  return { text: `${up ? '▲' : '▼'} ${pct(Math.abs(r))}`, tone: up === higherIsBetter ? 'good' : 'bad' };
}

/** `Oct 3`. */
export function shortDate(ts: number): string {
  return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** Stable colour per model name (chart stacks, legends, table dots). */
const PALETTE = ['#6b74ff', '#a78bfa', '#34d399', '#f5b74a', '#f472b6', '#38bdf8', '#fb923c', '#94a3b8'];
const assigned = new Map<string, string>();
export function modelColor(model: string, order: string[] = []): string {
  const i = order.indexOf(model);
  if (i >= 0) return PALETTE[i % PALETTE.length];
  let c = assigned.get(model);
  if (!c) {
    let h = 0;
    for (const ch of model) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    c = PALETTE[h % PALETTE.length];
    assigned.set(model, c);
  }
  return c;
}
