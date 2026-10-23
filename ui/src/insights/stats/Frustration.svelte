<script lang="ts">
  // Frustration: share of your prompts that omp flagged, per day. Clicking a day lists its flagged
  // prompts with a link to that moment in the agent's history.
  import type { FlaggedPrompt, FrustrationSignals, UsageReport } from '../../lib/protocol';
  import { href } from '../../lib/router.svelte';
  import { track } from '../../lib/track';
  import { pct, shortDate } from '../format';
  import { addDays } from '../store.svelte';
  import Section from './Section.svelte';
  import Tip from './Tip.svelte';
  import { agentTitle, labelEvery, niceMax } from './util';

  const H = 90;
  const PL = 34;
  const PR = 6;
  const PB = 16;
  const PT = 6;
  const SIGNALS: Record<keyof FrustrationSignals, string> = {
    yelling: 'CAPS',
    profanity: 'swearing',
    anguish: 'anguish',
    negation: 'negation',
    repetition: 'repetition',
    blame: 'blame',
  };
  const signalLabel = (s: string) => SIGNALS[s as keyof FrustrationSignals] ?? s;

  let { report }: { report: UsageReport } = $props();

  let width = $state(0);
  let hover = $state<{ i: number; x: number; y: number } | null>(null);
  let picked = $state<number | null>(null);

  const f = $derived(report.frustration);
  const days = $derived(report.days);
  const share = (i: number) => (days[i].prompts ? days[i].flagged / days[i].prompts : 0);
  const max = $derived(niceMax(Math.max(0, ...days.map((_, i) => share(i)))));
  const step = $derived(days.length > 1 ? Math.max(0, width - PL - PR) / (days.length - 1) : 0);
  const x = (i: number) => (days.length > 1 ? PL + i * step : PL + Math.max(0, width - PL - PR) / 2);
  const y = (v: number) => PT + (H - PB - PT) * (1 - v / max);
  const every = $derived(labelEvery(step || width));
  const points = $derived(days.flatMap((d, i) => (d.prompts ? [{ i, x: x(i), y: y(share(i)) }] : [])));
  const peak = $derived.by(() => {
    let best = -1;
    days.forEach((d, i) => {
      if (d.flagged && (best < 0 || share(i) > share(best))) best = i;
    });
    return best;
  });

  /** The day whose prompts are listed: the clicked one, or the only one when a day filter is set. */
  const day = $derived(picked !== null && days.some((d) => d.start === picked) ? picked : days.length === 1 ? days[0].start : null);
  const examples = $derived.by((): FlaggedPrompt[] => {
    if (day === null) return [];
    const end = addDays(day, 1);
    return f.examples.filter((e) => e.ts >= day && e.ts < end);
  });
  const dayRow = $derived(days.find((d) => d.start === day));

  function index(e: MouseEvent): number {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    const i = days.length > 1 ? Math.round((e.clientX - r.left - PL) / step) : 0;
    return i >= 0 && i < days.length ? i : -1;
  }
  function move(e: MouseEvent) {
    const i = index(e);
    hover = i < 0 ? null : { i, x: e.clientX, y: e.clientY };
  }
  function pick(e: MouseEvent) {
    const i = index(e);
    if (i < 0) return;
    picked = picked === days[i].start ? null : days[i].start;
    hover = null;
  }
  const time = (ts: number) => new Date(ts).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
</script>

