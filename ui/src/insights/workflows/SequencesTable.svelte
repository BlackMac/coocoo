<script lang="ts">
  // Frequent step sequences within one agent (or one 30-minute stretch of UI use), with how
  // their occurrences usually end.
  import type { WorkflowSequence } from '../../lib/protocol';
  import { span } from '../format';
  import Examples from './Examples.svelte';
  import RowTable from './RowTable.svelte';
  import { FRICTION_STEPS, stepLabel } from './steps';

  let { sequences }: { sequences: WorkflowSequence[] } = $props();

  const cols = [
    { label: 'Sequence', width: 'minmax(0, 4fr)' },
    { label: 'Count', width: 'minmax(0, 0.8fr)', num: true },
    { label: 'Median duration', width: 'minmax(0, 1.2fr)', num: true },
    { label: 'Ends with', width: 'minmax(0, 1fr)', num: true },
  ];

  const key = (s: WorkflowSequence) => s.steps.join('>');

  /** What most occurrences end with: a commit, friction, or neither. */
  function ending(s: WorkflowSequence): 'commit' | 'friction' | null {
    if (s.commits * 2 > s.count) return 'commit';
    if (s.friction * 2 > s.count) return 'friction';
    return null;
  }
</script>

<RowTable {cols} rows={sequences} {key} expandable={(s) => s.examples.length > 0} empty="No sequence occurred at least twice in this period.">
  {#snippet cells(s)}
    {@const end = ending(s)}
    <span class="steps" title={s.phone * 2 > s.count ? `${s.phone} of ${s.count} started on a phone` : undefined}>
      {#if s.phone * 2 > s.count}<span aria-label="On a phone">📱 </span>{/if}{#each s.steps as step, i (i)}{#if i > 0}<span class="arrow"> → </span>{/if}{#if FRICTION_STEPS[step]}<b>{stepLabel(step)}</b>{:else}{stepLabel(step)}{/if}{/each}
    </span>
    <span class="n" data-l="Count">{s.count}</span>
    <span class="n" data-l="Median" title="From the first to the last step">{span(s.medianMs)}</span>
    <span class="n" data-l="Ends with" title="{s.commits} of {s.count} with a commit within 30 min · {s.friction} with friction">
      {#if end === 'commit'}<span class="pill ok">commit</span>{:else if end === 'friction'}<span class="pill warn">friction</span>{:else}<span class="faint">—</span>{/if}
    </span>
  {/snippet}
  {#snippet detail(s)}
    <Examples examples={s.examples} section="sequences" title="Where it happened — opens the agent at the first step:" />
  {/snippet}
</RowTable>

<style>
  .steps {
    line-height: 1.5;
  }
  .steps b {
    font-weight: 600;
  }
  .arrow {
    color: var(--text-3);
  }
  .pill {
    display: inline-block;
    font-size: 10px;
    padding: 1px 7px;
    border-radius: 999px;
  }
  .pill.ok {
    background: var(--green-soft);
    color: var(--add-text);
  }
  .pill.warn {
    background: var(--amber-soft);
    color: var(--amber);
  }
</style>
