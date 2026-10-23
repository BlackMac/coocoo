<script lang="ts">
  // "Per day": stacked bars by model for cost, tokens or requests (commits as one bar). Hover shows
  // the per-model values; clicking a bar narrows every section to that day (click again clears).
  import type { InsightDay, UsageReport } from '../../lib/protocol';
  import { count, modelColor, shortDate, usd } from '../format';
  import { insights } from '../store.svelte';
  import Section from './Section.svelte';
  import Tip from './Tip.svelte';
  import { colorOrder, labelEvery, niceMax } from './util';

  type Metric = 'cost' | 'tokens' | 'requests' | 'commits';
  const METRICS: { id: Metric; label: string }[] = [
    { id: 'cost', label: 'Cost' },
    { id: 'tokens', label: 'Tokens' },
    { id: 'requests', label: 'Requests' },
    { id: 'commits', label: 'Commits' },
  ];
  const PL = 44;
  const PB = 20;
  const PT = 8;

  let { report }: { report: UsageReport } = $props();

  let metric = $state<Metric>('cost');
  let width = $state(0);
  let hover = $state<{ i: number; x: number; y: number } | null>(null);

  const H = $derived(width < 500 ? 170 : 200);
  const days = $derived(report.days);
  const order = $derived(colorOrder(report));
  /** Stack order, bottom first: the models by cost (as in the Models table). */
  const keys = $derived.by(() => {
    const k = [...new Set(report.models.map((m) => m.model))];
    for (const d of days) for (const m of Object.keys(d.models)) if (!k.includes(m)) k.push(m);
    return k;
  });
  const legend = $derived(keys.filter((k) => days.some((d) => d.models[k])));

  function value(d: InsightDay, model: string): number {
    const m = d.models[model];
    if (!m) return 0;
    return metric === 'cost' ? m.cost : metric === 'tokens' ? m.tokens : m.requests;
  }
  function total(d: InsightDay): number {
    return metric === 'commits' ? d.commits : keys.reduce((s, k) => s + value(d, k), 0);
  }
  const fmt = (v: number) => (metric === 'cost' ? usd(v) : count(v));

  const totals = $derived(days.map(total));
  const max = $derived(niceMax(Math.max(0, ...totals), metric !== 'cost'));
  const y = (v: number) => PT + (H - PB - PT) * (1 - v / max);
  const bw = $derived(days.length ? Math.max(0, width - PL) / days.length : 0);
  const gap = $derived(Math.min(4, bw * 0.25));
  const every = $derived(labelEvery(bw));

  const bars = $derived(
    days.map((d) => {
      const parts = metric === 'commits' ? [{ color: 'var(--green)', v: d.commits }] : keys.map((k) => ({ color: modelColor(k, order), v: value(d, k) }));
      let base = 0;
      return parts
        .filter((p) => p.v > 0)
        .map((p) => {
          const top = base + p.v;
          const seg = { color: p.color, y: y(top), h: Math.max(0, y(base) - y(top)) };
          base = top;
          return seg;
        });
    }),
  );

  function at(e: MouseEvent): number {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    const i = Math.floor((e.clientX - r.left - PL) / bw);
    return i >= 0 && i < days.length ? i : -1;
  }
  function move(e: MouseEvent) {
    const i = at(e);
    hover = i < 0 ? null : { i, x: e.clientX, y: e.clientY };
  }
  function pick(e: MouseEvent) {
    const i = at(e);
    if (i < 0) return;
    const start = days[i].start;
    insights.day = insights.day === start ? null : start;
    hover = null;
  }
  const longDate = (ts: number) => new Date(ts).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
</script>

