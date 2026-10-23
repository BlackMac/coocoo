<script lang="ts">
  // Kanban throughput: cards that entered Done per day. Cards whose done time is only known from
  // their last update (done before Coocoo recorded it) are drawn lighter.
  import type { UsageReport } from '../../lib/protocol';
  import { shortDate } from '../format';
  import Section from './Section.svelte';
  import Tip from './Tip.svelte';
  import { labelEvery, niceMax } from './util';

  const H = 120;
  const PL = 26;
  const PB = 18;
  const PT = 6;

  let { report }: { report: UsageReport } = $props();

  let width = $state(0);
  let hover = $state<{ i: number; x: number; y: number } | null>(null);

  const days = $derived(report.days);
  const total = $derived(days.reduce((s, d) => s + d.cardsDone, 0));
  const approx = $derived(days.reduce((s, d) => s + d.cardsDoneApprox, 0));
  const max = $derived(niceMax(Math.max(0, ...days.map((d) => d.cardsDone)), true));
  const y = (v: number) => PT + (H - PB - PT) * (1 - v / max);
  const bw = $derived(days.length ? Math.max(0, width - PL) / days.length : 0);
  const gap = $derived(Math.min(4, bw * 0.25));
  const every = $derived(labelEvery(bw));

  function move(e: MouseEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    const i = Math.floor((e.clientX - r.left - PL) / bw);
    hover = i >= 0 && i < days.length ? { i, x: e.clientX, y: e.clientY } : null;
  }
</script>

<Section title="Kanban throughput" note="cards moved to Done per day">
  {#snippet help()}
    Kanban cards that entered Done per day (your time zone), from the boards of the projects in the filter. Lighter part: cards whose
    done time is approximated by their last update, because they were finished before Coocoo recorded it.
  {/snippet}
  <div class="chart" bind:clientWidth={width}>
    {#if total === 0}
      <div class="none faint">No cards finished in this period.</div>
    {:else if width > 0}
      <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
      <svg {width} height={H} onmousemove={move} onmouseleave={() => (hover = null)} role="img" aria-label="{total} cards done">
        {#each [0, 3] as g (g)}
          {@const v = (max * g) / 3}
          <line x1={PL} x2={width} y1={y(v)} y2={y(v)} class="grid" />
          <text x={PL - 5} y={y(v) + 3} text-anchor="end">{v}</text>
        {/each}
        {#each days as d, i (d.index)}
          {@const exact = d.cardsDone - d.cardsDoneApprox}
          {#if d.cardsDone > 0}
            <rect x={PL + i * bw + gap / 2} y={y(exact)} width={Math.max(0.5, bw - gap)} height={y(0) - y(exact)} fill="var(--green)" rx={bw > 6 ? 1.5 : 0} />
            {#if d.cardsDoneApprox > 0}
              <rect x={PL + i * bw + gap / 2} y={y(d.cardsDone)} width={Math.max(0.5, bw - gap)} height={y(exact) - y(d.cardsDone)} fill="var(--green)" opacity="0.4" rx={bw > 6 ? 1.5 : 0} />
            {/if}
          {/if}
          {#if i % every === 0}
            <text x={PL + i * bw + bw / 2} y={H - 4} text-anchor="middle">{shortDate(d.start)}</text>
          {/if}
        {/each}
      </svg>
    {/if}
  </div>
  {#if total > 0}
    <div class="legend">
      <span><b>{total}</b> cards done</span>
      {#if approx > 0}<span><i class="sw approx"></i>{approx} with approximate done time</span>{/if}
    </div>
  {/if}
</Section>

{#if hover && days[hover.i]}
  {@const d = days[hover.i]}
  <Tip x={hover.x} y={hover.y}>
    <b>{new Date(d.start).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}</b>
    <div class="row">Cards done<b>{d.cardsDone}</b></div>
    {#if d.cardsDoneApprox > 0}<div class="row">of which approximate<b>{d.cardsDoneApprox}</b></div>{/if}
  </Tip>
{/if}

<style>
  .chart {
    width: 100%;
    min-width: 0;
  }
  svg {
    display: block;
  }
  svg text {
    fill: var(--text-3);
    font-size: 10px;
    font-family: var(--font);
  }
  .grid {
    stroke: var(--border);
  }
  .none {
    display: grid;
    place-items: center;
    height: 90px;
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
  }
  .legend b {
    font-weight: 600;
    color: var(--text);
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  .approx {
    background: var(--green);
    opacity: 0.4;
  }
</style>
