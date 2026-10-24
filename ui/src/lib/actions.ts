// Project actions from `.coocoo/actions.toml`: which ones a view offers, how they are shown, and
// starting them. `run` actions go to the daemon (`actions.run`, a terminal per run); prompts and
// URLs are carried out here with the existing agent and browser methods.

import type { Component } from 'svelte';
import { fleet, key } from './fleet.svelte';
import { openExternal } from './external';
import { worktreeOf } from './git.svelte';
import { Bug, Database, ExternalLink, Globe, MessageSquareText, Package, Play, RefreshCw, Rocket, Server, Terminal, TestTube, Wrench } from './icons';
import type { AgentSummary, Project, ProjectAction, TerminalAction, TerminalInfo } from './protocol';
import { router } from './router.svelte';
import { ui } from './ui.svelte';

type Icon = Component<{ size?: number }>;

/** The names `icon` may use (the daemon drops any other). */
const ICONS: Record<string, Icon> = {
  play: Play,
  server: Server,
  'test-tube': TestTube,
  terminal: Terminal,
  rocket: Rocket,
  globe: Globe,
  bug: Bug,
  package: Package,
  database: Database,
  wrench: Wrench,
  refresh: RefreshCw,
  message: MessageSquareText,
  'external-link': ExternalLink,
};

/** Where actions are offered: an agent's view (they run in its directory) or the project view. */
export interface ActionContext {
  project: Project;
  agent?: AgentSummary;
}

export function actionIcon(a: ProjectAction): Icon {
  return (a.icon && ICONS[a.icon]) || (a.kind === 'prompt' ? MessageSquareText : a.kind === 'url' ? ExternalLink : Play);
}

/** The actions of the context's scope, in file order. */
export function actionsFor(ctx: ActionContext): ProjectAction[] {
  const want = ctx.agent ? 'agent' : 'project';
  return (ctx.project.actions ?? []).filter((a) => a.scope === 'both' || a.scope === want);
}

/** The Run button's one-click action. */
export function defaultAction(list: ProjectAction[]): ProjectAction | undefined {
  return list.find((a) => a.default) ?? list[0];
}

/** Directory the context's commands run in (before an action's own `cwd`). */
export function baseDir(ctx: ActionContext): string {
  return ctx.agent?.cwd ?? ctx.project.path;
}

/** The current (or last) run of a `run` action in this context. */
export function actionRun(ctx: ActionContext, a: ProjectAction): TerminalInfo | undefined {
  return a.kind === 'run' ? fleet.actionRun(ctx.project.hostId, ctx.project.id, a.id, baseDir(ctx)) : undefined;
}

const LOOPBACK = /^(localhost|127(?:\.\d+){3}|\[::1\]|.+\.localhost)$/i;

/** Whether the URL points at the machine it is opened on (for us: the host the code runs on). */
export function isLoopbackUrl(url: string): boolean {
  try {
    return LOOPBACK.test(new URL(url).hostname);
  } catch {
    return false;
  }
}

/**
 * Loopback URLs point at the host where the code runs, so in an agent's view they open in the
 * project's browser (reachable from any client); everything else opens outside.
 */
export function urlTarget(a: Extract<ProjectAction, { kind: 'url' }>, ctx: ActionContext): 'browser' | 'external' {
  if (!ctx.agent) return 'external';
  if (a.target) return a.target;
  return isLoopbackUrl(a.url) ? 'browser' : 'external';
}

/** Opens a page in a browser profile on `host` and shows it in the agent's browser panel. */
export async function openInBrowserPanel(host: string, profile: string, url: string): Promise<void> {
  const tab = await fleet.call('browser.open', { profile, url }, host, 45_000);
  ui.browserFocus = { host, profile, targetId: tab.targetId };
  ui.showPanel('browser');
}

/** One line under the action's name in the menu. */
export function actionDetail(a: ProjectAction, ctx: ActionContext): string {
  switch (a.kind) {
    case 'run':
      return a.cwd ? `${a.command}  · in ${a.cwd}` : a.command;
    case 'prompt':
      return ctx.agent ? 'Prompt → this agent' : a.worktree ? 'Prompt → new agent in a worktree' : 'Prompt → new agent';
    case 'url':
      return `${a.url.replace(/^https?:\/\//, '')} → ${urlTarget(a, ctx) === 'browser' ? 'browser panel' : 'external'}`;
  }
}

