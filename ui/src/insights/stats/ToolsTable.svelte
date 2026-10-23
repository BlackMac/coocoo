<script lang="ts">
  // Tools the agents called, with their error rate (over 5 % is highlighted).
  import type { UsageReport } from '../../lib/protocol';
  import { count, pct } from '../format';
  import Section from './Section.svelte';
  import Table from './Table.svelte';

  type ToolRow = UsageReport['tools'][number];

  let { report }: { report: UsageReport } = $props();

  const maxCalls = $derived(Math.max(0, ...report.tools.map((t) => t.calls)));
  const rate = (t: ToolRow) => (t.calls ? t.errors / t.calls : 0);

  const cols = [
    { label: 'Tool' },
    { label: 'Calls', title: 'Tool calls by the agents, their subagents and advisor' },
    { label: 'Error rate', title: 'Calls that returned an error / calls' },
  ];
</script>

<Section title="Tools" note="calls · error rate">
  {#snippet help()}
    Tool calls of the agents, their subagents and advisor. <b>Error rate</b> = calls whose result was an error / calls; above 5 % is
    shown in red. Source: <code>~/.omp/stats.db</code>.
  {/snippet}
  <Table {cols} rows={report.tools} key={(t) => t.name} limit={12} empty="No tool calls in this period.">
    {#snippet row(t: ToolRow)}
      <td class="t mono">{t.name}</td>
      <td class="bar" data-l="calls"><i style:width="{maxCalls ? (100 * t.calls) / maxCalls : 0}%"></i><span>{count(t.calls)}</span></td>
      <td data-l="errors" class:bad={rate(t) > 0.05}>{pct(rate(t), 1)}</td>
    {/snippet}
  </Table>
</Section>

<style>
  .bad {
    color: var(--del-text);
  }
</style>