<Section title="Frustration" note="share of flagged prompts · click a day">
  {#snippet help()}
    omp checks every prompt you send for frustration signals: <b>CAPS</b> (yelling), <b>swearing</b>, <b>anguish</b>, <b>negation</b>
    (“no, not that”), <b>repetition</b> (repeating an earlier prompt) and <b>blame</b>. A prompt with at least one signal is flagged.
    Share per day = flagged prompts / prompts. Source: omp's <code>~/.omp/stats.db</code>; the prompt text shown here comes from there
    and is never written to Coocoo's usage log.
  {/snippet}
  {#if f.prompts === 0}
    <div class="none faint">No prompts in this period.</div>
  {:else}
    <div class="chart" bind:clientWidth={width}>
      {#if width > 0}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
        <svg {width} height={H} onmousemove={move} onmouseleave={() => (hover = null)} onclick={pick}>
          {#each [0, max] as v (v)}
            <line x1={PL} x2={width - PR} y1={y(v)} y2={y(v)} class="grid" />
            <text x={PL - 5} y={y(v) + 3} text-anchor="end">{pct(v, 0)}</text>
          {/each}
          {#each days as d, i (d.index)}
            {#if i % every === 0}<text x={x(i)} y={H - 3} text-anchor="middle">{shortDate(d.start)}</text>{/if}
          {/each}
          {#if day !== null}
            {@const i = days.findIndex((d) => d.start === day)}
            {#if i >= 0}<line x1={x(i)} x2={x(i)} y1={PT} y2={H - PB} class="mark" />{/if}
          {/if}
          <polyline points={points.map((p) => `${p.x},${p.y}`).join(' ')} class="line" />
          {#each points as p (p.i)}
            <circle cx={p.x} cy={p.y} r={days[p.i].start === day || hover?.i === p.i ? 4 : 2.4} class="dot" />
          {/each}
          <rect class="hit" x={PL - step / 2} y={0} width={Math.max(0, width - PL - PR + step)} height={H} />
        </svg>
      {/if}
    </div>
    <div class="legend">
      <span class="lead">{pct(f.flagged / f.prompts)} of prompts flagged <span class="faint">({f.flagged} of {f.prompts})</span></span>
      {#each Object.entries(SIGNALS) as [k, label] (k)}
        <span>{label} {f.signals[k as keyof FrustrationSignals]}</span>
      {/each}
      {#if peak >= 0 && day === null}<span class="faint">peak {shortDate(days[peak].start)} — click a day for its prompts</span>{/if}
    </div>

    {#if day !== null}
      <div class="examples">
        <div class="ehead">
          <b>{new Date(day).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}</b>
          <span class="faint">{dayRow?.flagged ?? 0} of {dayRow?.prompts ?? 0} prompts flagged</span>
          {#if picked !== null}<button class="link" onclick={() => (picked = null)}>Close</button>{/if}
        </div>
        {#each examples as e, k (k)}
          <div class="ex">
            <div class="meta">
              <span class="faint num">{time(e.ts)}</span>
              {#if e.agentId}
                <a
                  class="link agent"
                  href={href({ name: 'agent', host: e.hostId, id: e.agentId, at: e.ts })}
                  onclick={() => track('example', { agentId: e.agentId, meta: { section: 'stats' } })}>{agentTitle(report, e.hostId, e.agentId)}</a
                >
              {/if}
              {#each e.signals as s (s)}<span class="sig">{signalLabel(s)}</span>{/each}
            </div>
            <div class="text">{e.text}</div>
          </div>
        {:else}
          <div class="faint small">
            {dayRow?.flagged ? `Only the newest ${f.examples.length} flagged prompts are kept for the list; none of them is from this day.` : 'No flagged prompts on this day.'}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</Section>

{#if hover && days[hover.i]}
  {@const d = days[hover.i]}
  <Tip x={hover.x} y={hover.y}>
    <b>{new Date(d.start).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}</b>
    {#if d.prompts}
      <div class="row">Flagged<b>{d.flagged} of {d.prompts}</b></div>
      <div class="row">Share<b>{pct(share(hover.i))}</b></div>
      <div class="foot">Click to list the flagged prompts</div>
    {:else}
      <div class="foot">No prompts</div>
    {/if}
  </Tip>
{/if}

<style>
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
  .mark {
    stroke: var(--amber);
    stroke-dasharray: 2 3;
    opacity: 0.6;
  }
  .line {
    fill: none;
    stroke: var(--amber);
    stroke-width: 1.6;
  }
  .dot {
    fill: var(--amber);
  }
  .hit {
    fill: transparent;
  }
  .none {
    padding: 18px 0 8px;
    text-align: center;
    font-size: var(--fs-sm);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin-top: 8px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .lead {
    color: var(--text);
  }
  .examples {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 360px;
    overflow-y: auto;
  }
  .ehead {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: var(--fs-sm);
  }
  .ehead .link {
    margin-left: auto;
  }
  .ex {
    min-width: 0;
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
    font-size: var(--fs-xs);
    min-width: 0;
  }
  .link {
    all: unset;
    color: var(--accent-text);
    cursor: pointer;
    max-width: 100%;
  }
  .link:hover {
    text-decoration: underline;
  }
  .agent {
    display: inline-block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .sig {
    padding: 0 6px;
    border-radius: 99px;
    background: var(--amber-soft);
    color: var(--amber);
    font-size: 10px;
    line-height: 16px;
  }
  .text {
    margin-top: 2px;
    font-size: var(--fs-sm);
    color: var(--text-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .small {
    font-size: var(--fs-xs);
  }
</style>
