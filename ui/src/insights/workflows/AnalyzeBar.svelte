<script lang="ts">
  // Head of the Workflows tab: starts the pattern-analysis agent, shows its last run, and exports
  // the raw (content-free) usage log of the current filter.
  import { fleet } from '../../lib/fleet.svelte';
  import type { WorkflowReport } from '../../lib/protocol';
  import { href } from '../../lib/router.svelte';
  import { ui } from '../../lib/ui.svelte';
  import { count, shortDate } from '../format';
  import { insights } from '../store.svelte';
  import AnalyzeDialog from './AnalyzeDialog.svelte';

  let { report }: { report?: WorkflowReport } = $props();

  const PROJECT_KEY = 'coocoo.insights.analyzeProject';
  const EVENT_LIMIT = 100_000;

  let saved = $state(localStorage.getItem(PROJECT_KEY));
  let dialog = $state(false);
  let downloading = $state(false);

  const projects = $derived(fleet.projectList.filter((p) => fleet.host(p.hostId)?.online));
  /** Last chosen, else the project named `coocoo` (Coocoo's own repo), else the first. */
  const target = $derived(projects.find((p) => `${p.hostId}:${p.id}` === saved) ?? projects.find((p) => p.name === 'coocoo') ?? projects[0]);
  const last = $derived(report?.lastAnalyze);
  const lastProject = $derived(last ? fleet.project(last.hostId, last.projectId) : undefined);
  const lastAgent = $derived(last ? fleet.agent(last.hostId, last.agentId) : undefined);

  function remember(key: string) {
    saved = key;
    localStorage.setItem(PROJECT_KEY, key);
  }

  /** `2026-10-23` in local time. */
  function ymd(ts: number): string {
    const d = new Date(ts);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  async function download() {
    if (downloading) return;
    downloading = true;
    const query = insights.query;
    try {
      const r = await fleet.call('insights.events', { ...query, limit: EVENT_LIMIT }, undefined, 90_000);
      const body = r.events.map((e) => `${JSON.stringify(e)}\n`).join('');
      const from = query.from || r.events[0]?.ts || Date.now();
      const url = URL.createObjectURL(new Blob([body], { type: 'application/x-ndjson' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = `coocoo-events-${ymd(from)}-${ymd(query.to - 1)}.jsonl`;
      a.click();
      // The download has started reading the blob by the time this runs.
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      const offline = r.hosts.filter((h) => !h.ok).map((h) => h.name);
      const notes = [r.truncated ? `Only the first ${count(EVENT_LIMIT)} events.` : '', offline.length ? `Missing: ${offline.join(', ')} (offline).` : ''].filter(Boolean);
      ui.toast('success', `Downloaded ${r.events.length.toLocaleString()} events`, notes.join(' ') || undefined);
    } catch (err) {
      ui.error('Could not download the events', err);
    } finally {
      downloading = false;
    }
  }
</script>

<div class="card bar">
  <button class="btn primary" disabled={!target} onclick={() => (dialog = true)}>✦ Analyze patterns</button>
  <span class="what">
    {#if target}
      Starts an agent in <b>{target.name}</b> that reads these insights via <code>coocoo_insights</code> and files evidence-backed improvement cards into the Kanban inbox.
      It changes no code.
    {:else}
      Needs a project on an online host: the analysis agent runs there and files its cards into that project's Kanban inbox.
    {/if}
    {#if last}
      <span class="last">
        Last run: {shortDate(last.at)}{lastProject && lastProject.id !== target?.id ? ` in ${lastProject.name}` : ''} · {last.cards}
        {last.cards === 1 ? 'card' : 'cards'} ({last.accepted} accepted) ·
        {#if lastAgent}<a href={href({ name: 'agent', host: last.hostId, id: last.agentId })}>open agent</a>{:else}<span class="faint">agent deleted</span>{/if}
      </span>
    {:else}
      <span class="last faint">Not run yet.</span>
    {/if}
  </span>
  <button class="link" onclick={download} disabled={downloading} title="Every recorded event of the current filter, one JSON object per line">
    {#if downloading}<span class="spinner sm"></span>{/if} ⤓ Download events (JSONL)
  </button>
</div>

{#if dialog && target}
  <AnalyzeDialog {projects} initial="{target.hostId}:{target.id}" onclose={() => (dialog = false)} onstarted={remember} />
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 10px 14px;
    flex-wrap: wrap;
    padding: 14px 16px;
  }
  .what {
    flex: 1;
    min-width: 260px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .what b {
    color: var(--text);
    font-weight: 600;
  }
  code {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text);
  }
  .last {
    white-space: nowrap;
  }
  .last a {
    color: var(--accent-text);
    text-decoration: none;
  }
  .last a:hover {
    text-decoration: underline;
  }
  .link {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: var(--fs-sm);
    cursor: pointer;
    white-space: nowrap;
  }
  .link:hover {
    color: var(--text);
  }
  .link:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .spinner.sm {
    width: 11px;
    height: 11px;
  }
  @media (max-width: 600px) {
    .what {
      min-width: 0;
      flex-basis: 100%;
    }
    .last {
      white-space: normal;
    }
  }
</style>
