<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary } from '../lib/protocol';
  import { SquareKanban } from '../lib/icons';
  import StartCard from './StartCard.svelte';

  // Above the composer: the project card this agent works on, else the top of the project's
  // todo column, which one tap hands to this agent.
  let { agent: a }: { agent: AgentSummary } = $props();

  const board = $derived(a.projectId ? fleet.boards[key(a.hostId, a.projectId)] : undefined);
  // The card it is refining counts as its current card too, so Start does not offer another one.
  const doing = $derived(
    board?.cards.find((c) => c.column === 'doing' && c.agentId === a.id) ?? board?.cards.find((c) => c.refining && c.agentId === a.id),
  );
  const next = $derived(board?.cards.find((c) => c.column === 'todo'));
  const todoCount = $derived(board?.cards.filter((c) => c.column === 'todo').length ?? 0);

  $effect(() => {
    if (fleet.conn === 'open' && a.projectId) fleet.loadBoard(a.hostId, a.projectId).catch(() => {});
  });
</script>

{#if a.projectId && a.status !== 'stopped' && (doing || next)}
  <div class="next" class:doing={!!doing}>
    <button class="open" onclick={() => ui.showPanel('board')} title="Open the project board">
      <SquareKanban size={13} />
      <span class="lbl">{doing ? (doing.column === 'doing' ? 'Doing' : 'Refining') : 'Next'}</span>
      <span class="ttl ellipsis">{(doing ?? next)!.title}</span>
      {#if !doing && todoCount > 1}<span class="more faint num">+{todoCount - 1}</span>{/if}
    </button>
    {#if !doing && next}<StartCard host={a.hostId} projectId={a.projectId} card={next} agent={a} primary offerRefine />{/if}
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
</style>
