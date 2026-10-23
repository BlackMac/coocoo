// Helpers of the Stats tab: names for ids in a usage report (the live fleet knows current names;
// offline hosts, deleted projects and deleted agents only survive in the report) and chart scales.

import { fleet } from '../../lib/fleet.svelte';
import type { UsageReport } from '../../lib/protocol';

export function hostName(r: UsageReport, hostId: string): string {
  return fleet.host(hostId)?.name ?? r.hosts.find((h) => h.hostId === hostId)?.name ?? hostId;
}

export function projectName(r: UsageReport, hostId: string, projectId: string | undefined): string {
  if (!projectId) return '—';
  return fleet.project(hostId, projectId)?.name ?? r.projects.find((p) => p.hostId === hostId && p.projectId === projectId)?.name ?? projectId;
}

export function agentTitle(r: UsageReport, hostId: string, agentId: string): string {
  return r.agents.find((a) => a.hostId === hostId && a.id === agentId)?.title ?? fleet.agent(hostId, agentId)?.title ?? 'Agent';
}

/** Model names in a stable order for colours: the same model keeps its colour across filters. */
export function colorOrder(r: UsageReport): string[] {
  const names = [...r.allModels];
  for (const m of r.models) if (!names.includes(m.model)) names.push(m.model);
  return names;
}

/** `2.4s` / `850ms`. */
export function seconds(ms: number | undefined): string {
  if (ms == null) return '—';
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
}

/** Axis maximum ≥ `v` that splits into three round steps. */
export function niceMax(v: number, integer = false): number {
  if (!(v > 0)) return integer ? 3 : 1;
  const raw = v / 3;
  const p = 10 ** Math.floor(Math.log10(raw));
  let step = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw) ?? 10 * p;
  if (integer) step = Math.max(1, Math.ceil(step));
  return step * 3;
}

/** Every how many bars a date label fits under a chart. */
export function labelEvery(barWidth: number, labelWidth = 46): number {
  return Math.max(1, Math.ceil(labelWidth / Math.max(barWidth, 0.1)));
}
