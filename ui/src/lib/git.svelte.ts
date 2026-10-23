// Git status of an agent's directory, derived from its project's `git` summary (kept current by
// the daemon's .git watcher and after every turn): the one place that decides what the header
// chip, the warning bar above the composer and the agent rows show.

import { fleet, key } from './fleet.svelte';
import { ago } from './format';
import type { AgentSummary, GitOperation, Project, Worktree } from './protocol';
import { ui } from './ui.svelte';

export type GitSeverity = 'ok' | 'warn' | 'bad';

export interface AgentGit {
  project: Project;
  wt: Worktree;
  /** `checkout`: the project's main worktree; `worktree`: a linked one. */
  kind: 'checkout' | 'worktree';
  /** Branch name, or the short sha on a detached HEAD. */
  label: string;
  detached: boolean;
  /** What `ahead`/`behind` count against: the upstream (checkout) or the base branch (worktree). */
  against?: string;
  ahead: number;
  behind: number;
  dirty: number;
  conflicts: number;
  operation?: GitOperation;
  /** Linked worktree with nothing beyond its base and a clean tree. */
  merged: boolean;
  /** Live agents working in the same worktree, this one included. */
  shared: AgentSummary[];
  fetchedAt?: number;
  severity: GitSeverity;
}

export const OPERATION_LABEL: Record<GitOperation, string> = {
  merge: 'merge',
  rebase: 'rebase',
  cherryPick: 'cherry-pick',
  revert: 'revert',
  bisect: 'bisect',
};

/** A fetch older than this is flagged: the behind counts may be far off. */
export const STALE_FETCH_MS = 24 * 3600_000;

/** The worktree containing `cwd` (longest matching path: a cwd may be a subdirectory). */
export function worktreeOf(project: Project | undefined, cwd: string): Worktree | undefined {
  let best: Worktree | undefined;
  for (const wt of project?.git?.worktrees ?? []) {
    const inside = cwd === wt.path || cwd.startsWith(wt.path.endsWith('/') ? wt.path : `${wt.path}/`);
    if (inside && (!best || wt.path.length > best.path.length)) best = wt;
  }
  return best;
}

export function agentGit(a: AgentSummary, project: Project | undefined, agents: AgentSummary[]): AgentGit | undefined {
  const git = project?.git;
  const wt = worktreeOf(project, a.cwd);
  if (!project || !git || !wt) return undefined;
  const kind = wt.isMain ? 'checkout' : 'worktree';
  const detached = !wt.branch;
  const base = git.worktrees.find((w) => w.isMain)?.branch ?? undefined;
  const counted = kind === 'worktree' && wt.baseAhead != null;
  const ahead = (counted ? wt.baseAhead : wt.ahead) ?? 0;
  const behind = (counted ? wt.baseBehind : wt.behind) ?? 0;
  const dirty = wt.changed ?? 0;
  const conflicts = wt.conflicts ?? 0;
  const shared = agents.filter(
    (x) => x.hostId === a.hostId && x.projectId === a.projectId && x.status !== 'stopped' && worktreeOf(project, x.cwd)?.path === wt.path,
  );
  if (!shared.some((x) => x.id === a.id)) shared.unshift(a);
  return {
    project,
    wt,
    kind,
    label: wt.branch ?? wt.head ?? 'detached',
    detached,
    against: counted ? (base ?? undefined) : (wt.upstream ?? undefined),
    ahead,
    behind,
    dirty,
    conflicts,
    operation: wt.operation,
    merged: counted && ahead === 0 && dirty === 0 && !wt.operation,
    shared,
    fetchedAt: git.fetchedAt,
    severity: conflicts > 0 || wt.operation || detached ? 'bad' : behind > 0 ? 'warn' : 'ok',
  };
}

/** Problems worth a bar above the composer: a stale checkout, conflicts, operations, detached. */
export function needsBar(g: AgentGit): boolean {
  return g.severity === 'bad' || (g.kind === 'checkout' && g.behind > 0);
}

/** Fetch freshness for display: `fetched 3h ago` / `fetched just now` / `never fetched`. */
export function fetchedLabel(at: number | undefined, now: number): string {
  if (!at) return 'never fetched';
  const rel = ago(at, now);
  return rel === 'now' ? 'fetched just now' : `fetched ${rel} ago`;
}

export const isStale = (at: number | undefined, now: number) => !at || now - at > STALE_FETCH_MS;

