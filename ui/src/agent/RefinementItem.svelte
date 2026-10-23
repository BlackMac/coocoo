<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import type { TimelineItem } from '../lib/protocol';
  import { SquareKanban } from '../lib/icons';
  import RefinementView from './RefinementView.svelte';

  // A `kanban_refine` call in the chat: the card's refinement as it is stored on the board (the
  // latest one, should the card have been refined again since).
  let { item, host, projectId }: { item: TimelineItem; host: string; projectId: string } = $props();

  const cardId = $derived.by(() => {
    try {
      return String(JSON.parse(item.tool?.output ?? '').cardId ?? '');
    } catch {
      return '';
    }
  });
  const board = $derived(fleet.boards[key(host, projectId)]);
  const card = $derived(board?.cards.find((c) => c.id === cardId));

  $effect(() => {
    if (!board && fleet.conn === 'open') fleet.loadBoard(host, projectId).catch(() => {});
  });
</script>

{#if card?.refinement}
  <div class="ref">
    <div class="for faint"><SquareKanban size={12} /><span class="ellipsis">Card: {card.title}</span></div>
    <RefinementView {host} {projectId} {card} />
  </div>
{:else if board}
  <div class="gone faint">Refinement attached to a card that no longer exists.</div>
{/if}

<style>
  .ref {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .for {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .for :global(svg) {
    flex: none;
  }
  .gone {
    font-size: var(--fs-xs);
  }
</style>
