<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { ago, cost, modelName } from '../lib/format';
  import { href } from '../lib/router.svelte';
  import { agentState } from '../lib/status';
  import { now } from '../lib/clock.svelte';
  import type { AgentSummary } from '../lib/protocol';
  import { GitBranch, Users } from '../lib/icons';
  import StatusDot from './StatusDot.svelte';
  import DiffCount from './DiffCount.svelte';
  import HostDot from './HostDot.svelte';

  let { agent: a, showProject = true }: { agent: AgentSummary; showProject?: boolean } = $props();

  const st = $derived(agentState(a));
  const project = $derived(fleet.project(a.hostId, a.projectId));
  const host = $derived(fleet.host(a.hostId));
  const line2 = $derived(st === 'error' && a.error ? a.error : (a.preview ?? ''));
</script>

<a class="arow" data-state={st} href={href({ name: 'agent', host: a.hostId, id: a.id })}>
  <span class="c-dot"><StatusDot state={st} /></span>
  <span class="c-main">
    <span class="title" class:unread={a.unread}>
      <span class="ellipsis">{a.title}</span>
      {#if a.pendingRequests}<span class="badge">{a.pendingRequests}</span>{/if}
    </span>
    <span class="preview ellipsis" class:err={st === 'error'}>{line2 || ' '}</span>
  </span>
  <span class="c-where">
    {#if showProject}
      <span class="ellipsis proj">{project?.name ?? a.cwd.split('/').pop()}</span>
    {/if}
    <span class="chip host"><HostDot online={host?.online ?? false} size={5} />{host?.name ?? a.hostId}</span>
  </span>
  <span class="c-branch">
    {#if a.worktree}
      <span class="chip mono" title="Worktree {a.worktree.path} (from {a.worktree.base})"><GitBranch size={11} /><span class="ellipsis">{a.worktree.branch}</span></span>
    {:else if project?.git?.branch}
      <span class="chip mono faint-chip" title="Project checkout"><GitBranch size={11} /><span class="ellipsis">{project.git.branch}</span></span>
    {/if}
  </span>
  <span class="c-todo">
    {#if a.todos && a.todos.total}
      <span class="todo">
        <span class="tn num">{a.todos.done}/{a.todos.total}</span>
        <span class="progress" class:done={a.todos.done === a.todos.total}><i style:width="{(a.todos.done / a.todos.total) * 100}%"></i></span>
      </span>
      {#if a.todos.current}<span class="cur ellipsis" title={a.todos.current}>{a.todos.current}</span>{/if}
    {/if}
  </span>
  <span class="c-diff">
    {#if a.diff && (a.diff.additions || a.diff.deletions)}<DiffCount add={a.diff.additions} del={a.diff.deletions} />{/if}
  </span>
  <span class="c-meta num">
    {#if a.subagents}<span class="sub" title="{a.subagents} running subagents"><Users size={11} />{a.subagents}</span>{/if}
    {#if a.contextPercent != null}
      <span class="ctx" title="Context used · {modelName(a.model)}" class:hot={a.contextPercent >= 80}>{Math.round(a.contextPercent)}%</span>
    {/if}
    <span class="cost">{cost(a.costUsd)}</span>
  </span>
  <span class="c-time num" title={new Date(a.lastActivityAt).toLocaleString()}>{ago(a.lastActivityAt, now.value)}</span>
</a>

<style>
  .arow {
    display: grid;
    grid-template-columns: 14px minmax(200px, 1fr) 110px 150px 140px 76px 104px 30px;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 6px 12px;
    border-radius: var(--r);
    color: var(--text);
    transition: background var(--t-fast) var(--ease);
  }
  .arow:hover {
    background: var(--bg-hover);
  }
  .arow:focus-visible {
    outline-offset: -2px;
  }
  .c-dot {
    display: flex;
    justify-content: center;
  }
  .c-main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-weight: 450;
  }
  .title.unread {
    font-weight: 600;
  }
  .preview {
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .preview.err {
    color: var(--red);
  }
  .c-where {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    min-width: 0;
  }
  .proj {
    max-width: 100%;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .host {
    height: 18px;
  }
  .c-branch {
    min-width: 0;
  }
  .c-branch .chip {
    max-width: 100%;
  }
  .faint-chip {
    background: transparent;
    color: var(--text-3);
  }
  .c-todo {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .todo {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .tn {
    font-size: var(--fs-xs);
    color: var(--text-2);
    min-width: 26px;
  }
  .todo .progress {
    flex: 1;
    max-width: 90px;
  }
  .todo .progress > i {
    background: var(--accent);
  }
  .todo .progress.done > i {
    background: var(--green);
  }
  .cur {
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .c-diff {
    display: flex;
    justify-content: flex-end;
  }
  .c-meta {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 9px;
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .sub {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--violet);
  }
  .ctx.hot {
    color: var(--amber);
  }
  .cost {
    min-width: 36px;
    text-align: right;
  }
  .c-time {
    color: var(--text-3);
    font-size: var(--fs-xs);
    text-align: right;
  }

  @container (max-width: 900px) {
    .arow {
      grid-template-columns: 14px minmax(160px, 1fr) 110px 140px 76px 100px 30px;
    }
    .c-todo {
      display: none;
    }
  }
  @container (max-width: 820px) {
    .arow {
      grid-template-columns: 14px minmax(140px, 1fr) 120px 84px 34px;
    }
    .c-branch,
    .c-meta {
      display: none;
    }
  }
  @container (max-width: 560px) {
    .arow {
      grid-template-columns: 12px minmax(0, 1fr) auto;
      gap: 10px;
      padding: 8px 10px;
    }
    .c-where,
    .c-diff {
      display: none;
    }
  }
</style>
