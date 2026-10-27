<script lang="ts">
  import type { Component } from 'svelte';
  import { fleet } from '../lib/fleet.svelte';
  import { router } from '../lib/router.svelte';
  import { agentState, type AgentState } from '../lib/status';
  import { ui } from '../lib/ui.svelte';
  import { track } from '../lib/track';
  import { actionDisabled, actionIcon, actionRun, actionsFor, type ActionContext, showRun, startAction } from '../lib/actions';
  import { worktreeOf } from '../lib/git.svelte';
  import { Bot, CornerDownLeft, FolderGit2, Gamepad2, LayoutDashboard, Moon, Plus, Search, Server, Settings } from '../lib/icons';
  import StatusDot from './StatusDot.svelte';

  let { onnew }: { onnew: () => void } = $props();

  interface Entry {
    id: string;
    group: 'Agents' | 'Projects' | 'Hosts' | 'Actions';
    label: string;
    hint?: string;
    icon?: Component<{ size?: number }>;
    state?: AgentState;
    run: () => void;
  }

  let query = $state('');
  let selected = $state(0);
  let list: HTMLDivElement;

  const close = () => (ui.paletteOpen = false);

  const entries = $derived.by<Entry[]>(() => {
    const out: Entry[] = [
      { id: 'a:new', group: 'Actions', label: 'New agent', hint: 'N', icon: Plus, run: onnew },
      { id: 'a:ov', group: 'Actions', label: 'Go to overview', hint: 'G O', icon: LayoutDashboard, run: () => router.go({ name: 'overview' }) },
      { id: 'a:vibe', group: 'Actions', label: 'Vibe controller', hint: 'G V', icon: Gamepad2, run: () => router.go({ name: 'vibe' }) },
      { id: 'a:set', group: 'Actions', label: 'Hosts & settings', hint: 'G S', icon: Settings, run: () => router.go({ name: 'settings' }) },
      { id: 'a:proj', group: 'Actions', label: 'Add project…', icon: FolderGit2, run: () => (ui.dialog = { kind: 'addProject' }) },
      { id: 'a:host', group: 'Actions', label: 'Add host…', icon: Server, run: () => (ui.dialog = { kind: 'addHost' }) },
      { id: 'a:theme', group: 'Actions', label: `Switch to ${ui.theme === 'dark' ? 'light' : 'dark'} theme`, icon: Moon, run: () => ui.toggleTheme() },
    ];
    // Actions of what is open: the agent (in its directory) and its project, or the project.
    const route = router.route;
    const agent = route.name === 'agent' ? fleet.agent(route.host, route.id) : undefined;
    const project = agent ? fleet.project(agent.hostId, agent.projectId) : route.name === 'project' ? fleet.project(route.host, route.id) : undefined;
    const contexts: ActionContext[] = project ? (agent ? [{ project, agent }, { project }] : [{ project }]) : [];
    for (const ctx of contexts) {
      const where = ctx.agent ? [ctx.agent.title, worktreeOf(ctx.project, ctx.agent.cwd)?.branch].filter(Boolean).join(' · ') : `${ctx.project.name} · project`;
      for (const a of actionsFor(ctx)) {
        if (actionDisabled(a, ctx)) continue;
        const r = actionRun(ctx, a);
        const status = !r ? '' : !r.exitedAt ? ' · running' : r.exitCode === 0 ? ' · ✓' : r.exitCode == null ? ' · stopped' : ` · ✗ exit ${r.exitCode}`;
        out.push({
          id: `run:${ctx.agent ? 'a' : 'p'}:${a.id}`,
          group: 'Actions',
          label: `Run: ${a.name}`,
          hint: where + status,
          icon: actionIcon(a),
          run: () => (r && !r.exitedAt ? showRun(ctx.project.hostId, r, !!ctx.agent) : startAction(a, ctx)),
        });
      }
    }
    for (const a of Object.values(fleet.agents)) {
      const p = fleet.project(a.hostId, a.projectId);
      out.push({
        id: `ag:${a.hostId}:${a.id}`,
        group: 'Agents',
        label: a.title,
        hint: [p?.name, fleet.host(a.hostId)?.name].filter(Boolean).join(' · '),
        state: agentState(a),
        icon: Bot,
        run: () => router.go({ name: 'agent', host: a.hostId, id: a.id }),
      });
    }
    for (const p of Object.values(fleet.projects)) {
      out.push({
        id: `p:${p.hostId}:${p.id}`,
        group: 'Projects',
        label: p.name,
        hint: `${fleet.host(p.hostId)?.name ?? p.hostId} · ${p.path}`,
        icon: FolderGit2,
        run: () => router.go({ name: 'project', host: p.hostId, id: p.id }),
      });
    }
    for (const h of fleet.hostList) {
      out.push({
        id: `h:${h.id}`,
        group: 'Hosts',
        label: h.name,
        hint: `${h.online ? 'online' : 'offline'} · show only this host`,
        icon: Server,
        run: () => {
          fleet.hostFilter = h.id;
          router.go({ name: 'overview' });
        },
      });
    }
    return out;
  });

  /** Subsequence match; higher is better, -1 = no match. Rewards word starts and runs. */
  function score(text: string, q: string): number {
    if (!q) return 0;
    const t = text.toLowerCase();
    const direct = t.indexOf(q);
    if (direct >= 0) return 1000 - direct * 2 - (t.length - q.length) * 0.1 + (direct === 0 || /\W/.test(t[direct - 1]) ? 200 : 0);
    let s = 0;
    let ti = 0;
    let run = 0;
    for (const ch of q) {
      const at = t.indexOf(ch, ti);
      if (at < 0) return -1;
      run = at === ti ? run + 1 : 0;
      s += 10 + run * 5 + (at === 0 || /\W/.test(t[at - 1]) ? 15 : 0) - Math.min(at - ti, 10);
      ti = at + 1;
    }
    return s;
  }

  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const order = { Actions: 3, Agents: 0, Projects: 1, Hosts: 2 };
    if (!q) {
      const recent = entries
        .filter((e) => e.group === 'Agents')
        .slice()
        .sort((a, b) => {
          const aa = fleet.agents[a.id.slice(3)];
          const bb = fleet.agents[b.id.slice(3)];
          return (bb?.lastActivityAt ?? 0) - (aa?.lastActivityAt ?? 0);
        })
        .slice(0, 6);
      return [...entries.filter((e) => e.group === 'Actions'), ...recent];
    }
    return entries
      .map((e) => ({ e, s: Math.max(score(e.label, q), score(`${e.label} ${e.hint ?? ''}`, q) - 50) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s || order[a.e.group] - order[b.e.group])
      .slice(0, 40)
      .map((x) => x.e);
  });

  $effect(() => {
    void query;
    selected = 0;
  });

  $effect(() => {
    list?.querySelector(`[data-i="${selected}"]`)?.scrollIntoView({ block: 'nearest' });
  });

  /** Stable name of what an entry does for the usage log: built-in actions by id, the rest by kind (never labels). */
  function commandOf(e: Entry): string {
    if (e.id.startsWith('a:')) return e.id.slice(2);
    if (e.id.startsWith('run:')) return e.id.startsWith('run:a:') ? 'agentAction' : 'projectAction';
    return e.group === 'Agents' ? 'agent' : e.group === 'Projects' ? 'project' : 'host';
  }

  function run(e: Entry | undefined) {
    if (!e) return;
    track('palette.run', { meta: { command: commandOf(e) } });
    close();
    e.run();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) {
      e.preventDefault();
      selected = Math.min(results.length - 1, selected + 1);
    } else if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) {
      e.preventDefault();
      selected = Math.max(0, selected - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(results[selected]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="pal" role="dialog" aria-modal="true" aria-label="Command palette">
    <div class="q">
      <Search size={15} />
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={query}
        {onkeydown}
        placeholder="Jump to an agent, project or host, or run an action…"
        autofocus
        spellcheck="false"
        autocomplete="off"
        aria-controls="pal-list"
        aria-activedescendant="pal-{selected}"
      />
      <span class="kbd">Esc</span>
    </div>
    <div class="list" id="pal-list" role="listbox" bind:this={list}>
      {#each results as e, i (e.id)}
        {#if i === 0 || results[i - 1].group !== e.group}
          <div class="grp">{query.trim() || e.group !== 'Agents' ? e.group : 'Recent agents'}</div>
        {/if}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="item"
          class:sel={i === selected}
          id="pal-{i}"
          data-i={i}
          role="option"
          tabindex="-1"
          aria-selected={i === selected}
          onmousemove={() => (selected = i)}
          onclick={() => run(e)}
        >
          <span class="ic">
            {#if e.state}<StatusDot state={e.state} />{:else if e.icon}<e.icon size={15} />{/if}
          </span>
          <span class="lbl ellipsis">{e.label}</span>
          {#if e.hint}<span class="hint ellipsis">{e.hint}</span>{/if}
          {#if i === selected}<CornerDownLeft size={13} class="enter" />{/if}
        </div>
      {:else}
        <div class="none">No matches</div>
      {/each}
    </div>
  </div>
</div>

<style>
  /* As tall as the visible viewport (see Dialog.svelte): the list shrinks above the keyboard. */
  .overlay {
    position: fixed;
    inset: 0 0 auto;
    height: var(--app-height, 100%);
    z-index: 60;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 14vh 16px 16px;
    background: var(--overlay);
    animation: fade var(--t-fast) var(--ease);
  }
  .pal {
    display: flex;
    flex-direction: column;
    width: min(620px, 100%);
    max-height: 100%;
    border-radius: 12px;
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
    animation: rise var(--t) var(--ease);
  }
  .q {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 48px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 14px;
  }
  input::placeholder {
    color: var(--text-3);
  }
  .list {
    max-height: min(420px, 60vh);
    overflow-y: auto;
    padding: 6px;
  }
  @media (max-width: 640px) {
    .overlay {
      padding-top: max(16px, env(safe-area-inset-top));
    }
  }
  .grp {
    padding: 8px 10px 4px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r);
    cursor: pointer;
  }
  .item.sel {
    background: var(--bg-active);
  }
  .ic {
    display: grid;
    place-items: center;
    width: 16px;
    color: var(--text-2);
  }
  .lbl {
    flex: none;
    max-width: 60%;
  }
  .hint {
    flex: 1;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .item :global(.enter) {
    margin-left: auto;
    color: var(--text-3);
    flex: none;
  }
  .none {
    padding: 22px;
    text-align: center;
    color: var(--text-3);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.99);
    }
  }
  @media (max-width: 640px) {
    .overlay {
      padding: 8px;
    }
  }
</style>
