<script lang="ts">
  // Which Coocoo features are used, how often and from which kind of device; unused ones stay
  // listed so they stand out — unless the log began during the period, so "never" is unknown.
  import type { WorkflowFeature } from '../../lib/protocol';
  import { shortDate } from '../format';
  import RowTable from './RowTable.svelte';

  let {
    features,
    partialSince,
  }: {
    features: WorkflowFeature[];
    /** The usage log began after the period started (`0`: nothing recorded at all). */
    partialSince?: number;
  } = $props();
  const partial = $derived(partialSince !== undefined);

  const cols = [
    { label: 'Feature', width: 'minmax(0, 2fr)' },
    { label: 'Uses', width: 'minmax(0, 0.8fr)', num: true },
    { label: 'Days', width: 'minmax(0, 0.7fr)', num: true },
    { label: 'Last used', width: 'minmax(0, 1fr)', num: true },
    { label: 'Split', width: '84px', num: true },
  ];

  const DEVICES = [
    { id: 'desktop', label: 'desktop', color: 'var(--accent)' },
    { id: 'phone', label: 'phone', color: 'var(--violet)' },
    { id: 'browser', label: 'browser', color: 'var(--blue)' },
  ] as const;
</script>

<RowTable {cols} rows={features} key={(f) => f.id} empty="No features.">
  {#snippet cells(f)}
    <span class="ellipsis" class:rare={f.uses <= 2 && !partial}>{f.label}</span>
    <span class="n" data-l="Uses">{f.uses}</span>
    <span class="n" data-l="Days">{f.days}</span>
    <span class="n" data-l="Last">{f.lastUsed ? shortDate(f.lastUsed) : '—'}</span>
    <span class="n" data-l={f.uses > 2 ? 'Split' : undefined}>
      {#if f.uses <= 2 && partial}
        <span class="faint">{partialSince ? `recorded since ${shortDate(partialSince)}` : 'not recorded yet'}</span>
      {:else if f.uses === 0}
        <span class="faint">never used</span>
      {:else if f.uses <= 2}
        <span class="faint">rarely used</span>
      {:else}
        <span class="split" title={DEVICES.map((d) => `${d.label} ${f[d.id]}`).join(' · ')}>
          {#each DEVICES as d (d.id)}{#if f[d.id]}<i style:flex-grow={f[d.id]} style:background={d.color}></i>{/if}{/each}
        </span>
      {/if}
    </span>
  {/snippet}
</RowTable>
<div class="legend">
  {#each DEVICES as d (d.id)}<span><span class="sw" style:background={d.color}></span>{d.label}</span>{/each}
</div>

<style>
  .rare {
    color: var(--del-text);
  }
  .split {
    display: inline-flex;
    width: 80px;
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
    vertical-align: middle;
  }
  .split i {
    flex-basis: 0;
    min-width: 2px;
  }
  .legend {
    display: flex;
    gap: 14px;
    margin-top: 8px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 6px;
  }
</style>
