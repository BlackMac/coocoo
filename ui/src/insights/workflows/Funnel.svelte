<script lang="ts">
  // Stage funnel (agent journey, Kanban flow): how many reached each stage, how many of the
  // previous stage dropped off, and the median time it took. A stage opens the agents or cards
  // that reached it but not the next one.
  import type { FunnelStage } from '../../lib/protocol';
  import { pct, span } from '../format';
  import Examples from './Examples.svelte';

  let {
    stages,
    color,
    section,
    unit,
  }: {
    stages: FunnelStage[];
    color: string;
    /** Recorded with followed example links. */
    section: string;
    /** `agents` / `cards`, for the stuck list's title. */
    unit: string;
  } = $props();

  let open = $state<string | null>(null);
  const max = $derived(Math.max(1, ...stages.map((s) => s.count)));
  const openIndex = $derived(stages.findIndex((s) => s.id === open));
</script>

<div class="funnel">
  {#each stages as s, i (s.id)}
    {@const prev = i > 0 ? stages[i - 1] : undefined}
    {@const drop = prev && prev.count > 0 ? 1 - s.count / prev.count : 0}
    {@const next = stages[i + 1]}
    <button
      class="stage"
      class:on={open === s.id}
      disabled={!next || !s.stuck.length}
      aria-expanded={open === s.id}
      title={next ? `${s.count - next.count} ${unit} reached “${s.label}” but not “${next.label}”` : undefined}
      onclick={() => (open = open === s.id ? null : s.id)}
    >
      <span class="label ellipsis">{s.label}</span>
      <span class="track"><i style:width="{(100 * s.count) / max}%" style:background={color} style:opacity={1 - Math.min(i, 5) * 0.1}></i><b class="num" class:out={s.count / max < 0.12}>{s.count}</b></span>
      <span class="drop num faint">{drop > 0 ? `−${pct(drop)}` : ''}</span>
      <span class="time num faint">{prev?.medianToNextMs != null ? `→ ${span(prev.medianToNextMs)}` : ''}</span>
    </button>
  {/each}
  {#if openIndex >= 0 && stages[openIndex + 1]}
    {@const s = stages[openIndex]}
    <div class="stuck">
      <Examples examples={s.stuck} {section} title="Reached “{s.label}” but not “{stages[openIndex + 1].label}” — newest first, up to 20 {unit}:" />
    </div>
  {/if}
</div>

<style>
  .funnel {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .stage {
    all: unset;
    display: grid;
    grid-template-columns: 130px minmax(0, 1fr) 44px 64px;
    gap: 10px;
    align-items: center;
    padding: 3px 4px;
    border-radius: var(--r-sm);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .stage:disabled {
    cursor: default;
  }
  .stage:not(:disabled):hover,
  .stage.on {
    background: var(--bg-hover);
  }
  .stage:focus-visible {
    outline: 2px solid var(--accent);
  }
  .track {
    position: relative;
    height: 18px;
    background: var(--bg-3);
    border-radius: 3px;
    overflow: hidden;
  }
  .track i {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 3px;
  }
  .track b {
    position: absolute;
    left: 8px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--fs-xs);
    font-weight: 500;
    color: #fff;
  }
  .track b.out {
    color: var(--text);
  }
  .drop,
  .time {
    font-size: var(--fs-xs);
    text-align: right;
    white-space: nowrap;
  }
  .stuck {
    margin-top: 8px;
    padding: 10px 12px;
    background: var(--bg-2);
    border-radius: var(--r);
  }
  @media (max-width: 600px) {
    .stage {
      grid-template-columns: 92px minmax(0, 1fr) 38px 56px;
      gap: 6px;
    }
  }
</style>