<Section title="Per day">
  {#snippet actions()}
    <span class="seg" role="group" aria-label="Metric">
      {#each METRICS as m (m.id)}<button class:on={metric === m.id} onclick={() => (metric = m.id)}>{m.label}</button>{/each}
    </span>
  {/snippet}
  {#snippet help()}
    {#if metric === 'cost'}Cost per day (API equivalent at list prices), stacked by model.
    {:else if metric === 'tokens'}Tokens per day (input + output + cache read + cache write), stacked by model.
    {:else if metric === 'requests'}Requests (assistant responses) per day, stacked by model.
    {:else}Commits attributed to agents (exact or during one of their turns), by commit time.{/if}
    Days in your time zone. Click a bar to narrow every section to that day; click it again to clear.
  {/snippet}

  <div class="chart" bind:clientWidth={width}>
    {#if totals.every((v) => v === 0)}
      <div class="none faint" style:height="{H}px">No {METRICS.find((m) => m.id === metric)?.label.toLowerCase()} in this period.</div>
    {:else if width > 0}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
      <svg {width} height={H} onmousemove={move} onmouseleave={() => (hover = null)} onclick={pick}>
        {#each [0, 1, 2, 3] as g (g)}
          {@const v = (max * g) / 3}
          <line x1={PL} x2={width} y1={y(v)} y2={y(v)} class="grid" />
          <text x={PL - 6} y={y(v) + 3} text-anchor="end">{fmt(v)}</text>
        {/each}
        {#each bars as segs, i (days[i].index)}
          {@const dim = insights.day !== null && insights.day !== days[i].start}
          <g opacity={dim ? 0.3 : 1}>
            {#each segs as s, j (j)}
              <rect x={PL + i * bw + gap / 2} y={s.y} width={Math.max(0.5, bw - gap)} height={s.h} fill={s.color} rx={bw > 6 ? 1.5 : 0} />
            {/each}
          </g>
          {#if i % every === 0}
            <text x={PL + i * bw + bw / 2} y={H - 5} text-anchor="middle">{shortDate(days[i].start)}</text>
          {/if}
        {/each}
        <rect class="hit" x={PL} y={PT} width={Math.max(0, width - PL)} height={H - PB - PT} />
      </svg>
    {/if}
  </div>
  {#if metric !== 'commits' && legend.length}
    <div class="legend">
      {#each legend as m (m)}<span><i class="sw" style:background={modelColor(m, order)}></i>{m}</span>{/each}
    </div>
  {/if}
</Section>

{#if hover && days[hover.i]}
  {@const d = days[hover.i]}
  <Tip x={hover.x} y={hover.y}>
    <b>{longDate(d.start)}</b>
    <div class="rows">
      {#if metric === 'commits'}
        <div class="row">{d.commits} {d.commits === 1 ? 'commit' : 'commits'}</div>
      {:else}
        {#each keys.filter((k) => value(d, k) > 0) as k (k)}
          <div class="row"><i class="sw" style:background={modelColor(k, order)}></i>{k}<b>{fmt(value(d, k))}</b></div>
        {/each}
        <div class="row total">Total<b>{fmt(totals[hover.i])}</b></div>
      {/if}
    </div>
    <div class="foot">{insights.day === d.start ? 'Click to clear the day filter' : 'Click to filter to this day'}</div>
  </Tip>
{/if}

<style>
  .seg {
    display: inline-flex;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--r);
    padding: 2px;
  }
  .seg button {
    all: unset;
    padding: 3px 10px;
    border-radius: var(--r-sm);
    color: var(--text-2);
    cursor: pointer;
    font-size: var(--fs-sm);
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button.on {
    background: var(--bg-3);
    color: var(--text);
  }
  .chart {
    width: 100%;
    min-width: 0;
  }
  svg {
    display: block;
    cursor: pointer;
  }
  svg text {
    fill: var(--text-3);
    font-size: 10px;
    font-family: var(--font);
  }
  .grid {
    stroke: var(--border);
  }
  .hit {
    fill: transparent;
  }
  .none {
    display: grid;
    place-items: center;
    font-size: var(--fs-sm);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin-top: 6px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .rows {
    margin: 4px 0 2px;
  }
  .total {
    margin-top: 2px;
    padding-top: 2px;
    border-top: 1px solid var(--border-strong);
  }
  @media (max-width: 600px) {
    .seg button {
      padding: 3px 8px;
    }
  }
</style>
