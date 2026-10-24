// Git status of an agent's directory, derived from its project's `git` summary (kept current by
// the daemon's .git watcher and after every turn): the one place that decides what the header
// chip, the warning bar above the composer and the agent rows show.

import { openExternal } from './external';
import { fleet, key } from './fleet.svelte';
import { ago } from './format';
import type { AgentSummary, CommitInfo, GitOperation, GithubInfo, MergeMethod, Project, PullRequestInfo, Worktree } from './protocol';
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
  /** Linked worktree whose changes are all in the base: no commits ahead, or the same changes (squash/rebase merged). */
  inBase: boolean;
  /** Clean linked worktree with nothing to merge: `inBase`, or its pull request was merged at the current HEAD. */
  merged: boolean;
  /** The worktree can be cleaned up: fully merged, or its pull request was merged (e.g. squashed). */
  cleanable: boolean;
  /** The upstream's remote branch was deleted (pruned by a fetch). */
  upstreamGone: boolean;
  /** Live agents working in the same worktree, this one included. */
  shared: AgentSummary[];
  fetchedAt?: number;
  severity: GitSeverity;
  /** The main worktree's branch: what a pull request targets by default. */
  base?: string;
  /** Commits not on the upstream (`wt.ahead`); absent without one. */
  unpushed?: number;
  /** Default remote for branches without upstream. */
  remote?: string;
  /** Set when that remote is on GitHub. */
  github?: GithubInfo;
  /** The branch's pull request (linked worktrees). */
  pr?: PullRequestInfo;
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
  const pr = kind === 'worktree' ? wt.pullRequest : undefined;
  const inBase = counted && (ahead === 0 || !!wt.changesInBase);
  const prMerged = pr?.state === 'merged' && !!pr.headSha && !!wt.head && pr.headSha.startsWith(wt.head);
  const merged = kind === 'worktree' && (inBase || prMerged) && dirty === 0 && !wt.operation;
  const upstreamGone = !!wt.upstream && !!wt.upstreamGone;
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
    inBase,
    merged,
    cleanable: merged || (pr?.state === 'merged' && dirty === 0 && !wt.operation),
    upstreamGone,
    shared,
    fetchedAt: git.fetchedAt,
    severity: conflicts > 0 || wt.operation || detached ? 'bad' : behind > 0 && !merged ? 'warn' : 'ok',
    base: base ?? undefined,
    unpushed: wt.upstream && !upstreamGone ? wt.ahead : undefined,
    remote: git.remote,
    github: git.github,
    pr,
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

/** The prompt sent by "Ask agent to update" after a push was rejected (non-fast-forward). */
export function rejectedPushPrompt(g: AgentGit): string {
  const target = pushTarget(g) ?? 'the remote branch';
  return (
    `Pushing \`${g.label}\` to \`${target}\` was rejected: the remote branch has commits yours does not. Fetch, then rebase onto ` +
    `(or merge) \`${target}\`. Resolve conflicts, run the checks, push again, and tell me what changed on the remote.`
  );
}

/** The prompt sent by "Let the agent write it" (instead of the pull request form). */
export function writePrPrompt(g: AgentGit): string {
  return (
    `Push your branch and open a pull request against \`${g.base ?? g.against ?? 'the default branch'}\` with \`gh pr create\`; ` +
    `write a title and a description of what changed and why, based on the card and your commits. Reply with the URL.`
  );
}

/** The prompt sent by "Ask agent to fix checks". */
export function fixChecksPrompt(pr: PullRequestInfo): string {
  const names = pr.checks.failing.map((c) => `\`${c.name}\``).join(', ');
  return (
    `Checks of pull request #${pr.number} (${pr.url}) failed${names ? `: ${names}` : ''}. Look at the failures ` +
    `(\`gh pr checks ${pr.number}\`, \`gh run view <run-id> --log-failed\`), fix them, run the checks locally where you can, ` +
    `commit and push, and tell me what was wrong.`
  );
}

// ─── push / pull request ─────────────────────────────────────────────────────────────────────

/** Agents that may still commit in their directory. */
export const isWorking = (a: AgentSummary) => a.status === 'starting' || a.status === 'running' || a.status === 'waiting';

/** Where a push goes: the upstream, else `<remote>/<branch>` (created by the push). */
export function pushTarget(g: AgentGit): string | undefined {
  if (g.wt.upstream) return g.wt.upstream;
  return g.remote && g.wt.branch ? `${g.remote}/${g.wt.branch}` : undefined;
}

/** Push is offered when there is a remote and something to send, or the branch was never pushed (or its remote branch was deleted and it has unmerged work). */
export function showPush(g: AgentGit): boolean {
  if (!g.wt.upstream && !g.remote) return false;
  if (g.upstreamGone) return !g.merged;
  return !g.wt.upstream || g.wt.ahead > 0;
}

