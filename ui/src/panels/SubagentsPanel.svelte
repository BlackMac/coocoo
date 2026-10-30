<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { RpcError } from '../lib/client';
  import { ui } from '../lib/ui.svelte';
  import { router } from '../lib/router.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import type { AgentSummary, Subagent, TimelineItem } from '../lib/protocol';
  import { MessageSquareText, Users } from '../lib/icons';
  import SubagentRow from '../agent/SubagentRow.svelte';
  import SubagentDetail from './SubagentDetail.svelte';

  // Every subagent the agent started, newest call first, live: what each does right now, and a
  // click away its transcript (`ui.subagentOpen`, also set by a click on one in the chat).
  let { agent }: { agent: AgentSummary } = $props();

  const host = $derived(agent.hostId);
  const k = $derived(key(agent.hostId, agent.id));
  const items = $derived(fleet.subagents[k]);

  let error = $state<string | null>(null);
  // Per agent, and again after a reconnect (events may have been missed).
  $effect(() => {
    void k;
    if (fleet.conn !== 'open') return;
    untrack(() => {
      error = null;
      fleet.loadSubagents(agent.hostId, agent.id).catch((err) => {
        const old = err instanceof RpcError && err.code === 'unknown_method';
        const name = fleet.hosts[agent.hostId]?.name ?? 'This host';
        error = old ? `${name} runs an older Coocoo version. Update it to see the agent's subagents here.` : err instanceof Error ? err.message : String(err);
      });
    });
  });

  interface Entry {
    ref: string;
    item: TimelineItem;
    sub: Subagent;
  }
  /** Newest call first; within a call, in start order. */
  const groups = $derived(
    [...(items ?? [])]
      .reverse()
      .map((item) => ({ item, entries: (item.tool?.subagents ?? []).map((sub): Entry => ({ ref: `${item.id}/${sub.id}`, item, sub })) })),
  );
  const flat = $derived(groups.flatMap((g) => g.entries));
  const running = $derived(flat.filter((e) => e.sub.status === 'running').length);

  const openRef = $derived(ui.subagentOpen[k] ?? null);
  const index = $derived(openRef ? flat.findIndex((e) => e.ref === openRef) : -1);
  const current = $derived(index >= 0 ? flat[index] : undefined);

  const pick = (ref: string | null) => (ui.subagentOpen[k] = ref);
  const step = (d: number) => flat.length && pick(flat[(index + d + flat.length) % flat.length].ref);
  const showInChat = (item: TimelineItem) => router.go({ name: 'agent', host, id: agent.id, at: item.ts });

  const when = (ts: number) => {
    const t = ago(ts, now.value);
    return t === 'now' ? 'just now' : `${t} ago`;
  };
</script>

<div class="subagents">
  {#if current}
    {#key current.ref}
      <SubagentDetail
        {host}
        agentId={agent.id}
        item={current.item}
        sub={current.sub}
        position={[index, flat.length]}
        onback={() => pick(null)}
        onstep={step}
        onchat={() => showInChat(current.item)}
      />
    {/key}
  {:else if !items && !error}
    <div class="empty"><span class="spinner"></span></div>
  {:else if !flat.length}
    <div class="empty">
      <strong>{error ? 'Subagents not available' : 'No subagents yet'}</strong>
      <span>{error ?? 'Subagents the agent starts (its task tool) show up here while they work, each with its transcript.'}</span>
    </div>
  {:else}
    <div class="summary faint num">
      <Users size={13} />
      <span>{flat.length} subagent{flat.length === 1 ? '' : 's'}</span>
      {#if running}<span class="run">· {running} running</span>{/if}
    </div>
    <div class="list">
      {#each groups as g (g.item.id)}
        <section class="group">
          <header class="gh">
            <span class="intent ellipsis" title={g.item.tool?.intent}>{g.item.tool?.intent || g.item.tool?.name}</span>
            <span class="faint when">{when(g.item.ts)}</span>
            <button class="btn ghost icon sm" onclick={() => showInChat(g.item)} aria-label="Show in chat" title="Show in chat"><MessageSquareText size={13} /></button>
          </header>
          {#each g.entries as e (e.ref)}
            <SubagentRow s={e.sub} onclick={() => pick(e.ref)} />
          {/each}
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .subagents {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    container-type: inline-size;
  }
  .empty {
    flex: 1;
    padding: 24px;
    text-align: center;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    flex: none;
  }
  .run {
    color: var(--blue);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 8px 12px;
  }
  .group + .group {
    margin-top: 10px;
  }
  .gh {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 2px 0 2px 6px;
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-weight: 500;
  }
  .intent {
    flex: 1;
    min-width: 0;
  }
  .when {
    flex: none;
    font-weight: 400;
  }
</style>
