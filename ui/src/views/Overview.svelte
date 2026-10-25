<script lang="ts">
  import { fleet, type AttentionItem } from '../lib/fleet.svelte';
  import type { UiRequest } from '../lib/protocol';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { href, router } from '../lib/router.svelte';
  import { agentState, GROUPS } from '../lib/status';
  import { ui } from '../lib/ui.svelte';
  import { track } from '../lib/track';
  import { parseError } from '../lib/providerError';
  import { CircleAlert, CircleCheck, Inbox, Plus, FolderGit2, ChevronDown } from '../lib/icons';
  import HostCard from '../components/HostCard.svelte';
  import AgentRow from '../components/AgentRow.svelte';
  import ProjectCard from '../components/ProjectCard.svelte';
  import RequestCard from '../components/RequestCard.svelte';
  import DiffCount from '../components/DiffCount.svelte';
  import HostDot from '../components/HostDot.svelte';
  import ViewTabs from '../insights/ViewTabs.svelte';

  $effect(() => fleet.watchStats());

  const hosts = $derived(fleet.hostList.filter((h) => !fleet.hostFilter || h.id === fleet.hostFilter));
  const groups = $derived(
    GROUPS.map((g) => ({ ...g, agents: fleet.agentList.filter((a) => g.states.includes(agentState(a))) })).filter((g) => g.agents.length),
  );
  const counts = $derived.by(() => {
    const c = { working: 0, needs: 0, review: 0 };
    for (const a of fleet.agentList) {
      const s = agentState(a);
      if (s === 'working') c.working++;
      else if (s === 'needs' || s === 'error') c.needs++;
      else if (s === 'review') c.review++;
    }
    return c;
  });

  let folded = $state<Record<string, boolean>>({ hibernated: true });
  let qi = $state(-1);
  let queueEl = $state<HTMLElement>();

  function focusItem(i: number) {
    qi = Math.max(0, Math.min(fleet.attention.length - 1, i));
    queueEl?.querySelector<HTMLElement>(`[data-q="${qi}"]`)?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey || ui.dialog || ui.paletteOpen) return;
    const t = e.target as HTMLElement;
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
    if (e.key === 'j' || e.key === 'k') {
      if (!fleet.attention.length) return;
      e.preventDefault();
      track('shortcut', { meta: { key: e.key } });
      focusItem(qi < 0 ? 0 : qi + (e.key === 'j' ? 1 : -1));
    } else if (e.key === 'Enter' && t.dataset.q !== undefined) {
      const item = fleet.attention[Number(t.dataset.q)];
      if (item) router.go({ name: 'agent', host: item.agent.hostId, id: item.agent.id });
    }
  }

  async function markRead(item: AttentionItem) {
    try {
      await fleet.call('agents.markRead', { id: item.agent.id }, item.agent.hostId);
      fleet.agents[item.agentKey] = { ...item.agent, unread: false };
    } catch (err) {
      ui.error('Could not mark as read', err);
    }
  }

  /** One-line summary for requests that are not expanded. */
  function summary(r: UiRequest): string {
    const p = r.payload;
    if (p.method === 'secret') return `Secret ${p.name} requested`;
    if (p.method !== 'ask') return p.title;
    const first = p.questions[0]?.question ?? 'Question';
    return p.questions.length > 1 ? `${p.questions.length} questions · ${first}` : first;
  }
</script>

<svelte:window {onkeydown} />

