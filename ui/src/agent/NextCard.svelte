<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary } from '../lib/protocol';
  import { Play, SquareKanban } from '../lib/icons';

  // Above the composer: the card the agent works on, else the top of its todo column, which one
  // tap hands to the agent.
  let { agent: a }: { agent: AgentSummary } = $props();

  const board = $derived(fleet.boards[key(a.hostId, a.id)]);
  const doing = $derived(board?.cards.find((c) => c.column === 'doing'));
  const next = $derived(board?.cards.find((c) => c.column === 'todo'));
  const todoCount = $derived(board?.cards.filter((c) => c.column === 'todo').length ?? 0);

  $effect(() => {
    if (fleet.conn === 'open') fleet.loadBoard(a.hostId, a.id).catch(() => {});
  });

  let busy = $state(false);
  async function start() {
    if (!next || busy) return;
    busy = true;
    try {
      await fleet.call('agents.cardStart', { id: a.id, cardId: next.id }, a.hostId, 60_000);
    } catch (err) {
      ui.error('Could not start the card', err);
    } finally {
      busy = false;
    }
  }
</script>

{#if a.status !== 'stopped' && (doing || next)}
  <div class="next" class:doing={!!doing}>
    <button class="open" onclick={() => ui.showPanel('board')} title="Open the board">
      <SquareKanban size={13} />
      <span class="lbl">{doing ? 'Doing' : 'Next'}</span>
      <span class="ttl ellipsis">{(doing ?? next)!.title}</span>
      {#if !doing && todoCount > 1}<span class="more faint num">+{todoCount - 1}</span>{/if}
    </button>
    {#if !doing}
      <button class="btn primary sm go" onclick={start} disabled={busy} title="Let the agent work on this card">
        {#if busy}<span class="spinner"></span>{:else}<Play size={11} fill="currentColor" />{/if}Start
      </button>
    {/if}
  </div>
{/if}

<style>
  .next {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 4px 4px 6px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg-1);
  }
  .next.doing {
    border-color: color-mix(in srgb, var(--blue) 40%, var(--border));
  }
  .open {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 4px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .open:hover {
    background: var(--bg-hover);
  }
  .lbl {
    flex: none;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .doing .lbl {
    color: var(--blue);
  }
  .ttl {
    color: var(--text);
  }
  .more {
    flex: none;
    font-size: var(--fs-xs);
  }
  .go {
    flex: none;
    gap: 4px;
  }
  .go .spinner {
    width: 10px;
    height: 10px;
  }
</style>
