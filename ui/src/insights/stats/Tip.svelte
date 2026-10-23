<script lang="ts">
  // Hover tooltip of the charts: follows the pointer and stays inside the window.
  import type { Snippet } from 'svelte';

  let { x, y, children }: { x: number; y: number; children: Snippet } = $props();

  let w = $state(0);
  let h = $state(0);
  const left = $derived(Math.max(8, Math.min(x + 14, window.innerWidth - w - 8)));
  const top = $derived(y + 12 + h > window.innerHeight - 8 ? Math.max(8, y - h - 12) : y + 12);
</script>

<div class="tip" role="tooltip" bind:offsetWidth={w} bind:offsetHeight={h} style:left="{left}px" style:top="{top}px">
  {@render children()}
</div>

<style>
  .tip {
    position: fixed;
    z-index: 50;
    pointer-events: none;
    min-width: 180px;
    max-width: min(320px, calc(100vw - 16px));
    padding: 8px 10px;
    background: var(--bg-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-xs);
    line-height: 1.5;
  }
  .tip :global(.row) {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tip :global(.row b) {
    margin-left: auto;
    padding-left: 12px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .tip :global(.sw) {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .tip :global(.foot) {
    margin-top: 4px;
    color: var(--text-3);
  }
</style>
