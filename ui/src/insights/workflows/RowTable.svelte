<script lang="ts" generics="T">
  // Table of the Workflows tab: a row opens its examples below it. Built from grid rows instead of
  // a <table> so that on a phone every row becomes a stacked block (name on top, the numbers
  // labelled below it) without horizontal scrolling.
  //
  // `cells` renders one element per column; numeric ones carry class `n`, and every one but the
  // first a `data-l` with its column label for the stacked layout.
  import type { Snippet } from 'svelte';

  interface Column {
    label: string;
    /** CSS grid track, e.g. `minmax(0, 1fr)` or `80px`. */
    width: string;
    num?: boolean;
  }

  let {
    cols,
    rows,
    key,
    expandable = () => false,
    cells,
    detail,
    empty,
  }: {
    cols: Column[];
    rows: T[];
    key: (row: T) => string;
    expandable?: (row: T) => boolean;
    cells: Snippet<[T]>;
    detail?: Snippet<[T]>;
    empty: string;
  } = $props();

  let open = $state<string | null>(null);
  const template = $derived(cols.map((c) => c.width).join(' '));

  function toggle(row: T) {
    const k = key(row);
    open = open === k ? null : k;
  }
</script>

<div class="table" style:--cols={template}>
  <div class="row head" aria-hidden="true">
    {#each cols as c (c.label)}<span class:n={c.num}>{c.label}</span>{/each}
  </div>
  {#each rows as row (key(row))}
    {@const k = key(row)}
    {#if expandable(row)}
      <div
        class="row click"
        class:on={open === k}
        role="button"
        tabindex="0"
        aria-expanded={open === k}
        onclick={() => toggle(row)}
        onkeydown={(e) => {
          if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
            e.preventDefault();
            toggle(row);
          }
        }}
      >
        {@render cells(row)}
      </div>
      {#if open === k && detail}<div class="detail">{@render detail(row)}</div>{/if}
    {:else}
      <div class="row">{@render cells(row)}</div>
    {/if}
  {:else}
    <div class="none faint">{empty}</div>
  {/each}
</div>

<style>
  .table {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    gap: 12px;
    align-items: center;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .row > :global(*) {
    min-width: 0;
  }
  .row > :global(.n) {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .head {
    padding-top: 0;
    font-size: var(--fs-xs);
    color: var(--text-3);
    font-weight: 500;
  }
  .head .n {
    text-align: right;
  }
  .click {
    cursor: pointer;
    border-radius: var(--r-sm);
  }
  .click:hover,
  .click.on {
    background: var(--bg-hover);
  }
  .click:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .detail {
    padding: 10px 12px;
    background: var(--bg-2);
    border-bottom: 1px solid var(--border);
  }
  .none {
    padding: 10px 8px;
    font-size: var(--fs-sm);
  }
  @media (max-width: 600px) {
    .head {
      display: none;
    }
    .row {
      display: flex;
      flex-wrap: wrap;
      gap: 2px 14px;
      padding: 8px 4px;
    }
    .row > :global(:first-child) {
      flex-basis: 100%;
    }
    .row > :global([data-l]) {
      text-align: left;
      font-size: var(--fs-sm);
    }
    .row > :global([data-l])::before {
      content: attr(data-l) ' ';
      color: var(--text-3);
      font-size: var(--fs-xs);
    }
    .detail {
      padding: 10px;
    }
  }
</style>
