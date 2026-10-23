<script lang="ts">
  // Stats tab: what the Coocoo agents (with their subagents and advisor) cost and produced — per
  // day, model, project, agent and tool — from omp's statistics of every host plus Coocoo's agent
  // ledger and git. Every section reads the one report of the current filter, so a chip set by
  // clicking a bar or row narrows all of them.
  import { untrack } from 'svelte';
  import FilterBar from '../insights/FilterBar.svelte';
  import { insights } from '../insights/store.svelte';
  import AgentsTable from '../insights/stats/AgentsTable.svelte';
  import DayChart from '../insights/stats/DayChart.svelte';
  import Frustration from '../insights/stats/Frustration.svelte';
  import Heatmap from '../insights/stats/Heatmap.svelte';
  import HowComputed from '../insights/stats/HowComputed.svelte';
  import KanbanChart from '../insights/stats/KanbanChart.svelte';
  import ModelsTable from '../insights/stats/ModelsTable.svelte';
  import ProjectsTable from '../insights/stats/ProjectsTable.svelte';
  import StatsKpis from '../insights/stats/StatsKpis.svelte';
  import ToolsTable from '../insights/stats/ToolsTable.svelte';

  const usage = $derived(insights.usage);
  const report = $derived(usage.data);
  const hasData = $derived.by(() => {
    if (!report) return false;
    const t = report.totals;
    return t.requests > 0 || t.prompts > 0 || t.commits > 0 || t.cardsDone > 0 || report.agents.length > 0;
  });

  // Refetch whenever the filter changes; the store caches per filter and drops stale answers.
  $effect(() => {
    void insights.query;
    untrack(() => insights.loadUsage());
  });
</script>

<div class="scroll">
  <div class="page">
    <FilterBar current="stats" sources={report?.hosts ?? []} models={report?.allModels ?? []} loading={usage.loading} onrefresh={() => insights.loadUsage(true)} />

    {#if usage.error && !usage.loading}
      <div class="banner err">
        <span>Could not load the stats: {usage.error}{report ? ' — showing the last result.' : ''}</span>
        <button class="link" onclick={() => insights.loadUsage(true)}>Retry</button>
      </div>
    {/if}

    {#if report}
      <div class="body" class:stale={usage.loading} aria-busy={usage.loading}>
        <StatsKpis {report} />
        {#if hasData}
          <DayChart {report} />
          <div class="grid2">
            <ModelsTable {report} />
            <ProjectsTable {report} />
          </div>
          <AgentsTable {report} />
          <div class="grid2">
            <ToolsTable {report} />
            <Heatmap {report} />
          </div>
          <div class="grid2">
            <KanbanChart {report} />
            <Frustration {report} />
          </div>
        {:else}
          <div class="card empty">
            <strong>No agent activity in this period</strong>
            <span>No requests, prompts, commits or finished cards{insights.hasFilters ? ' match the filters' : ''}.</span>
            {#if insights.hasFilters}<button class="btn sm" onclick={() => insights.clearFilters()}>Clear filters</button>{/if}
          </div>
        {/if}
        <HowComputed />
      </div>
    {:else if usage.loading}
      <div class="card empty">
        <span class="spinner"></span>
        <strong>Loading stats…</strong>
        <span>Each host syncs omp's statistics first; this can take up to a minute.</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .scroll {
    flex: 1;
    overflow-y: auto;
  }
  .page {
    max-width: 1280px;
    margin: 0 auto;
    padding: 20px 24px 48px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
    transition: opacity var(--t) var(--ease);
  }
  .body.stale {
    opacity: 0.55;
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
  }
  .banner {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 8px 12px;
    border-radius: var(--r);
    font-size: var(--fs-sm);
    margin-top: -4px;
  }
  .banner.err {
    background: var(--red-soft);
    color: var(--del-text);
  }
  .link {
    all: unset;
    margin-left: auto;
    color: var(--accent-text);
    cursor: pointer;
  }
  @media (max-width: 900px) {
    .grid2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (max-width: 600px) {
    .page {
      padding: 16px 12px 40px;
      gap: 16px;
    }
    .body {
      gap: 14px;
    }
  }
</style>