/** Why push is unavailable, or `undefined` when it can run. */
export function pushBlocker(g: AgentGit): string | undefined {
  if (g.detached) return 'detached HEAD';
  if (!pushTarget(g)) return 'no remote';
  if (g.operation) return `a ${OPERATION_LABEL[g.operation]} is in progress`;
  return undefined;
}

/** "Push sends 3 commits to origin/x (new branch)." for worktree agents. */
export function pushHint(g: AgentGit): string | undefined {
  const target = pushTarget(g);
  if (!target) return undefined;
  if (g.upstreamGone) return `Push recreates ${target}, which was deleted on the remote.`;
  if (g.wt.upstream) return `Push sends ${plural(g.wt.ahead, 'commit')} to ${target}.`;
  if (g.kind === 'worktree' && g.ahead > 0) return `Push sends ${plural(g.ahead, 'commit')} to ${target} (new branch).`;
  return `Push creates ${target} (new branch).`;
}

/** Why the pull request row cannot be used (no PR features), or `undefined`. Push stays available. */
export function prBlocker(g: AgentGit): string | undefined {
  if (g.kind !== 'worktree') return 'only for worktree agents';
  if (!g.github) return g.remote ? `${g.remote} is not on GitHub` : 'no remote';
  if (g.github.gh === 'missing') return `gh not installed on ${fleet.host(g.project.hostId)?.name ?? g.project.hostId}`;
  if (g.github.gh === 'loggedOut') return 'gh not logged in';
  if (g.detached) return 'detached HEAD';
  return undefined;
}

/** Why merging is unavailable (comma-separated reasons), or `undefined` when GitHub reports it mergeable. */
export function mergeBlocker(pr: PullRequestInfo): string | undefined {
  if (pr.state !== 'open') return `the pull request is ${pr.state}`;
  if (pr.mergeState === 'CLEAN' || pr.mergeState === 'HAS_HOOKS') return undefined;
  const why: string[] = [];
  if (pr.draft || pr.mergeState === 'DRAFT') why.push('draft');
  if (pr.checks.failed) why.push(`${plural(pr.checks.failed, 'check')} failed`);
  if (pr.checks.pending) why.push(`waiting for ${plural(pr.checks.pending, 'running check')}`);
  if (pr.review === 'changesRequested') why.push('changes requested');
  else if (pr.review === 'reviewRequired') why.push('review required');
  if (pr.mergeState === 'DIRTY') why.push('merge conflict');
  if (pr.mergeState === 'BEHIND') why.push('branch behind base');
  if (pr.mergeState === 'UNKNOWN') why.push('mergeability unknown');
  if (!why.length) why.push(pr.mergeState === 'BLOCKED' ? 'blocked by repository rules' : `merge state ${pr.mergeState.toLowerCase()}`);
  return why.join(', ');
}

export type PrSeverity = 'ok' | 'pending' | 'bad' | 'draft' | 'merged' | 'closed';

/** Badge colour: green open & green, accent (neutral) checks running, red failed/conflict, grey draft, violet merged. */
export function prSeverity(pr: PullRequestInfo): PrSeverity {
  if (pr.state === 'merged') return 'merged';
  if (pr.state === 'closed') return 'closed';
  if (pr.draft) return 'draft';
  if (pr.checks.failed > 0 || pr.mergeState === 'DIRTY') return 'bad';
  if (pr.checks.pending > 0) return 'pending';
  return 'ok';
}

/** The mark after the number in a badge: `#42 ✓`. */
export const PR_MARK: Record<PrSeverity, string> = { ok: '✓', pending: '◷', bad: '✗', draft: 'draft', merged: 'merged', closed: '' };

/** One-line PR summary for tooltips. */
export function prSummary(pr: PullRequestInfo): string {
  const parts = [`Pull request #${pr.number}: ${pr.title}`, pr.draft && pr.state === 'open' ? 'draft' : pr.state];
  if (pr.state === 'open') {
    if (pr.checks.total) parts.push(`checks ${pr.checks.passed}/${pr.checks.total}`);
    if (pr.mergeState === 'DIRTY') parts.push('merge conflict');
  }
  return parts.join(' · ');
}

export const MERGE_METHODS: MergeMethod[] = ['squash', 'merge', 'rebase'];
export const MERGE_LABEL: Record<MergeMethod, string> = { squash: 'Squash and merge', merge: 'Create a merge commit', rebase: 'Rebase and merge' };

