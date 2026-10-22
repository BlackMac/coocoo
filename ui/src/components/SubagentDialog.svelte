<script lang="ts">
  import { tick } from 'svelte';
  import { fleet } from '../lib/fleet.svelte';
  import { cost, duration } from '../lib/format';
  import type { Subagent, TimelineItem } from '../lib/protocol';
  import { ui } from '../lib/ui.svelte';
  import Dialog from './Dialog.svelte';
  import ItemList from '../agent/ItemList.svelte';

  // Read-only transcript of one subagent, read from its omp session file on the agent's host.
  // Refreshes while the subagent runs.
  let { hostId, agentId, subagentId, sessionFile }: { hostId: string; agentId: string; subagentId: string; sessionFile: string } = $props();

  const POLL_MS = 2000;

  /** The subagent's current state, from the parent agent's timeline (live updates). */
  const sub = $derived.by<Subagent | undefined>(() => {
    for (const item of fleet.timeline(hostId, agentId).items) {
      const found = item.tool?.subagents?.find((s) => s.id === subagentId && s.sessionFile === sessionFile);
      if (found) return found;
    }
    return undefined;
  });
  const live = $derived(sub?.status === 'running');

  let items = $state<TimelineItem[]>([]);
  let hasMore = $state(false);
  let error = $state('');
  let loaded = $state(false);
  let scroller = $state<HTMLDivElement>();

  async function load(isLive: boolean) {
    try {
      const page = await fleet.call('agents.subagentTimeline', { id: agentId, sessionFile, live: isLive }, hostId);
      const el = scroller;
      const atBottom = !el || el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      items = page.items;
      hasMore = page.hasMore;
      error = '';
      if (atBottom || !loaded) {
        await tick();
        if (scroller) scroller.scrollTop = scroller.scrollHeight;
      }
      loaded = true;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  // Load now, poll while it runs; one last read once it finishes (unanswered calls settle).
  $effect(() => {
    const isLive = live;
    void load(isLive);
    if (!isLive) return;
    const timer = setInterval(() => document.visibilityState === 'visible' && load(true), POLL_MS);
    return () => clearInterval(timer);
  });
</script>

<Dialog title="Subagent {subagentId}" width={860} onclose={() => (ui.dialog = null)}>
  {#if sub}
    <div class="info num">
      <span class="status" data-status={sub.status}>{sub.status}</span>
      <span>{sub.agent}</span>
      {#if sub.model}<span class="mono">{sub.model}</span>{/if}
      {#if sub.endedAt}<span>{duration(Math.max(0, sub.endedAt - sub.startedAt))}</span>{/if}
      {#if sub.toolCount}<span>{sub.toolCount} tool{sub.toolCount === 1 ? '' : 's'}</span>{/if}
      {#if sub.costUsd}<span>{cost(sub.costUsd)}</span>{/if}
    </div>
    {#if sub.description && sub.description !== sub.id}<div class="desc">{sub.description}</div>{/if}
  {/if}
  <div class="list" bind:this={scroller}>
    {#if hasMore}<div class="faint more">Earlier messages are not shown.</div>{/if}
    {#if error && !loaded}
      <div class="faint">Could not load the transcript: {error}</div>
    {:else if !loaded}
      <div class="loading"><span class="spinner"></span></div>
    {:else if !items.length}
      <div class="faint">No messages yet.</div>
    {/if}
    <ItemList {items} />
    {#if live}<div class="faint working"><span class="spinner"></span>{sub?.activity ?? 'Working…'}</div>{/if}
  </div>
</Dialog>

<style>
  .info {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .status {
    font-weight: 600;
    color: var(--green);
  }
  .status[data-status='running'] {
    color: var(--blue);
  }
  .status[data-status='failed'] {
    color: var(--red);
  }
  .status[data-status='aborted'] {
    color: var(--text-3);
  }
  .desc {
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: calc(80vh - 120px);
    overflow-y: auto;
    padding-right: 4px;
  }
  .more {
    text-align: center;
    font-size: var(--fs-xs);
  }
  .loading {
    display: grid;
    place-items: center;
    padding: 32px;
  }
  .working {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
  }
  .working .spinner {
    width: 11px;
    height: 11px;
    flex: none;
  }
</style>
