<script lang="ts">
  import type { Subagent } from '../lib/protocol';
  import { ChevronRight, Users } from '../lib/icons';
  import SubagentRow from './SubagentRow.svelte';

  // The subagents a `task` call started: open while any runs, collapsed to one line once all are
  // done. A row opens the subagent in the side panel's Subagents tab.
  let { subagents, onsubagent }: { subagents: Subagent[]; onsubagent?: (s: Subagent) => void } = $props();

  const running = $derived(subagents.filter((s) => s.status === 'running').length);
  const failed = $derived(subagents.filter((s) => s.status === 'failed').length);
  /** `null` = follow the state (open while running). */
  let expanded = $state<boolean | null>(null);
  const open = $derived(expanded ?? running > 0);
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
      <SubagentRow {s} onclick={onsubagent && (() => onsubagent(s))} />
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
</style>
