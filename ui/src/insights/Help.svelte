<script lang="ts">
  // "?" next to a number: explains formula and source, and shows its breakdown. Opens on hover
  // (desktop) and on tap (touch), closes on outside click or Escape.
  import type { Snippet } from 'svelte';

  let { label = 'How is this computed?', children }: { label?: string; children: Snippet } = $props();

  let open = $state(false);
  let el = $state<HTMLElement>();
  let pop = $state<HTMLElement>();
  let pos = $state({ left: 0, top: 0 });

  function place() {
    if (!el || !pop) return;
    const r = el.getBoundingClientRect();
    const w = pop.offsetWidth;
    const h = pop.offsetHeight;
    const left = Math.max(8, Math.min(window.innerWidth - w - 8, r.left + r.width / 2 - w / 2));
    const below = r.bottom + 6 + h < window.innerHeight;
    pos = { left, top: below ? r.bottom + 6 : Math.max(8, r.top - h - 6) };
  }

  $effect(() => {
    if (open) place();
  });

  function onwindow(e: MouseEvent) {
    if (open && el && !el.contains(e.target as Node) && !pop?.contains(e.target as Node)) open = false;
  }
</script>

<svelte:window onclick={onwindow} onkeydown={(e) => e.key === 'Escape' && (open = false)} onscroll={() => (open = false)} />

<span class="help" bind:this={el}>
  <button
    class="q"
    aria-label={label}
    aria-expanded={open}
    onclick={(e) => {
      e.stopPropagation();
      open = !open;
    }}
    onmouseenter={(e) => {
      if (matchMedia('(hover: hover)').matches) open = true;
      e.stopPropagation();
    }}
    onmouseleave={() => {
      if (matchMedia('(hover: hover)').matches) open = false;
    }}>?</button
  >
  {#if open}
    <div class="pop" role="tooltip" bind:this={pop} style:left="{pos.left}px" style:top="{pos.top}px">
      {@render children()}
    </div>
  {/if}
</span>

<style>
  .help {
    display: inline-flex;
    vertical-align: middle;
  }
  .q {
    all: unset;
    display: inline-grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid var(--border-strong);
    color: var(--text-3);
    font-size: 9px;
    line-height: 1;
    cursor: help;
    text-transform: none;
    letter-spacing: 0;
  }
  .q:hover,
  .q[aria-expanded='true'] {
    color: var(--text);
    border-color: var(--text-3);
  }
  .pop {
    position: fixed;
    z-index: 50;
    max-width: min(340px, calc(100vw - 16px));
    min-width: 200px;
    padding: 9px 11px;
    background: var(--bg-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    box-shadow: var(--shadow-pop);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 400;
    line-height: 1.5;
    text-transform: none;
    letter-spacing: 0;
    white-space: normal;
    cursor: auto;
  }
  .pop :global(table) {
    width: 100%;
    border-collapse: collapse;
    margin-top: 4px;
  }
  .pop :global(td) {
    padding: 1px 0;
  }
  .pop :global(td:last-child) {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .pop :global(code) {
    font-family: var(--mono);
    font-size: 0.95em;
  }
</style>