/** The repo's allowed merge methods in display order; all three while unknown. */
export function mergeMethods(github: GithubInfo | undefined): MergeMethod[] {
  const allowed = github?.mergeMethods ?? [];
  return allowed.length ? MERGE_METHODS.filter((m) => allowed.includes(m)) : MERGE_METHODS;
}

/** Section "Ziel"/"Goal" of a refinement: the text under that heading up to the next heading. */
export function goalSection(md: string): string | undefined {
  const heading = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/;
  let out: string[] | undefined;
  for (const line of md.split('\n')) {
    const h = heading.exec(line);
    if (h) {
      if (out) break;
      if (/^(ziel|goal)\b/i.test(h[1].replace(/[*_`]/g, '').trim())) out = [];
      continue;
    }
    out?.push(line);
  }
  return out?.join('\n').trim() || undefined;
}

export interface PrDraft {
  title: string;
  body: string;
}

/** Pull request form prefill: title from the card (else the oldest outgoing commit), body from card, goal and commits. */
export async function prDraft(a: AgentSummary, g: AgentGit): Promise<PrDraft> {
  const { hostId, id: projectId } = g.project;
  const [commits] = await Promise.all([
    fleet.call('git.outgoing', { projectId, path: g.wt.path }, hostId, 30_000).catch((err): CommitInfo[] => {
      ui.error('Could not list the commits', err);
      return [];
    }),
    fleet.boards[key(hostId, projectId)] ? undefined : fleet.loadBoard(hostId, projectId).catch((err) => ui.error('Could not load the board', err)),
  ]);
  // The card in Doing handed to this agent.
  const card = fleet.boards[key(hostId, projectId)]?.cards.find((c) => c.column === 'doing' && c.agentId === a.id);
  const parts: string[] = [];
  if (card) {
    parts.push(`Kanban: ${card.title} (${card.id})`);
    const body = card.body?.trim() ?? '';
    const about = (card.refinement && goalSection(card.refinement.text)) || (body.length > 600 ? `${body.slice(0, 600).trimEnd()}…` : body);
    if (about) parts.push(about);
  }
  // Oldest first, like the description Coocoo writes for `kanban_done`.
  if (commits.length) parts.push(['Commits:', ...[...commits].reverse().map((c) => `- ${c.sha.slice(0, 7)} ${c.summary}`)].join('\n'));
  return { title: card?.title ?? commits.at(-1)?.summary ?? g.label, body: parts.join('\n\n') };
}

/** In-flight work per worktree (`host:path`), shared by every chip showing it. */
export const pushing = $state<Record<string, boolean>>({});
export const prBusy = $state<Record<string, 'create' | 'merge'>>({});
export const prLoading = $state<Record<string, boolean>>({});
/** Why the last `git.prStatus` of a worktree failed. */
export const prErrors = $state<Record<string, string>>({});

export const wtKey = (g: AgentGit) => key(g.project.hostId, g.wt.path);

/** PR status older than this is refetched when the popover opens. */
export const PR_STALE_MS = 60_000;

/** Whether opening the popover should refresh the PR status. */
export function wantsPrStatus(g: AgentGit, now: number): boolean {
  return g.kind === 'worktree' && !!g.github && !g.detached && (!g.wt.prCheckedAt || now - g.wt.prCheckedAt > PR_STALE_MS);
}

/** Fire-and-forget: the result arrives with `project.updated`. */
export async function refreshPr(g: AgentGit): Promise<void> {
  const k = wtKey(g);
  if (prLoading[k]) return;
  prLoading[k] = true;
  try {
    await fleet.call('git.prStatus', { projectId: g.project.id, path: g.wt.path }, g.project.hostId, 60_000);
    delete prErrors[k];
  } catch (err) {
    prErrors[k] = err instanceof Error ? err.message : String(err);
  } finally {
    delete prLoading[k];
  }
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

/** Moves a linked worktree whose changes are all in the base onto the base (`git.resetToBase`). */
export async function resetToBase(g: AgentGit): Promise<void> {
  try {
    const r = await fleet.call('git.resetToBase', { projectId: g.project.id, path: g.wt.path }, g.project.hostId, 60_000);
    if (r.ok) ui.toast('success', `Reset ${g.label} to ${g.base ?? 'the base'}`);
    else ui.toast('error', 'Reset failed', r.output);
  } catch (err) {
    ui.error('Reset failed', err);
  }
}

/** Sends the agent a prompt (the "Ask agent to …" buttons). */
export async function askAgent(a: AgentSummary, message: string): Promise<void> {
  try {
    await fleet.call('agents.prompt', { id: a.id, message, mode: 'auto' }, a.hostId);
  } catch (err) {
    ui.error('Sending the prompt failed', err);
  }
}

export const askToUpdate = (a: AgentSummary, g: AgentGit, now: number) => askAgent(a, updatePrompt(g, now));

/** Push: worktree agents right away, the shared project checkout after the confirmation dialog. */
export function push(a: AgentSummary, g: AgentGit): void {
  if (g.kind === 'checkout') ui.dialog = { kind: 'push', hostId: a.hostId, agentId: a.id };
  else void pushNow(a, g);
}

export async function pushNow(a: AgentSummary, g: AgentGit): Promise<void> {
  const k = wtKey(g);
  if (pushing[k]) return;
  pushing[k] = true;
  const target = pushTarget(g);
  try {
    const r = await fleet.call('git.push', { projectId: g.project.id, path: g.wt.path }, g.project.hostId, 120_000);
    if (r.ok) ui.toast('success', `Pushed ${g.label}`, target ? `to ${target}` : undefined);
    else if (r.rejected)
      ui.toast(
        'error',
        'Push rejected',
        `${target ?? 'The remote branch'} has commits ${g.label} does not. Update the branch, then push again.`,
        a.status !== 'stopped' ? { label: 'Ask agent to update', run: () => askAgent(a, rejectedPushPrompt(g)) } : undefined,
      );
    else ui.toast('error', 'Push failed', r.output);
  } catch (err) {
    ui.error('Push failed', err);
  } finally {
    delete pushing[k];
  }
}

/** Pushes if needed and opens the pull request against the project checkout's branch; `undefined` on failure. */
export async function createPr(g: AgentGit, draft: PrDraft & { draft: boolean }): Promise<PullRequestInfo | undefined> {
  const k = wtKey(g);
  if (prBusy[k]) return undefined;
  prBusy[k] = 'create';
  try {
    const pr = await fleet.call(
      'git.prCreate',
      { projectId: g.project.id, path: g.wt.path, title: draft.title, body: draft.body, draft: draft.draft, base: g.base },
      g.project.hostId,
      120_000,
    );
    ui.toast('success', `Opened pull request #${pr.number}`, pr.title, { label: 'Open ↗', run: () => openExternal(pr.url) });
    return pr;
  } catch (err) {
    ui.error('Creating the pull request failed', err);
    return undefined;
  } finally {
    delete prBusy[k];
  }
}

export async function mergePr(g: AgentGit, pr: PullRequestInfo, method: MergeMethod, deleteBranch: boolean): Promise<void> {
  const k = wtKey(g);
  if (prBusy[k]) return;
  prBusy[k] = 'merge';
  try {
    const r = await fleet.call('git.prMerge', { projectId: g.project.id, path: g.wt.path, method, deleteBranch }, g.project.hostId, 120_000);
    if (r.ok) ui.toast('success', `Merged pull request #${pr.number}`, pr.title);
    else ui.toast('error', `Merging #${pr.number} failed`, r.output);
  } catch (err) {
    ui.error(`Merging #${pr.number} failed`, err);
  } finally {
    delete prBusy[k];
  }
}

// ─── merge method per project ────────────────────────────────────────────────────────────────

const MERGE_KEY = 'coocoo.mergeMethod';

/** Project (`host:id`) → last merge method used. */
const lastMergeMethod = $state<Record<string, MergeMethod>>(
  (() => {
    try {
      const v = JSON.parse(localStorage.getItem(MERGE_KEY) ?? '{}');
      return v && typeof v === 'object' ? v : {};
    } catch {
      return {};
    }
  })(),
);

/** The method last used in the project if still allowed, else squash (or the first allowed). */
export function mergeMethodFor(project: Project, allowed: MergeMethod[]): MergeMethod {
  const last = lastMergeMethod[key(project.hostId, project.id)];
  if (last && allowed.includes(last)) return last;
  return allowed.includes('squash') ? 'squash' : (allowed[0] ?? 'squash');
}

/** Remembers the method picked in the merge split button for the project. */
export function rememberMergeMethod(project: Project, method: MergeMethod) {
  lastMergeMethod[key(project.hostId, project.id)] = method;
  localStorage.setItem(MERGE_KEY, JSON.stringify(lastMergeMethod));
}

/** Why fast-forward is unavailable, or `undefined` when it can run. */
export function fastForwardBlocker(g: AgentGit): string | undefined {
  if (!g.wt.upstream) return 'the branch has no upstream';
  if (g.wt.ahead > 0) return `${plural(g.wt.ahead, 'local commit')} not on ${g.wt.upstream}`;
  if (g.dirty > 0) return `${plural(g.dirty, 'uncommitted file')}`;
  if (g.operation) return `a ${OPERATION_LABEL[g.operation]} is in progress`;
  const busy = g.shared.filter(isWorking);
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
