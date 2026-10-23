<script lang="ts">
  // One KPI tile: label with "?" breakdown, big value, sub line, change vs. the previous period.
  import type { Snippet } from 'svelte';
  import type { Delta } from './format';
  import Help from './Help.svelte';

  let {
    label,
    value,
    sub,
    delta = null,
    help,
  }: {
    label: string;
    value: string;
    sub?: string;
    delta?: Delta | null;
    /** Formula, source and breakdown. */
    help?: Snippet;
  } = $props();
</script>

<div class="card kpi">
  <div class="l">
    <span class="ellipsis">{label}</span>
    {#if help}<Help label="How is {label} computed?">{@render help()}</Help>{/if}
  </div>
  <div class="v num">{value}</div>
  <div class="s num">
    {#if delta}<span class="d {delta.tone}">{delta.text}</span>{/if}
    {#if sub}<span>{sub}</span>{/if}
  </div>
</div>

<style>
  .kpi {
    padding: 12px 14px;
    min-width: 0;
  }
  .l {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .v {
    font-size: 22px;
    font-weight: 600;
    margin: 4px 0 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .s {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .d.good {
    color: var(--green);
  }
  .d.bad {
    color: var(--red);
  }
  .d.flat {
    color: var(--text-3);
  }
  @media (max-width: 600px) {
    .v {
      font-size: 19px;
    }
  }
</style>
