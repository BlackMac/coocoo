<script lang="ts">
  // "When you work": prompts by local weekday × hour.
  import type { UsageReport } from '../../lib/protocol';
  import Section from './Section.svelte';

  const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const HOURS = Array.from({ length: 24 }, (_, h) => h);

  let { report }: { report: UsageReport } = $props();

  const cell = (d: number, h: number) => report.heatmap[d]?.[h] ?? 0;
  const max = $derived(Math.max(0, ...report.heatmap.flat()));
</script>

<Section title="When you work" note="prompts by weekday × hour (local time)">
  {#snippet help()}
    Prompts you sent to agents, by weekday and hour in your time zone. Darker = more prompts (relative to the busiest hour, {max}
    prompts). Hover a cell for its count.
  {/snippet}
  {#if max > 0}
    <div class="heat">
      <span></span>
      {#each HOURS as h (h)}<span class="hr">{h % 3 === 0 ? h : ''}</span>{/each}
      {#each WEEKDAYS as day, d (day)}
        <span class="wd">{day}</span>
        {#each HOURS as h (h)}
          {@const n = cell(d, h)}
          <span class="c" title="{day} {h}:00–{h + 1}:00 · {n} {n === 1 ? 'prompt' : 'prompts'}"><i style:opacity={n ? 0.15 + (0.85 * n) / max : 0.05}></i></span>
        {/each}
      {/each}
    </div>
  {:else}
    <div class="none faint">No prompts in this period.</div>
  {/if}
</Section>

<style>
  .heat {
    display: grid;
    grid-template-columns: 28px repeat(24, minmax(0, 1fr));
    gap: 2px;
    font-size: 10px;
    color: var(--text-3);
  }
  .hr {
    text-align: left;
    line-height: 14px;
  }
  .wd {
    line-height: 14px;
  }
  .c {
    height: 14px;
    border-radius: 2px;
    background: var(--bg-2);
    overflow: hidden;
  }
  .c i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .none {
    padding: 18px 0 8px;
    text-align: center;
    font-size: var(--fs-sm);
  }
  @media (max-width: 600px) {
    .heat {
      grid-template-columns: 26px repeat(24, minmax(0, 1fr));
      gap: 1px;
    }
  }
</style>
