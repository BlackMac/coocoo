<script lang="ts">
  // Head of the Stats and Workflows tabs: page tabs, period, project/model pickers, filter chips,
  // per-host sync status with refresh, and a banner for hosts whose numbers are missing.
  import { fleet } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import type { InsightHostSource } from '../lib/protocol';
  import { insights, RANGES } from './store.svelte';
  import ViewTabs from './ViewTabs.svelte';

  let {
    current,
    sources,
    models = [],
    loading = false,
    onrefresh,
  }: {
    current: 'stats' | 'workflows';
    /** Per-host sources of the shown report. */
    sources: InsightHostSource[];
    /** Model names for the model picker. */
    models?: string[];
    loading?: boolean;
    onrefresh: () => void;
  } = $props();

  const projects = $derived(fleet.projectList);
  const projectValue = $derived(insights.project ? `${insights.project.hostId}:${insights.project.id}` : '');
  const offline = $derived(sources.filter((s) => !s.ok));
  const statsMissing = $derived(sources.filter((s) => s.ok && s.statsError));

  function pickProject(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    const p = projects.find((x) => `${x.hostId}:${x.id}` === v);
    insights.project = p ? { hostId: p.hostId, id: p.id, name: p.name } : null;
  }

  function pickModel(e: Event) {
    insights.model = (e.currentTarget as HTMLSelectElement).value || null;
  }

  const dayLabel = $derived(insights.day === null ? '' : new Date(insights.day).toLocaleDateString(undefined, { month: 'short', day: 'numeric', weekday: 'short' }));
</script>

<div class="head">
  <ViewTabs {current} />
  <div class="filters">
    <div class="seg" role="group" aria-label="Period">
      {#each RANGES as r (r.id)}
        <button class:on={insights.day === null && insights.range === r.id} onclick={() => insights.setRange(r.id)}>{r.label}</button>
      {/each}
    </div>
    <select class="sel" value={projectValue} onchange={pickProject} aria-label="Project">
      <option value="">All projects</option>
      {#each projects as p (`${p.hostId}:${p.id}`)}
        <option value={`${p.hostId}:${p.id}`}>{p.name}{fleet.hostList.length > 1 ? ` · ${fleet.host(p.hostId)?.name ?? ''}` : ''}</option>
      {/each}
    </select>
    <select class="sel" value={insights.model ?? ''} onchange={pickModel} aria-label="Model">
      <option value="">All models</option>
      {#each models as m (m)}<option value={m}>{m}</option>{/each}
      {#if insights.model && !models.includes(insights.model)}<option value={insights.model}>{insights.model}</option>{/if}
    </select>
  </div>
  <div class="sync faint">
    {#if loading}<span class="spinner sm"></span>{/if}
    <span class="hosts">
      {#each sources.filter((s) => s.ok) as s, i (s.hostId)}
        {#if i > 0}<span> · </span>{/if}<span title={s.statsDb ?? ''}>{s.name} {s.syncedAt ? `synced ${ago(s.syncedAt, now.value)}` : 'not synced'}</span>
      {/each}
    </span>
    <button class="link" onclick={onrefresh} disabled={loading}>↻ Refresh</button>
  </div>
</div>

{#if insights.hasFilters}
  <div class="chips">
    {#if insights.day !== null}<span class="fchip">Day: {dayLabel} <button aria-label="Remove day filter" onclick={() => (insights.day = null)}>✕</button></span>{/if}
    {#if insights.project}<span class="fchip">Project: {insights.project.name} <button aria-label="Remove project filter" onclick={() => (insights.project = null)}>✕</button></span>{/if}
    {#if insights.model}<span class="fchip">Model: {insights.model} <button aria-label="Remove model filter" onclick={() => (insights.model = null)}>✕</button></span>{/if}
    <button class="link" onclick={() => insights.clearFilters()}>Clear all</button>
  </div>
{/if}

{#each offline as s (s.hostId)}
  <div class="banner warn">{s.name} is offline — its agents are missing from all numbers below.{#if s.error}<span class="faint"> ({s.error})</span>{/if}</div>
{/each}
{#each statsMissing as s (s.hostId)}
  <div class="banner warn">{s.name}: no token or cost numbers — {s.statsError}</div>
{/each}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 14px 18px;
    flex-wrap: wrap;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
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
  .sel {
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--r);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    padding: 4px 8px;
    max-width: 200px;
    min-width: 0;
  }
  .sync {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    min-width: 0;
  }
  .hosts {
    min-width: 0;
  }
  .link {
    all: unset;
    color: var(--accent-text);
    cursor: pointer;
    white-space: nowrap;
  }
  .link:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .chips {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: -14px;
    font-size: var(--fs-xs);
  }
  .fchip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--accent-soft);
    color: var(--accent-text);
    border-radius: 999px;
    padding: 2px 4px 2px 10px;
  }
  .fchip button {
    all: unset;
    cursor: pointer;
    opacity: 0.7;
    padding: 0 5px;
  }
  .fchip button:hover {
    opacity: 1;
  }
  .banner {
    padding: 8px 12px;
    border-radius: var(--r);
    font-size: var(--fs-sm);
    margin-top: -14px;
  }
  .banner.warn {
    background: var(--amber-soft);
    color: var(--amber);
  }
  .spinner.sm {
    width: 11px;
    height: 11px;
  }
  @media (max-width: 600px) {
    .sync {
      margin-left: 0;
      width: 100%;
      justify-content: space-between;
    }
    .sel {
      flex: 1;
    }
  }
</style>
