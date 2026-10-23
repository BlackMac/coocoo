<script lang="ts">
  // Starts the pattern-analysis agent (`insights.analyze` on the project's host) and opens it.
  import { untrack } from 'svelte';
  import { fleet } from '../../lib/fleet.svelte';
  import type { Project } from '../../lib/protocol';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/ui.svelte';
  import { track } from '../../lib/track';
  import Dialog from '../../components/Dialog.svelte';
  import { shortDate } from '../format';
  import { addDays, insights, midnight } from '../store.svelte';

  let {
    projects,
    initial,
    onclose,
    onstarted,
  }: {
    /** Projects on online hosts. */
    projects: Project[];
    /** `host:id` preselected. */
    initial: string;
    onclose: () => void;
    /** The project the analysis ran in (`host:id`), remembered as the next default. */
    onstarted: (key: string) => void;
  } = $props();

  // Not a `ui.dialog` (that union belongs to the app shell), so it reports itself to the usage log.
  const DIALOG = { meta: { dialog: 'analyze' } };
  track('dialog.open', DIALOG);

  function cancel() {
    track('dialog.cancel', DIALOG);
    onclose();
  }

  type PeriodId = 'filter' | '7d' | '30d' | '90d';
  const FIXED: { id: Exclude<PeriodId, 'filter'>; label: string; days: number }[] = [
    { id: '7d', label: 'Last 7 days', days: 7 },
    { id: '30d', label: 'Last 30 days', days: 30 },
    { id: '90d', label: 'Last 90 days', days: 90 },
  ];

  // The dialog seeds from the filter once; changing the filter behind it does not move the form.
  const filter = untrack(() => insights.query);
  const filterIsFixed = untrack(() => insights.day === null && FIXED.some((f) => f.id === insights.range));
  const filterLabel = filter.from ? `${shortDate(filter.from)} – ${shortDate(filter.to - 1)}` : 'All time';

  let target = $state(untrack(() => initial));
  let period = $state<PeriodId>(untrack(() => (filterIsFixed ? (insights.range as PeriodId) : 'filter')));
  let focus = $state('');
  let busy = $state(false);

  const project = $derived(projects.find((p) => `${p.hostId}:${p.id}` === target));
  const multiHost = $derived(fleet.hostList.length > 1);

  function bounds(): { from: number; to: number; tzOffset: number } {
    const fixed = FIXED.find((f) => f.id === period);
    if (!fixed) return { from: filter.from, to: filter.to, tzOffset: filter.tzOffset };
    const today = midnight(Date.now());
    const from = addDays(today, 1 - fixed.days);
    return { from, to: addDays(today, 1), tzOffset: -new Date(from).getTimezoneOffset() };
  }

  async function start(e?: SubmitEvent) {
    e?.preventDefault();
    if (busy || !project) return;
    busy = true;
    const p = project;
    try {
      const a = await fleet.call('insights.analyze', { projectId: p.id, ...bounds(), focus: focus.trim() || undefined }, p.hostId, 60_000);
      fleet.agents[`${a.hostId}:${a.id}`] = a;
      track('dialog.submit', DIALOG);
      onstarted(target);
      onclose();
      router.go({ name: 'agent', host: a.hostId, id: a.id });
      ui.toast('success', 'Pattern analysis started', `It files its findings as cards into the ${p.name} inbox.`);
    } catch (err) {
      ui.error('Could not start the analysis', err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Analyze patterns" width={520} onclose={cancel}>
  <form class="form" id="analyze-patterns" onsubmit={start}>
    <p class="intro">
      An agent reads the insights of the chosen period via <code>coocoo_insights</code> and files evidence-backed improvement cards into the project's Kanban
      inbox. It changes no code.
    </p>
    <label class="field">
      <span>Project <span class="faint">(the agent runs here; its cards go to this inbox)</span></span>
      <select class="select" bind:value={target}>
        {#each projects as p (`${p.hostId}:${p.id}`)}
          <option value="{p.hostId}:{p.id}">{p.name}{multiHost ? ` · ${fleet.host(p.hostId)?.name ?? p.hostId}` : ''}</option>
        {/each}
      </select>
    </label>
    <div class="field">
      <span>Period</span>
      <div class="seg" role="group" aria-label="Period">
        {#if !filterIsFixed}
          <button type="button" class:on={period === 'filter'} onclick={() => (period = 'filter')}>{filterLabel}</button>
        {/if}
        {#each FIXED as f (f.id)}
          <button type="button" class:on={period === f.id} onclick={() => (period = f.id)}>{f.label}</button>
        {/each}
      </div>
    </div>
    <label class="field">
      <span>Focus <span class="faint">(optional)</span></span>
      <textarea class="textarea" rows="3" bind:value={focus} placeholder="e.g. why Kanban cards get moved back from Doing"></textarea>
    </label>
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={cancel}>Cancel</button>
    <button class="btn primary" type="submit" form="analyze-patterns" disabled={busy || !project}>
      {#if busy}<span class="spinner"></span>{/if} Start analysis
    </button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .intro {
    margin: 0;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  code {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text);
  }
  .seg {
    display: inline-flex;
    flex-wrap: wrap;
    align-self: flex-start;
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
  .seg button:focus-visible {
    outline: 2px solid var(--accent);
  }
</style>