/** `{branch}`, `{project}`, `{cwd}` in prompts; unknown placeholders stay as written. */
export function fillPlaceholders(text: string, ctx: ActionContext): string {
  const cwd = baseDir(ctx);
  const wt = worktreeOf(ctx.project, cwd);
  const branch = wt?.branch ?? wt?.head ?? ctx.project.git?.branch ?? '';
  return text.replace(/\{(branch|project|cwd)\}/g, (_, name: string) => (name === 'branch' ? branch : name === 'project' ? ctx.project.name : cwd));
}

/** Whether the action can start in this context right now. */
export function actionDisabled(a: ProjectAction, ctx: ActionContext): string | undefined {
  if (!fleet.host(ctx.project.hostId)?.online) return 'Host offline';
  if (a.kind === 'prompt' && ctx.agent?.status === 'stopped') return 'The agent is archived';
  return undefined;
}

/** Start an action, after its confirmation question if it has one. */
export function startAction(a: ProjectAction, ctx: ActionContext): void {
  if (a.confirm) {
    ui.dialog = { kind: 'confirm', title: a.name, message: a.confirm, action: 'Run', run: () => launch(a, ctx) };
    return;
  }
  void launch(a, ctx);
}

/** Run a `run` action again (the daemon replaces the earlier run), e.g. from its terminal. */
export function restartRun(host: string, run: TerminalAction): void {
  const project = fleet.project(host, run.projectId);
  const action = project?.actions?.find((a) => a.id === run.actionId);
  if (!project || !action) {
    ui.toast('error', `${run.name} is no longer in .coocoo/actions.toml`);
    return;
  }
  const start = () => runCommand(host, project.id, action.id, run.base, false);
  if (action.confirm) ui.dialog = { kind: 'confirm', title: action.name, message: action.confirm, action: 'Run', run: start };
  else void start();
}

export function stopRun(host: string, t: TerminalInfo): void {
  fleet.call('terminal.kill', { id: t.id }, host).catch((err) => ui.error(`Could not stop ${t.action?.name ?? 'the run'}`, err));
}

export function closeRun(host: string, t: TerminalInfo): void {
  fleet
    .call('terminal.close', { id: t.id }, host)
    .then(() => delete fleet.terminals[key(host, t.id)])
    .catch((err) => ui.error('Could not close the run', err));
}

/** Show a run's terminal: the agent's Terminal tab, or the project page's terminal section. */
export function showRun(host: string, t: TerminalInfo, inAgent: boolean): void {
  ui.terminalFocus = { host, id: t.id };
  if (inAgent) ui.showPanel('terminal');
}

async function runCommand(host: string, projectId: string, actionId: string, cwd: string, inAgent: boolean): Promise<void> {
  const t = await fleet.call('actions.run', { projectId, actionId, cwd }, host, 30_000);
  fleet.terminals[key(host, t.id)] = t;
  showRun(host, t, inAgent);
}

async function launch(a: ProjectAction, ctx: ActionContext): Promise<void> {
  const host = ctx.project.hostId;
  try {
    switch (a.kind) {
      case 'run':
        await runCommand(host, ctx.project.id, a.id, baseDir(ctx), !!ctx.agent);
        return;
      case 'prompt': {
        const message = fillPlaceholders(a.prompt, ctx);
        const agent = ctx.agent;
        if (agent) {
          const running = agent.status === 'running' || agent.status === 'starting';
          await fleet.call('agents.prompt', { id: agent.id, message, mode: running ? 'followUp' : 'auto' }, host, 60_000);
          if (running) ui.toast('info', 'Queued for after this turn', a.name);
          return;
        }
        const created = await fleet.call(
          'agents.create',
          { projectId: ctx.project.id, title: a.name, prompt: message, worktree: a.worktree || undefined },
          host,
          120_000,
        );
        fleet.agents[key(host, created.id)] = created;
        router.go({ name: 'agent', host, id: created.id });
        return;
      }
      case 'url': {
        if (urlTarget(a, ctx) === 'external') {
          openExternal(a.url);
          return;
        }
        await openInBrowserPanel(host, ctx.project.id, a.url);
        return;
      }
    }
  } catch (err) {
    ui.error(`${a.name} failed`, err);
  }
}
