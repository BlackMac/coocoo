<script lang="ts">
  import { cost, duration } from '../lib/format';
  import type { Subagent } from '../lib/protocol';
  import { Check, ChevronRight, Users, X } from '../lib/icons';

  // The subagents a `task` call started: open while any runs, collapsed to one line once all are
  // done. A row opens the subagent's transcript.
  let { subagents, onsubagent }: { subagents: Subagent[]; onsubagent?: (s: Subagent) => void } = $props();

  const running = $derived(subagents.filter((s) => s.status === 'running').length);
  const failed = $derived(subagents.filter((s) => s.status === 'failed').length);
  /** `null` = follow the state (open while running). */
  let expanded = $state<boolean | null>(null);
  const open = $derived(expanded ?? running > 0);

  // Running subagents count up every second (the shared clock ticks every 10 s).
  let tick = $state(Date.now());
  $effect(() => {
    if (!running) return;
    tick = Date.now();
    const timer = setInterval(() => (tick = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  const elapsed = (s: Subagent) => {
    const ms = Math.max(0, (s.endedAt ?? tick) - s.startedAt);
    return ms < 1000 ? '0s' : duration(Math.floor(ms / 1000) * 1000);
  };
</script>

<div class="subs" class:open>
  <button class="head" onclick={() => (expanded = !open)} aria-expanded={open}>
    <Users size={12} />
    <span>{subagents.length} subagent{subagents.length === 1 ? '' : 's'}</span>
    <span class="faint">
      {#if running}· {running} running{:else if failed}· {failed} failed{:else}· done{/if}
    </span>
    <span class="chev"><ChevronRight size={11} /></span>
  </button>
  {#if open}
    {#each subagents as s (s.id)}
      <button class="sub" data-status={s.status} disabled={!s.sessionFile || !onsubagent} onclick={() => onsubagent?.(s)} title={s.sessionFile ? 'Open transcript' : undefined}>
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
          <span>{elapsed(s)}</span>
        </span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .subs {
    display: flex;
    flex-direction: column;
    padding: 0 6px 6px 27px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    height: 22px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .head:hover {
    background: var(--bg-hover);
  }
  .chev {
    display: flex;
    color: var(--text-3);
    transition: transform var(--t-fast) var(--ease);
  }
  .open .chev {
    transform: rotate(90deg);
  }
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
