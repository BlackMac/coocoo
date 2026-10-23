<script lang="ts" generics="T">
  // Table of the Stats tab. A real <table> on wide screens (columns size to their numbers); on a
  // phone every row becomes a stacked block: the title cell on its own line, then the key numbers
  // labelled inline (`data-l`), less important columns hidden — no horizontal scrolling.
  //
  // `row` renders one <td> per column. Cell classes: `t` title (first column), `n` number,
  // `hm` hidden on a phone, `bar` with an inline share bar (`<i style:width>` + `<span>`).
  import type { Snippet } from 'svelte';

  interface Column {
    label: string;
    /** Left-aligned text column (the first one always is). */
    left?: boolean;
    /** Hidden on a phone. */
    hm?: boolean;
    /** Header tooltip: what the column counts. */
    title?: string;
  }

  let {
    cols,
    rows,
    key,
    row,
    onrow,
    active,
    open,
    detail,
    limit = 0,
    empty,
  }: {
    cols: Column[];
    rows: T[];
    key: (r: T) => string;
    row: Snippet<[T]>;
    /** Click (or Enter) on a row. */
    onrow?: (r: T) => void;
    /** Highlighted rows (the active filter). */
    active?: (r: T) => boolean;
    /** Rows whose `detail` is shown below them. */
    open?: (r: T) => boolean;
    detail?: Snippet<[T]>;
    /** Show only the first `limit` rows until "Show all" (0 = all). */
    limit?: number;
    empty: string;
  } = $props();

  let all = $state(false);
  const shown = $derived(limit && !all ? rows.slice(0, limit) : rows);

  function keydown(e: KeyboardEvent, r: T) {
    if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      onrow?.(r);
    }
  }
</script>

{#if rows.length}
  <div class="tbl">
    <table>
      <thead>
        <tr>
          {#each cols as c, i (c.label)}<th class:left={i === 0 || c.left} class:hm={c.hm} title={c.title}>{c.label}</th>{/each}
        </tr>
      </thead>
      <tbody>
        {#each shown as r (key(r))}
          {@const isOpen = !!open?.(r)}
          <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
          <tr
            class="row"
            class:click={!!onrow}
            class:on={!!active?.(r)}
            class:open={isOpen}
            tabindex={onrow ? 0 : undefined}
            aria-expanded={detail ? isOpen : undefined}
            onclick={onrow && (() => onrow?.(r))}
            onkeydown={onrow && ((e) => keydown(e, r))}
          >
            {@render row(r)}
          </tr>
          {#if detail && isOpen}
            <tr class="detail"><td colspan={cols.length}>{@render detail(r)}</td></tr>
          {/if}
        {/each}
      </tbody>
    </table>
  </div>
  {#if limit && rows.length > limit}
    <button class="more" onclick={() => (all = !all)}>{all ? 'Show fewer' : `Show all ${rows.length}`}</button>
  {/if}
{:else}
  <div class="none faint">{empty}</div>
{/if}

<style>
  .tbl {
    overflow-x: auto;
    min-width: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 500;
    text-align: right;
    white-space: nowrap;
  }
  th.left {
    text-align: left;
  }
  th[title] {
    cursor: help;
  }
  .tbl :global(td) {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    text-align: right;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .tbl :global(td.t) {
    text-align: left;
    max-width: 340px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tbl :global(td.left) {
    text-align: left;
  }
  tr.click {
    cursor: pointer;
  }
  tr.click:hover > :global(td),
  tr.open > :global(td) {
    background: var(--bg-hover);
  }
  tr.on > :global(td) {
    background: var(--accent-soft);
  }
  tr.row:last-child > :global(td) {
    border-bottom: 0;
  }
  .tbl :global(td.bar) {
    position: relative;
  }
  .tbl :global(td.bar > i) {
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    background: var(--accent-soft);
    border-radius: 2px;
  }
  .tbl :global(td.bar > span) {
    position: relative;
  }
  .detail > td {
    padding: 12px 14px;
    background: var(--bg-2);
    text-align: left;
    white-space: normal;
  }
  .more {
    all: unset;
    display: block;
    margin: 8px auto 0;
    color: var(--accent-text);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .none {
    padding: 18px 0 8px;
    text-align: center;
    font-size: var(--fs-sm);
  }

  @media (max-width: 600px) {
    .tbl {
      overflow: visible;
    }
    table,
    tbody,
    tr,
    .tbl :global(td) {
      display: block;
    }
    thead {
      display: none;
    }
    tr.row {
      display: flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: 2px 12px;
      padding: 8px 4px;
      border-bottom: 1px solid var(--border);
    }
    tr.row > :global(td) {
      padding: 0;
      border: 0;
      text-align: left;
      background: none !important;
    }
    tr.click:hover,
    tr.open {
      background: var(--bg-hover);
    }
    tr.on {
      background: var(--accent-soft);
    }
    .tbl :global(td.t) {
      flex: 1 0 100%;
      max-width: none;
      white-space: normal;
      overflow-wrap: anywhere;
    }
    .tbl :global(td.hm) {
      display: none;
    }
    .tbl :global(td[data-l]::after) {
      content: ' ' attr(data-l);
      color: var(--text-3);
    }
    .tbl :global(td.bar > i) {
      display: none;
    }
    .detail > td {
      padding: 10px 6px;
    }
  }
</style>
