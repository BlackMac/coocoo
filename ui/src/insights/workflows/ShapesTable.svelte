<script lang="ts">
  // Session shapes: agents classified by a fixed rule (first match wins), with what each shape
  // costs and how often it ends in a commit.
  import type { WorkflowShape } from '../../lib/protocol';
  import { pct, span, usd } from '../format';
  import Help from '../Help.svelte';
  import Examples from './Examples.svelte';
  import RowTable from './RowTable.svelte';

  let { shapes }: { shapes: WorkflowShape[] } = $props();

  const cols = [
    { label: 'Shape', width: 'minmax(0, 1.6fr)' },
    { label: 'Agents', width: 'minmax(0, 1fr)', num: true },
    { label: 'Prompts', width: 'minmax(0, 1fr)', num: true },
    { label: 'Turn length', width: 'minmax(0, 1fr)', num: true },
    { label: 'Avg. cost', width: 'minmax(0, 1fr)', num: true },
    { label: 'Commit rate', width: 'minmax(0, 1fr)', num: true },
  ];
</script>

<RowTable {cols} rows={shapes} key={(s) => s.id} expandable={(s) => s.examples.length > 0} empty="No agent with a prompt in this period.">
  {#snippet cells(s)}
    <span class="name">
      <span class="ellipsis" title={s.rule}>{s.label}</span>
      <Help label="Rule for {s.label}">{s.rule}</Help>
    </span>
    <span class="n" data-l="Agents">{s.agents}</span>
    <span class="n" data-l="Prompts" title="Median prompts per agent">{s.promptsMedian}</span>
    <span class="n" data-l="Turn" title="Median turn length">{span(s.turnMedianMs)}</span>
    <span class="n" data-l="Cost" title="Average API-equivalent cost per agent">{usd(s.costAvg)}</span>
    <span class="n" class:good={s.commitRate != null && s.commitRate >= 0.8} data-l="Commits" title="Agents with a commit ÷ agents that changed files">{pct(s.commitRate)}</span>
  {/snippet}
  {#snippet detail(s)}
    <Examples examples={s.examples} section="shapes" title="{s.label} — examples" />
  {/snippet}
</RowTable>

<style>
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .good {
    color: var(--add-text);
  }
</style>
