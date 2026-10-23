<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { href, router } from '../lib/router.svelte';
  import { agentState, STATE_RANK, type AgentState } from '../lib/status';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary, Project } from '../lib/protocol';
  import { ChevronRight, Download, FolderGit2, LayoutDashboard, Moon, Plus, Search, Server, Sun, Bot } from '../lib/icons';
  import StatusDot from './StatusDot.svelte';
  import HostDot from './HostDot.svelte';

  /** Desktop installers (deb, AppImage, dmg, nsis) built by .github/workflows/desktop-release.yml. */
  const DESKTOP_DOWNLOAD_URL = 'https://github.com/BlackMac/coocoo/releases/latest';

  const COLLAPSE_KEY = 'coocoo.collapsed';
  let collapsed = $state<Record<string, boolean>>(JSON.parse(localStorage.getItem(COLLAPSE_KEY) ?? '{}'));

  function toggle(k: string) {
    collapsed[k] = !collapsed[k];
    localStorage.setItem(COLLAPSE_KEY, JSON.stringify(collapsed));
  }

  const route = $derived(router.route);
  const activeAgent = $derived(route.name === 'agent' ? key(route.host, route.id) : null);
  const activeProject = $derived(route.name === 'project' ? key(route.host, route.id) : null);

  interface Group {
    hostId: string;
    name: string;
    online: boolean;
    projects: { p: Project; agents: AgentSummary[] }[];
    loose: AgentSummary[];
  }

  const groups = $derived.by<Group[]>(() => {
    const out: Group[] = [];
    for (const h of fleet.hostList) {
      if (fleet.hostFilter && h.id !== fleet.hostFilter) continue;
      const agents = fleet.agentList.filter(
        (a) => a.hostId === h.id && (a.status !== 'stopped' || key(a.hostId, a.id) === activeAgent),
      );
      const projects = fleet.projectList
        .filter((p) => p.hostId === h.id)
        .map((p) => ({ p, agents: agents.filter((a) => a.projectId === p.id) }));
      const loose = agents.filter((a) => !a.projectId || !fleet.projects[key(h.id, a.projectId)]);
      if (!projects.length && !loose.length && !h.online) continue;
      out.push({ hostId: h.id, name: h.name, online: h.online, projects, loose });
    }
    return out;
  });

  /** Most urgent state among a project's agents, for the collapsed summary dot. */
  function summaryState(agents: AgentSummary[]): AgentState | null {
    let best: AgentState | null = null;
    for (const a of agents) {
      const s = agentState(a);
      if (STATE_RANK[s] <= STATE_RANK.review && (best === null || STATE_RANK[s] < STATE_RANK[best])) best = s;
    }
    return best;
  }
</script>

