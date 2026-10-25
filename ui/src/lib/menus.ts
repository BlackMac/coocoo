// The "…" menus of agents and projects: the agent page, the project page and the sidebar rows
// offer the same actions, built here.

import type { Component } from 'svelte';
import { copyText } from './clipboard';
import { fleet, key } from './fleet.svelte';
import { fetchProject } from './git.svelte';
import { Archive, ArrowDownToLine, Copy, Moon, Pencil, Plus, Trash2 } from './icons';
import type { AgentSummary, Project } from './protocol';
import { router } from './router.svelte';
import { ui } from './ui.svelte';

export interface MenuItem {
  label: string;
  icon?: Component<{ size?: number }>;
  danger?: boolean;
  disabled?: boolean;
  run: () => void;
}

export function copy(text: string) {
  copyText(text).then(() => ui.toast('success', 'Copied'), (err) => ui.error('Copy failed', err));
}

async function lifecycle(a: AgentSummary, method: 'agents.hibernate' | 'agents.archive', label: string) {
  try {
    fleet.agents[key(a.hostId, a.id)] = await fleet.call(method, { id: a.id }, a.hostId);
  } catch (err) {
    ui.error(`${label} failed`, err);
  }
}

/** Asks, then deletes the agent; leaves its page when it is the open one. */
export function deleteAgent(a: AgentSummary, removeWorktree: boolean, deleteBranch = false) {
  ui.dialog = {
    kind: 'confirm',
    title: deleteBranch ? 'Clean up worktree' : 'Delete agent',
    message: deleteBranch
      ? `Delete “${a.title}”, its session, the worktree ${a.worktree?.path} and the branch ${a.worktree?.branch}? Everything on it is merged.`
      : removeWorktree
        ? `Delete “${a.title}”, its session and the worktree ${a.worktree?.path}? Uncommitted work in the worktree is lost.`
        : `Delete “${a.title}” and its session? The working directory is kept.`,
    action: 'Delete',
    danger: true,
    run: async () => {
      await fleet.call('agents.delete', { id: a.id, removeWorktree, deleteBranch }, a.hostId);
      const r = router.route;
      if (r.name === 'agent' && r.host === a.hostId && r.id === a.id) {
        router.go(a.projectId ? { name: 'project', host: a.hostId, id: a.projectId } : { name: 'overview' });
      }
    },
  };
}

/** `rename` adds a Rename entry (the pages rename in place, by clicking the title). */
export function agentMenu(a: AgentSummary, rename?: () => void): MenuItem[] {
  return [
    ...(rename ? [{ label: 'Rename', icon: Pencil, run: rename }] : []),
    { label: 'Hibernate', icon: Moon, disabled: ['hibernated', 'stopped'].includes(a.status), run: () => lifecycle(a, 'agents.hibernate', 'Hibernate') },
    { label: 'Archive', icon: Archive, disabled: a.status === 'stopped', run: () => lifecycle(a, 'agents.archive', 'Archive') },
    { label: 'Delete', icon: Trash2, danger: true, run: () => deleteAgent(a, false) },
    ...(a.worktree ? [{ label: 'Delete with worktree', icon: Trash2, danger: true, run: () => deleteAgent(a, true) }] : []),
  ];
}

export function renameAgent(a: AgentSummary, title: string) {
  fleet
    .call('agents.rename', { id: a.id, title }, a.hostId)
    .then((s) => (fleet.agents[key(a.hostId, a.id)] = s))
    .catch((err) => ui.error('Rename failed', err));
}

export function renameProject(p: Project, name: string) {
  fleet
    .call('projects.update', { id: p.id, name }, p.hostId)
    .then((np) => (fleet.projects[key(p.hostId, p.id)] = np))
    .catch((err) => ui.error('Rename failed', err));
}

/** Asks, then removes the project from Coocoo; leaves its page when it is the open one. */
export function removeProject(p: Project) {
  ui.dialog = {
    kind: 'confirm',
    title: 'Remove project',
    message: `Remove “${p.name}” from Coocoo? Files on disk are not touched.`,
    action: 'Remove',
    danger: true,
    run: async () => {
      await fleet.call('projects.remove', { id: p.id }, p.hostId);
      delete fleet.projects[key(p.hostId, p.id)];
      const r = router.route;
      if (r.name === 'project' && r.host === p.hostId && r.id === p.id) router.go({ name: 'overview' });
    },
  };
}

/**
 * The project page has its own buttons for a new agent and fetching and renames in place; the
 * sidebar (`full`) offers them in the menu.
 */
export function projectMenu(p: Project, full?: { rename: () => void }): MenuItem[] {
  return [
    ...(full
      ? [
          { label: 'New agent', icon: Plus, run: () => (ui.dialog = { kind: 'newAgent', hostId: p.hostId, projectId: p.id }) },
          { label: 'Rename', icon: Pencil, run: full.rename },
          ...(p.git ? [{ label: 'Fetch', icon: ArrowDownToLine, run: () => void fetchProject(p) }] : []),
        ]
      : []),
    { label: 'Copy path', icon: Copy, run: () => copy(p.path) },
    { label: 'Remove project', icon: Trash2, danger: true, run: () => removeProject(p) },
  ];
}
