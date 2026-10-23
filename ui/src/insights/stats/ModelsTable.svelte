<script lang="ts">
  // Models: requests, cost, cache hit, latency and errors per model; a row sets the model filter.
  import type { InsightModelRow, UsageReport } from '../../lib/protocol';
  import { count, modelColor, pct, usd } from '../format';
  import { insights } from '../store.svelte';
  import Section from './Section.svelte';
  import Table from './Table.svelte';
  import { colorOrder, seconds } from './util';

  let { report }: { report: UsageReport } = $props();

  const order = $derived(colorOrder(report));
  const maxCost = $derived(Math.max(0, ...report.models.map((m) => m.cost.total)));

  const cols = [
    { label: 'Model' },
    { label: 'Requests', title: 'Assistant responses by this model' },
    { label: 'Cost', title: 'API equivalent at list prices' },
    { label: 'Cache hit', title: 'cache read / (input + cache read + cache write)' },
    { label: 'TTFT', hm: true, title: 'Mean time to first token' },
    { label: 'tok/s', hm: true, title: 'Mean output tokens per second of generation' },
    { label: 'Errors', title: 'Responses that ended with an error / requests' },
  ];

  function pick(m: InsightModelRow) {
    insights.model = insights.model === m.model ? null : m.model;
  }
</script>

<Section title="Models" note="click a row to filter">
  {#snippet help()}
    Per model, over the agents, their subagents and advisor. <b>Cost</b>: API equivalent at list prices. <b>Cache hit</b> = cache read /
    (input + cache read + cache write) tokens. <b>TTFT</b>: mean time to the first token. <b>tok/s</b>: output tokens per second of
    generation (response time minus TTFT), averaged per response. <b>Errors</b> = responses with stop reason error / requests. Source:
    <code>~/.omp/stats.db</code>.
  {/snippet}
  <Table {cols} rows={report.models} key={(m) => `${m.provider}/${m.model}`} onrow={pick} active={(m) => insights.model === m.model} empty="No requests in this period.">
    {#snippet row(m: InsightModelRow)}
      <td class="t">
        <span class="name"><i class="sw" style:background={modelColor(m.model, order)}></i>{m.model}</span>
        <span class="prov faint">{m.provider}</span>
      </td>
      <td data-l="req">{count(m.requests)}</td>
      <td class="bar"><i style:width="{maxCost ? (100 * m.cost.total) / maxCost : 0}%"></i><span>{usd(m.cost.total)}</span></td>
      <td data-l="cache hit">{pct(m.cacheHit)}</td>
      <td class="hm">{seconds(m.ttft)}</td>
      <td class="hm">{m.tps == null ? '—' : Math.round(m.tps)}</td>
      <td data-l="errors" class:bad={m.requests > 0 && m.failed / m.requests > 0.05}>{pct(m.requests ? m.failed / m.requests : undefined)}</td>
    {/snippet}
  </Table>
</Section>

<style>
  .name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .prov {
    display: block;
    margin-left: 14px;
    font-size: 10px;
  }
  .bad {
    color: var(--del-text);
  }
  @media (max-width: 600px) {
    .prov {
      display: inline;
      margin-left: 6px;
    }
  }
</style>