export const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** The prompt sent by "Ask agent to update/resolve". */
export function updatePrompt(g: AgentGit, now: number): string {
  const where = g.kind === 'checkout' ? `Your checkout (\`${g.label}\`)` : `Your branch (\`${g.label}\`)`;
  if (g.operation || g.conflicts) {
    const what = g.operation ? `has a ${OPERATION_LABEL[g.operation]} in progress` : 'has unresolved conflicts';
    const conflicts = g.conflicts ? ` with ${plural(g.conflicts, 'conflicted file')}` : '';
    const finish = g.operation ? `Finish or abort the ${OPERATION_LABEL[g.operation]} in progress` : 'Resolve them';
    const resolve = g.operation && g.conflicts ? `, resolve the ${plural(g.conflicts, 'conflict')}` : '';
    return `${where} ${what}${conflicts}. ${finish}${resolve}, run the checks, and tell me where things stand.`;
  }
  if (g.detached) {
    return `${where} is on a detached HEAD. Get back onto a branch (create one if you made commits here) and tell me what you did.`;
  }
  const against = g.against ?? 'its base';
  if (g.kind === 'checkout') {
    const fetched = g.fetchedAt ? `as of the last fetch: ${fetchedLabel(g.fetchedAt, now)}` : 'as far as is known without a fetch';
    return (
      `${where} is ${plural(g.behind, 'commit')} behind \`${against}\` (${fetched}). Fetch, then bring it up to date: ` +
      `fast-forward if you have no local work, otherwise commit your work and rebase onto (or merge) \`${against}\`. ` +
      `Resolve conflicts, run the checks, and tell me what changed upstream that affects your task.`
    );
  }
  return (
    `${where} is ${plural(g.behind, 'commit')} behind \`${against}\`. Bring it up to date: commit your work, then rebase onto ` +
    `(or merge) \`${against}\`. Resolve conflicts, run the checks, and tell me what changed on \`${against}\` that affects your task.`
  );
}

// ─── actions ─────────────────────────────────────────────────────────────────────────────────

/** Projects (`host:id`) with a fetch in flight, shared by every chip and bar showing them. */
export const fetching = $state<Record<string, boolean>>({});

export async function fetchProject(project: Project): Promise<void> {
  const k = key(project.hostId, project.id);
  if (fetching[k]) return;
  fetching[k] = true;
  try {
    const r = await fleet.call('git.fetch', { projectId: project.id }, project.hostId, 75_000);
    if (!r.ok) ui.toast('error', `Fetch failed: ${project.name}`, r.output);
  } catch (err) {
    ui.error(`Fetch failed: ${project.name}`, err);
  } finally {
    delete fetching[k];
  }
}

export async function fastForward(g: AgentGit): Promise<void> {
  try {
    const r = await fleet.call('git.fastForward', { projectId: g.project.id, path: g.wt.path }, g.project.hostId, 60_000);
    if (r.ok) ui.toast('success', `Fast-forwarded ${g.label}`, g.against ? `Now at ${g.against}` : undefined);
    else ui.toast('error', 'Fast-forward failed', r.output);
  } catch (err) {
    ui.error('Fast-forward failed', err);
  }
}

export async function askToUpdate(a: AgentSummary, g: AgentGit, now: number): Promise<void> {
  try {
    await fleet.call('agents.prompt', { id: a.id, message: updatePrompt(g, now), mode: 'auto' }, a.hostId);
  } catch (err) {
    ui.error('Sending the prompt failed', err);
  }
}

/** Why fast-forward is unavailable, or `undefined` when it can run. */
export function fastForwardBlocker(g: AgentGit): string | undefined {
  if (!g.wt.upstream) return 'the branch has no upstream';
  if (g.wt.ahead > 0) return `${plural(g.wt.ahead, 'local commit')} not on ${g.wt.upstream}`;
  if (g.dirty > 0) return `${plural(g.dirty, 'uncommitted file')}`;
  if (g.operation) return `a ${OPERATION_LABEL[g.operation]} is in progress`;
  const busy = g.shared.filter((x) => x.status === 'starting' || x.status === 'running' || x.status === 'waiting');
  if (busy.length) return busy.length === 1 ? `${busy[0].title} is working in this checkout` : `${busy.length} agents are working in this checkout`;
  return undefined;
}

// ─── dismissing the bar ──────────────────────────────────────────────────────────────────────

const DISMISS_KEY = 'coocoo.gitbar';

function loadDismissed(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(DISMISS_KEY) ?? '{}');
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

/** Agent (`host:id`) → state signature the user dismissed the bar for. */
const dismissed = $state<Record<string, string>>(loadDismissed());

/** The bar comes back once the state it was dismissed for changes. */
const signature = (g: AgentGit) => [g.kind, g.label, g.wt.head, g.behind, g.conflicts, g.operation ?? ''].join('|');

export function isDismissed(a: AgentSummary, g: AgentGit): boolean {
  return dismissed[key(a.hostId, a.id)] === signature(g);
}

export function dismiss(a: AgentSummary, g: AgentGit) {
  dismissed[key(a.hostId, a.id)] = signature(g);
  localStorage.setItem(DISMISS_KEY, JSON.stringify(dismissed));
}
