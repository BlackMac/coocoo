<script lang="ts">
  // Friction signals in the period and their trend against the previous one; more is worse.
  import type { WorkflowFriction } from '../../lib/protocol';
  import Examples from './Examples.svelte';
  import RowTable from './RowTable.svelte';

  let { friction }: { friction: WorkflowFriction[] } = $props();

  const cols = [
    { label: 'Signal', width: 'minmax(0, 3fr)' },
    { label: 'Count', width: 'minmax(0, 0.7fr)', num: true },
    { label: 'Trend', width: 'minmax(0, 0.7fr)', num: true },
  ];

  function trend(f: WorkflowFriction): { text: string; tone: 'bad' | 'good' | 'flat' } {
    if (f.prev == null) return { text: '—', tone: 'flat' };
    if (f.prev === 0) return f.count ? { text: 'new', tone: 'bad' } : { text: '—', tone: 'flat' };
    const d = f.count - f.prev;
    if (d === 0) return { text: '±0', tone: 'flat' };
    return d > 0 ? { text: `▲ ${d}`, tone: 'bad' } : { text: `▼ ${-d}`, tone: 'good' };
  }
</script>

<RowTable {cols} rows={friction} key={(f) => f.id} expandable={(f) => f.examples.length > 0} empty="No friction signals.">
  {#snippet cells(f)}
    {@const t = trend(f)}
    <span class="ellipsis" title={f.label}>{f.label}</span>
    <span class="n" data-l="Count">{f.count}</span>
    <span class="n {t.tone}" data-l="Trend" title={f.prev != null ? `Previous period: ${f.prev}` : 'No comparison for this period'}>{t.text}</span>
  {/snippet}
  {#snippet detail(f)}
    <Examples examples={f.examples} section="friction" title="{f.label} — latest examples:" />
  {/snippet}
</RowTable>

<style>
  .bad {
    color: var(--del-text);
  }
  .good {
    color: var(--add-text);
  }
  .flat {
    color: var(--text-3);
  }
</style>