<div class="scroll">
  <div class="page">
    <div class="titlebar">
      <ViewTabs current="overview" />
      <div class="summary num">
        <span><i class="d working"></i>{counts.working} working</span>
        <span><i class="d needs"></i>{counts.needs} need you</span>
        <span><i class="d review"></i>{counts.review} ready for review</span>
      </div>
    </div>

    <section class="hosts" aria-label="Hosts">
      {#each hosts as h (h.id)}<HostCard host={h} />{/each}
    </section>

    <section aria-label="Needs you">
      <h2 class="section-title">
        <Inbox size={14} /> Needs you <span class="count">{fleet.needsCount}</span>
        {#if counts.review}<span class="count">· {counts.review} to review</span>{/if}
        {#if fleet.attention.length}<span class="keys faint"><span class="kbd">J</span><span class="kbd">K</span> to move · <span class="kbd">↵</span> to open</span>{/if}
      </h2>
      {#if fleet.attention.length}
        <div class="queue card" bind:this={queueEl}>
          {#each fleet.attention as item, i (item.request?.id ?? `${item.kind}:${item.agentKey}`)}
            {@const host = fleet.host(item.agent.hostId)}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <div class="qitem" data-kind={item.kind} data-q={i} tabindex="0" onfocus={() => (qi = i)} class:focused={qi === i}>
              <div class="qhead">
                <span class="qicon">
                  {#if item.kind === 'request'}<span class="pulse"></span>{:else if item.kind === 'error'}<CircleAlert size={14} />{:else}<CircleCheck size={14} />{/if}
                </span>
                <a class="qtitle ellipsis" href={href({ name: 'agent', host: item.agent.hostId, id: item.agent.id })} tabindex="-1">{item.agent.title}</a>
                <span class="chip"><HostDot online={host?.online ?? false} size={5} />{host?.name}</span>
                {#if item.kind === 'review' && item.agent.diff}
                  <DiffCount add={item.agent.diff.additions} del={item.agent.diff.deletions} files={item.agent.diff.files} />
                {/if}
                <span class="when faint num">{ago(item.ts, now.value)}</span>
                {#if item.kind === 'review'}
                  <button class="btn ghost sm" tabindex="-1" onclick={() => markRead(item)}>Mark read</button>
                {/if}
                {#if item.kind !== 'request'}
                  <a class="btn sm" href={href({ name: 'agent', host: item.agent.hostId, id: item.agent.id })} tabindex="-1">{item.kind === 'review' ? 'Review' : 'Open'}</a>
                {/if}
              </div>
              {#if item.kind === 'request' && item.request}
                <!-- Only the selected request shows its answer controls; the rest stay one line. -->
                {#if i === Math.max(qi, 0)}
                  <div class="qbody"><RequestCard request={item.request} host={item.agent.hostId} compact /></div>
                {:else}
                  <button class="qbody qsum" tabindex="-1" onclick={() => focusItem(i)}>
                    <span class="ellipsis">{summary(item.request)}</span><span class="ans">Answer</span>
                  </button>
                {/if}
              {:else if item.kind === 'error'}
                <div class="qbody err">{item.agent.error ? parseError(item.agent.error).title : 'The last turn failed.'}</div>
              {:else if item.agent.preview}
                <div class="qbody preview">{item.agent.preview}</div>
              {/if}
            </div>
          {/each}
        </div>
      {:else}
        <div class="card calm">
          <CircleCheck size={16} />
          <span>Nothing needs you right now.</span>
          {#if counts.working}<span class="faint">{counts.working} agent{counts.working === 1 ? ' is' : 's are'} working.</span>{/if}
        </div>
      {/if}
    </section>

    <section aria-label="Agents" class="agents-sec">
      <h2 class="section-title">Agents <span class="count">{fleet.agentList.length}</span></h2>
      {#if groups.length}
        <div class="groups card">
          {#each groups as g (g.id)}
            <div class="group">
              <button class="ghead" onclick={() => (folded[g.id] = !folded[g.id])} aria-expanded={!folded[g.id]}>
                <span class="caret" class:open={!folded[g.id]}><ChevronDown size={13} /></span>
                <span class="gl" data-g={g.id}>{g.label}</span>
                <span class="count num">{g.agents.length}</span>
              </button>
              {#if !folded[g.id]}
                <div class="rows">
                  {#each g.agents as a (`${a.hostId}:${a.id}`)}<AgentRow agent={a} />{/each}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {:else}
        <div class="card empty">
          <strong>No agents yet</strong>
          <span>Start one on any project — locally or on a remote host.</span>
          <button class="btn primary" onclick={() => (ui.dialog = { kind: 'newAgent', hostId: fleet.hostFilter ?? undefined })}><Plus size={14} /> New agent</button>
        </div>
      {/if}
    </section>

    <section aria-label="Projects">
      <h2 class="section-title">
        Projects <span class="count">{fleet.projectList.length}</span>
        <button class="btn ghost sm add" onclick={() => (ui.dialog = { kind: 'addProject', hostId: fleet.hostFilter ?? undefined })}><Plus size={13} /> Add project</button>
      </h2>
      {#if fleet.projectList.length}
        <div class="pgrid">
          {#each fleet.projectList as p (`${p.hostId}:${p.id}`)}<ProjectCard project={p} />{/each}
        </div>
      {:else}
        <div class="card empty">
          <FolderGit2 size={20} />
          <strong>No projects</strong>
          <span>Add a directory on any host to start agents in it.</span>
        </div>
      {/if}
    </section>
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
    gap: 26px;
  }
  .titlebar {
    display: flex;
    align-items: baseline;
    gap: 16px;
    flex-wrap: wrap;
  }
  .summary {
    display: flex;
    gap: 14px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .summary span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .d {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .d.working {
    background: var(--blue);
  }
  .d.needs {
    background: var(--amber);
  }
  .d.review {
    background: var(--green);
  }

  .hosts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px;
    margin-top: -8px;
  }

  .keys {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-weight: 400;
  }
  .queue {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .qitem {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 14px;
    border-left: 2px solid transparent;
    outline: none;
  }
  .qitem + .qitem {
    border-top: 1px solid var(--border);
  }
  .qitem.focused:focus-visible,
  .qitem:focus-within {
    background: var(--bg-hover);
  }
  .qitem.focused:focus-visible {
    border-left-color: var(--accent);
  }
  .qhead {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .qicon {
    display: grid;
    place-items: center;
    width: 16px;
    flex: none;
  }
  [data-kind='error'] .qicon {
    color: var(--red);
  }
  [data-kind='review'] .qicon {
    color: var(--green);
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--amber);
    box-shadow: 0 0 0 3px var(--amber-soft);
  }
  .qtitle {
    font-weight: 500;
    color: var(--text);
  }
  .qtitle:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .qhead .chip {
    height: 18px;
    flex: none;
  }
  .when {
    margin-left: auto;
    font-size: var(--fs-xs);
    flex: none;
  }
  .qbody {
    padding-left: 25px;
    min-width: 0;
  }
  .qsum {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    margin-left: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .qsum .ellipsis {
    flex: 1;
  }
  .ans {
    flex: none;
    color: var(--amber);
    font-weight: 500;
  }
  .qsum:hover .ans {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .qbody.err {
    color: var(--red);
    font-size: var(--fs-sm);
  }
  .qbody.preview {
    color: var(--text-2);
    font-size: var(--fs-sm);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .qhead .btn {
    flex: none;
  }
  .calm {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    color: var(--text-2);
  }
  .calm :global(svg) {
    color: var(--green);
  }

  .agents-sec {
    container-type: inline-size;
  }
  .groups {
    padding: 4px;
  }
  .group + .group {
    margin-top: 2px;
    border-top: 1px solid var(--border);
    padding-top: 2px;
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: pointer;
  }
  .ghead:hover {
    color: var(--text);
  }
  .caret {
    display: flex;
    transform: rotate(-90deg);
    transition: transform var(--t-fast) var(--ease);
    color: var(--text-3);
  }
  .caret.open {
    transform: none;
  }
  .gl[data-g='needs'] {
    color: var(--amber);
  }
  .gl[data-g='working'] {
    color: var(--blue);
  }
  .gl[data-g='review'] {
    color: var(--green);
  }
  .ghead .count {
    color: var(--text-3);
    font-weight: 500;
  }
  .rows {
    padding-bottom: 4px;
  }

  .add {
    margin-left: auto;
    font-weight: 500;
  }
  .pgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 10px;
  }

  @media (max-width: 640px) {
    .hosts {
      display: flex;
      overflow-x: auto;
      scroll-snap-type: x mandatory;
      margin: -8px -12px 0;
      padding: 0 12px 2px;
      scrollbar-width: none;
    }
    .hosts > :global(*) {
      flex: 0 0 82%;
      scroll-snap-align: center;
    }
    .qhead {
      flex-wrap: wrap;
      row-gap: 6px;
    }
    .qtitle {
      flex: 1 1 calc(100% - 30px);
    }
    .qhead .chip {
      margin-left: 25px;
    }
    .qhead > :global(.dc) {
      display: none;
    }
    .page {
      padding: 16px 12px 40px;
      gap: 22px;
    }
    .keys {
      display: none;
    }
    .qbody {
      padding-left: 0;
    }
  }
</style>