{#snippet agentRow(a: AgentSummary, depth: number)}
  {@const k = key(a.hostId, a.id)}
  <a class="row agent" class:active={activeAgent === k} style:--depth={depth} href={href({ name: 'agent', host: a.hostId, id: a.id })}>
    <StatusDot state={agentState(a)} size={7} />
    <span class="ellipsis" class:unread={a.unread}>{a.title}</span>
    {#if a.pendingRequests}<span class="badge sm">{a.pendingRequests}</span>{/if}
  </a>
{/snippet}

{#snippet agentTree(agents: AgentSummary[], depth: number)}
  {@const ids = new Set(agents.map((a) => a.id))}
  {#each agents.filter((a) => !a.parentId || !ids.has(a.parentId)) as a (a.id)}
    {@render agentRow(a, depth)}
    {#each agents.filter((c) => c.parentId === a.id) as c (c.id)}
      {@render agentRow(c, depth + 1)}
    {/each}
  {/each}
{/snippet}

<aside class="sidebar">
  <div class="brand">
    <span class="logo" aria-hidden="true">
      <svg viewBox="0 0 32 32" width="22" height="22"><rect width="32" height="32" rx="8" fill="var(--accent)" /><circle cx="12.5" cy="16" r="5" fill="none" stroke="#fff" stroke-width="2.6" /><circle cx="20.5" cy="16" r="5" fill="none" stroke="#fff" stroke-width="2.6" opacity=".85" /></svg>
    </span>
    <span class="name">Coocoo</span>
    {#if !fleet.native}
      <a class="btn ghost icon sm dl" href={DESKTOP_DOWNLOAD_URL} target="_blank" rel="noopener noreferrer" title="Download the desktop app" aria-label="Download the desktop app">
        <Download size={14} />
      </a>
    {/if}
    <span class="conn" data-state={fleet.conn} title={`Daemon: ${fleet.conn}`}></span>
  </div>

  <div class="hosts" role="radiogroup" aria-label="Host filter">
    <button class="hchip" class:on={!fleet.hostFilter} role="radio" aria-checked={!fleet.hostFilter} onclick={() => (fleet.hostFilter = null)}>All hosts</button>
    {#each fleet.hostList as h (h.id)}
      <button
        class="hchip"
        class:on={fleet.hostFilter === h.id}
        role="radio"
        aria-checked={fleet.hostFilter === h.id}
        title={h.online ? h.name : `${h.name} — offline`}
        onclick={() => (fleet.hostFilter = fleet.hostFilter === h.id ? null : h.id)}
      >
        <HostDot online={h.online} />{h.name}
      </button>
    {/each}
  </div>

  <nav class="nav">
    <a class="row" class:active={route.name === 'overview'} href="#/">
      <LayoutDashboard size={15} />
      <span>Overview</span>
      {#if fleet.needsCount}<span class="badge">{fleet.needsCount}</span>{/if}
    </a>
    <button class="row" onclick={() => (ui.paletteOpen = true)}>
      <Search size={15} />
      <span>Jump to…</span>
      <span class="kbd">⌘K</span>
    </button>
  </nav>

  <div class="tree">
    <div class="tree-head">
      <span>Projects</span>
      <button class="btn ghost icon sm" title="Add project" aria-label="Add project" onclick={() => (ui.dialog = { kind: 'addProject', hostId: fleet.hostFilter ?? undefined })}>
        <Plus size={14} />
      </button>
    </div>
    {#each groups as g (g.hostId)}
      {#if fleet.hostList.length > 1 && !fleet.hostFilter}
        <div class="host-label"><HostDot online={g.online} size={5} />{g.name}</div>
      {/if}
      {#each g.projects as { p, agents } (p.id)}
        {@const k = key(p.hostId, p.id)}
        {@const open = !collapsed[k]}
        {@const sum = summaryState(agents)}
        {@const icon = fleet.projectIcon(p)}
        <div class="project">
          <div class="row prow" class:active={activeProject === k}>
            <button class="caret" class:open aria-label={open ? 'Collapse' : 'Expand'} aria-expanded={open} onclick={() => toggle(k)}>
              <ChevronRight size={13} />
            </button>
            <a class="plink" href={href({ name: 'project', host: p.hostId, id: p.id })}>
              {#if icon}<img class="picon" src={icon} alt="" />{:else}<FolderGit2 size={14} />{/if}
              <span class="ellipsis">{p.name}</span>
            </a>
            {#if !open && sum}<StatusDot state={sum} size={6} />{/if}
            {#if agents.length}<span class="cnt num">{agents.length}</span>{/if}
          </div>
          {#if open}
            {@render agentTree(agents, 1)}
          {/if}
        </div>
      {/each}
      {#if g.loose.length}
        <div class="project">
          <div class="row prow loose">
            <span class="caret-pad"></span>
            <Bot size={14} />
            <span class="ellipsis">Other agents</span>
          </div>
          {@render agentTree(g.loose, 1)}
        </div>
      {/if}
      {#if !g.projects.length && !g.loose.length}
        <div class="none">{g.online ? 'No projects yet' : 'Offline'}</div>
      {/if}
    {/each}
  </div>

  <div class="foot">
    <a class="row" class:active={route.name === 'settings'} href="#/settings">
      <Server size={15} />
      <span>Hosts &amp; Settings</span>
    </a>
    <button class="btn ghost icon" title="Toggle theme" aria-label="Toggle theme" onclick={() => ui.toggleTheme()}>
      {#if ui.theme === 'dark'}<Sun size={15} />{:else}<Moon size={15} />{/if}
    </button>
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    border-right: 1px solid var(--border);
    background: var(--bg-1);
    user-select: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    height: var(--header-h);
    padding: 0 14px;
    flex: none;
  }
  .logo {
    display: flex;
  }
  .name {
    font-weight: 600;
    font-size: 14px;
    letter-spacing: -0.01em;
  }
  .dl {
    margin-left: -4px;
  }
  .conn {
    width: 6px;
    height: 6px;
    margin-left: auto;
    border-radius: 50%;
    background: var(--amber);
  }
  .conn[data-state='open'] {
    background: var(--green);
    opacity: 0.8;
  }
  .conn[data-state='closed'],
  .conn[data-state='unauthorized'] {
    background: var(--red);
  }

  .hosts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 2px 10px 10px;
    flex: none;
  }
  .hchip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .hchip:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .hchip.on {
    background: var(--bg-active);
    border-color: var(--border-strong);
    color: var(--text);
  }

  .nav,
  .foot {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 0 8px;
    flex: none;
  }
  .foot {
    flex-direction: row;
    align-items: center;
    gap: 4px;
    padding: 8px;
    border-top: 1px solid var(--border);
  }
  .foot .row {
    flex: 1;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs);
    text-align: left;
    cursor: pointer;
    min-width: 0;
    width: 100%;
  }
  .row:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .row.active {
    background: var(--bg-active);
    color: var(--text);
  }
  .row > span:not(.badge):not(.kbd):not(.cnt) {
    flex: 1;
    min-width: 0;
  }
  .row .kbd {
    margin-left: auto;
  }

  .tree {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 8px 12px;
  }
  .tree-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 30px;
    padding: 0 2px 0 8px;
    margin-top: 6px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .host-label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 8px 4px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .prow {
    gap: 4px;
    padding-left: 2px;
    cursor: default;
  }
  .prow.loose {
    gap: 6px;
    color: var(--text-3);
  }
  .caret,
  .caret-pad {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex: none;
    padding: 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
  }
  .caret :global(svg) {
    transition: transform var(--t-fast) var(--ease);
  }
  .caret.open :global(svg) {
    transform: rotate(90deg);
  }
  .caret:hover {
    color: var(--text);
  }
  .picon {
    width: 14px;
    height: 14px;
    flex: none;
    object-fit: contain;
    border-radius: 3px;
  }
  .plink {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    height: 100%;
    color: inherit;
  }
  .cnt {
    color: var(--text-3);
    font-size: var(--fs-xs);
    padding-right: 2px;
  }
  .agent {
    height: 26px;
    padding-left: calc(10px + var(--depth) * 14px);
    font-size: var(--fs-sm);
  }
  .agent .unread {
    color: var(--text);
    font-weight: 500;
  }
  .badge.sm {
    min-width: 16px;
    height: 16px;
    font-size: 10px;
  }
  .none {
    padding: 4px 10px 6px 28px;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
</style>
