<script lang="ts">
  import type { AgentSummary, TaskSummary } from '../lib/protocol';
  import { ui } from '../lib/ui.svelte';
  import { CornerLeftUp, ListChecks, Target } from '../lib/icons';

  // Pinned above the transcript: the agent's own summary of its current task
  // (`coocoo_task_summary`) and the progress of its plan.
  let { agent: a, task, onjump }: { agent: AgentSummary; task: TaskSummary; onjump: (ms: number) => void } = $props();

  let open = $state(false);
  const todos = $derived(a.todos);
</script>

<div class="task" class:open>
  <div class="row">
    <Target size={13} />
    <span class="lbl">Task</span>
    <button class="txt" aria-expanded={open} title={open ? 'Collapse' : task.text} onclick={() => (open = !open)}>{task.text}</button>
    {#if todos}
      <button class="plan num" title={todos.current ? `Now: ${todos.current}` : 'Open the plan'} onclick={() => ui.showPanel('plan', { agentId: a.id, projectId: a.projectId ?? undefined })}>
        <ListChecks size={13} />{todos.done}/{todos.total}
      </button>
    {/if}
    {#if task.since}
      <button class="btn ghost icon sm" title="Jump to the request" aria-label="Jump to the request" onclick={() => onjump(task.since!)}><CornerLeftUp size={13} /></button>
    {/if}
  </div>
</div>

<style>
  .task {
    flex: none;
    border-bottom: 1px solid var(--border);
    background: var(--bg-1);
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    max-width: 920px;
    margin: 0 auto;
    padding: 5px 12px 5px 16px;
    color: var(--blue);
    line-height: 20px;
  }
  .row > :global(svg) {
    flex: none;
    margin-top: 3.5px;
  }
  .lbl {
    flex: none;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .txt {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .open .txt {
    white-space: normal;
  }
  .plan {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    height: 20px;
    padding: 0 6px;
    border: 0;
    border-radius: 99px;
    background: var(--bg-3);
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .plan:hover {
    color: var(--text);
  }
  .btn.icon {
    flex: none;
    width: 20px;
    height: 20px;
    color: var(--text-2);
  }
  @media (max-width: 640px) {
    .row {
      padding: 5px 8px 5px 12px;
    }
  }
</style>
