<script lang="ts">
  import type { AgentDetail, AgentSummary } from '../lib/protocol';
  import { Check, CircleDashed, X } from '../lib/icons';

  let { detail, agent }: { detail?: AgentDetail; agent: AgentSummary } = $props();

  const phases = $derived(detail?.todos ?? []);
  const total = $derived(phases.reduce((n, p) => n + p.tasks.length, 0));
  const done = $derived(phases.reduce((n, p) => n + p.tasks.filter((t) => t.status === 'completed').length, 0));
</script>

<div class="plan">
  {#if !detail}
    <div class="empty"><span class="spinner"></span></div>
  {:else if !phases.length}
    <div class="empty">
      <strong>No plan yet</strong>
      <span>{agent.status === 'running' ? 'The agent has not written a todo list for this task.' : 'Todo lists the agent writes show up here.'}</span>
    </div>
  {:else}
    <div class="sum">
      <span class="num"><strong>{done}</strong> of {total} done</span>
      <span class="progress"><i style:width="{total ? (done / total) * 100 : 0}%"></i></span>
    </div>
    {#each phases as ph, pi (pi)}
      {@const pd = ph.tasks.filter((t) => t.status === 'completed').length}
      <section class="phase">
        <h3><span>{ph.name}</span><span class="faint num">{pd}/{ph.tasks.length}</span></h3>
        <ul>
          {#each ph.tasks as t, ti (ti)}
            <li data-status={t.status}>
              <span class="mark">
                {#if t.status === 'completed'}<Check size={11} strokeWidth={3} />{:else if t.status === 'abandoned'}<X size={11} strokeWidth={3} />{:else if t.status === 'in_progress'}<span class="spinner"></span>{:else}<CircleDashed size={14} />{/if}
              </span>
              <span class="txt">{t.content}</span>
              {#if t.status === 'blocked'}<span class="chip blocked">blocked</span>{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

<style>
  .plan {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .sum {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .sum .progress {
    flex: 1;
  }
  .sum .progress > i {
    background: var(--green);
  }
  .sum strong {
    color: var(--text);
  }
  h3 {
    display: flex;
    justify-content: space-between;
    margin: 0 0 6px;
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  li {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 5px 0;
    line-height: 1.45;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    margin-top: 1px;
    flex: none;
    border-radius: 50%;
    color: var(--text-3);
  }
  [data-status='completed'] .mark {
    background: var(--green);
    color: #fff;
  }
  [data-status='abandoned'] .mark {
    background: var(--bg-3);
    color: var(--text-3);
  }
  [data-status='completed'] .txt,
  [data-status='abandoned'] .txt {
    color: var(--text-3);
  }
  [data-status='abandoned'] .txt {
    text-decoration: line-through;
  }
  [data-status='in_progress'] .txt {
    color: var(--text);
    font-weight: 500;
  }
  [data-status='in_progress'] .spinner {
    width: 13px;
    height: 13px;
    border-top-color: var(--blue);
  }
  .txt {
    flex: 1;
    color: var(--text-2);
  }
  .blocked {
    color: var(--amber);
    height: 18px;
  }
</style>
