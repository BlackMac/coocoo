<script lang="ts">
  import { cost, duration } from '../lib/format';
  import type { Subagent } from '../lib/protocol';
  import { Check, X } from '../lib/icons';

  // One subagent: status, name, what it does right now, tools, cost and run time (counting up
  // every second while it runs). Used in the chat below its `task` call and in the Subagents tab.
  let { s, onclick, on = false }: { s: Subagent; onclick?: () => void; on?: boolean } = $props();

  let tick = $state(Date.now());
  $effect(() => {
    if (s.status !== 'running') return;
    tick = Date.now();
    const timer = setInterval(() => (tick = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  const elapsed = $derived.by(() => {
    const ms = Math.max(0, (s.endedAt ?? tick) - s.startedAt);
    return ms < 1000 ? '0s' : duration(Math.floor(ms / 1000) * 1000);
  });
</script>

<button class="sub" class:on data-status={s.status} disabled={!onclick} {onclick} title={onclick ? 'Show its transcript' : undefined}>
  <span class="st">
    {#if s.status === 'running'}<span class="spinner"></span>{:else if s.status === 'completed'}<Check size={12} />{:else if s.status === 'failed'}<X size={12} />{:else}<span class="dash">–</span>{/if}
  </span>
  <span class="main">
    <span class="line">
      <span class="name mono">{s.id}</span>
      <span class="agent">{s.agent}</span>
      {#if s.description && s.description !== s.id}<span class="desc ellipsis">{s.description}</span>{/if}
    </span>
    {#if s.status === 'running' && s.activity}<span class="activity ellipsis">{s.activity}</span>{/if}
  </span>
  <span class="meta num">
    {#if s.toolCount}<span>{s.toolCount} tool{s.toolCount === 1 ? '' : 's'}</span>{/if}
    {#if s.costUsd}<span>{cost(s.costUsd)}</span>{/if}
    <span>{elapsed}</span>
  </span>
</button>

<style>
  .sub {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 5px 6px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    text-align: left;
    cursor: pointer;
  }
  .sub:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .sub.on {
    background: var(--bg-active);
  }
  .sub:disabled {
    cursor: default;
  }
  .st {
    display: grid;
    place-items: center;
    width: 14px;
    height: 17px;
    flex: none;
    color: var(--green);
  }
  .st .spinner {
    width: 10px;
    height: 10px;
    border-top-color: var(--blue);
  }
  [data-status='failed'] .st,
  [data-status='failed'] .name {
    color: var(--red);
  }
  .dash {
    color: var(--text-3);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    line-height: 17px;
  }
  .name {
    flex: none;
    color: var(--text);
    font-weight: 500;
  }
  .agent {
    flex: none;
    color: var(--text-3);
  }
  .desc {
    min-width: 0;
  }
  .activity {
    color: var(--text-3);
  }
  .meta {
    display: flex;
    gap: 8px;
    flex: none;
    color: var(--text-3);
    line-height: 17px;
  }
  @container (max-width: 520px) {
    .desc,
    .meta span:not(:last-child) {
      display: none;
    }
  }
</style>
