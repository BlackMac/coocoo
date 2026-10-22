// Small display formatters shared across views.

const UNITS: [number, string][] = [
  [60, 's'],
  [60, 'm'],
  [24, 'h'],
  [7, 'd'],
  [4.35, 'w'],
  [12, 'mo'],
  [Infinity, 'y'],
];

/** Compact relative time: `now`, `42s`, `5m`, `3h`, `2d`. */
export function ago(ts: number, now = Date.now()): string {
  let v = Math.max(0, (now - ts) / 1000);
  if (v < 5) return 'now';
  for (const [step, unit] of UNITS) {
    if (v < step) return `${Math.floor(v)}${unit}`;
    v /= step;
  }
  return '';
}

/** Compact countdown: `12m`, `3h 20m`, `2d 4h`; `now` once reached. */
export function until(ts: number, now = Date.now()): string {
  const m = Math.floor((ts - now) / 60_000);
  if (m < 1) return 'now';
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  return h % 24 ? `${Math.floor(h / 24)}d ${h % 24}h` : `${Math.floor(h / 24)}d`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function cost(usd: number | undefined): string {
  if (usd == null) return '—';
  if (usd < 0.01) return usd === 0 ? '$0' : '<$0.01';
  return usd < 10 ? `$${usd.toFixed(2)}` : `$${usd.toFixed(1)}`;
}

export function compact(n: number | undefined): string {
  if (n == null) return '—';
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

export function duration(ms: number | undefined): string {
  if (ms == null) return '';
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

export function dateTime(ts: number): string {
  return new Date(ts).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** Last path segment(s) for dense display: `/a/b/c/d` → `…/c/d`. */
export function shortPath(path: string, keep = 2): string {
  const parts = path.split('/').filter(Boolean);
  if (parts.length <= keep) return path;
  return `…/${parts.slice(-keep).join('/')}`;
}

/** `provider/model-id` → `model-id`. */
export function modelName(model: string | undefined): string {
  if (!model) return '—';
  const i = model.indexOf('/');
  return i < 0 ? model : model.slice(i + 1);
}
