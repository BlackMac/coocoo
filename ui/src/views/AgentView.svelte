<script lang="ts" module>
  // Heavy panels load on first use and stay cached for the session.
  let changesMod: Promise<typeof import('../panels/ChangesPanel.svelte')> | undefined;
  let browserMod: Promise<typeof import('../panels/BrowserPanel.svelte')> | undefined;
  let terminalMod: Promise<typeof import('../panels/TerminalPanel.svelte')> | undefined;
</script>

<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { cost, modelName } from '../lib/format';
  import { router } from '../lib/router.svelte';
  import { agentState, STATE_LABEL } from '../lib/status';
  import { ui, type PanelTab } from '../lib/ui.svelte';
  import { Archive, Ellipsis, FileDiff, Globe, Info, ListChecks, Moon, PanelRight, Terminal, Trash2, MessageSquareText } from '../lib/icons';
  import StatusDot from '../components/StatusDot.svelte';
  import InlineEdit from '../components/InlineEdit.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import Transcript from '../agent/Transcript.svelte';
  import Composer from '../agent/Composer.svelte';
  import RequestCard from '../components/RequestCard.svelte';
  import PlanPanel from '../panels/PlanPanel.svelte';
  import InfoPanel from '../panels/InfoPanel.svelte';

  let { host, id }: { host: string; id: string } = $props();

  const k = $derived(key(host, id));
  const a = $derived(fleet.agents[k]);
  const st = $derived(a ? agentState(a) : 'idle');
  const timeline = $derived(fleet.timeline(host, id));
  const detail = $derived(fleet.details[k]);
  const requests = $derived(fleet.requests[k] ?? []);
  const project = $derived(a ? fleet.project(host, a.projectId) : undefined);

  $effect(() => {
    void fleet.loadDetail(host, id);
  });

  // Opening the agent (or it finishing while open and visible) marks it read.
  $effect(() => {
    if (a?.unread && document.visibilityState === 'visible') {
      fleet.call('agents.markRead', { id }, host).catch(() => {});
    }
  });

  const TABS: { id: PanelTab; label: string; icon: typeof FileDiff }[] = [
    { id: 'changes', label: 'Changes', icon: FileDiff },
    { id: 'browser', label: 'Browser', icon: Globe },
    { id: 'terminal', label: 'Terminal', icon: Terminal },
    { id: 'plan', label: 'Plan', icon: ListChecks },
    { id: 'info', label: 'Info', icon: Info },
  ];

  const loadChanges = () => (changesMod ??= import('../panels/ChangesPanel.svelte'));
  const loadBrowser = () => (browserMod ??= import('../panels/BrowserPanel.svelte'));
  const loadTerminal = () => (terminalMod ??= import('../panels/TerminalPanel.svelte'));

  async function act(method: 'agents.hibernate' | 'agents.archive', label: string) {
    try {
      const s = await fleet.call(method, { id }, host);
      fleet.agents[k] = s;
    } catch (err) {
      ui.error(`${label} failed`, err);
    }
  }

  function del(removeWorktree: boolean) {
    ui.dialog = {
      kind: 'confirm',
      title: 'Delete agent',
      message: removeWorktree
        ? `Delete “${a?.title}”, its session and the worktree ${a?.worktree?.path}? Uncommitted work in the worktree is lost.`
        : `Delete “${a?.title}” and its session? The working directory is kept.`,
      action: 'Delete',
      danger: true,
      run: async () => {
        await fleet.call('agents.delete', { id, removeWorktree }, host);
        router.go(a?.projectId ? { name: 'project', host, id: a.projectId } : { name: 'overview' });
      },
    };
  }

  const menu = $derived<MenuItem[]>([
    { label: 'Hibernate', icon: Moon, disabled: !a || ['hibernated', 'stopped'].includes(a.status), run: () => act('agents.hibernate', 'Hibernate') },
    { label: 'Archive', icon: Archive, disabled: a?.status === 'stopped', run: () => act('agents.archive', 'Archive') },
    { label: 'Delete', icon: Trash2, danger: true, run: () => del(false) },
    ...(a?.worktree ? [{ label: 'Delete with worktree', icon: Trash2, danger: true, run: () => del(true) }] : []),
  ]);

  // ─── Resizable panel ───────────────────────────────────────────────────────────────────────
  let dragging = $state(false);
  function startResize(e: PointerEvent) {
    const startX = e.clientX;
    const startW = ui.panel.width;
    dragging = true;
    const move = (ev: PointerEvent) => {
      ui.panel.width = Math.max(340, Math.min(window.innerWidth - 480, startW + (startX - ev.clientX)));
    };
    const up = () => {
      dragging = false;
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }
</script>

{#if !a}
  <div class="empty"><strong>Agent not found</strong><span>It may have been deleted, or its host is offline.</span></div>
{:else}
  <div class="agent" class:resizing={dragging} class:panel-open={ui.panel.open} style:--panel-w="{ui.panel.width}px" data-mobile={ui.mobilePane}>
    <section class="chat">
      <header class="ahead">
        <StatusDot state={st} />
        <h1><InlineEdit value={a.title} label="Agent title" onsave={(title) => fleet.call('agents.rename', { id, title }, host).then((s) => (fleet.agents[k] = s)).catch((e) => ui.error('Rename failed', e))} /></h1>
        <span class="state" data-state={st}>{STATE_LABEL[st]}</span>
        <span class="meta num">
          <span title={a.model}>{modelName(a.model)}</span>
          {#if a.thinking}<span class="faint">· {a.thinking}</span>{/if}
          {#if a.contextPercent != null}<span class="sep">·</span><span class:hot={a.contextPercent >= 80}>{Math.round(a.contextPercent)}% ctx</span>{/if}
          <span class="sep">·</span><span>{cost(a.costUsd)}</span>
        </span>
        <div class="hacts">
          <Menu label="Agent actions" items={menu}>{#snippet trigger()}<Ellipsis size={15} />{/snippet}</Menu>
          <button class="btn ghost icon sm toggle" class:active={ui.panel.open} title="Toggle side panel" aria-label="Toggle side panel" onclick={() => (ui.panel.open = !ui.panel.open)}>
            <PanelRight size={15} />
          </button>
        </div>
      </header>
      <nav class="mtabs" aria-label="Agent panes">
        <button class:on={ui.mobilePane === 'chat'} onclick={() => (ui.mobilePane = 'chat')}><MessageSquareText size={14} />Chat</button>
        {#each TABS as t (t.id)}
          <button class:on={ui.mobilePane === t.id} onclick={() => ui.showPanel(t.id)}><t.icon size={14} />{t.label}</button>
        {/each}
      </nav>
      {#if a.error && st === 'error'}
        <div class="errbar">{a.error}</div>
      {/if}
      <Transcript {timeline} agent={a} />
      <div class="bottom">
        {#if requests.length}
          <div class="reqs">{#each requests as r (r.id)}<RequestCard request={r} {host} />{/each}</div>
        {/if}
        <Composer agent={a} {detail} />
      </div>
    </section>

    {#if ui.panel.open}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="resizer" onpointerdown={startResize} title="Drag to resize"></div>
      <aside class="panel">
        <div class="tabs" role="tablist">
          {#each TABS as t (t.id)}
            <button role="tab" class:on={ui.panel.tab === t.id} aria-selected={ui.panel.tab === t.id} onclick={() => ui.showPanel(t.id)}>
              <t.icon size={14} />{t.label}
              {#if t.id === 'changes' && a.diff?.files}<span class="cnt num">{a.diff.files}</span>{/if}
              {#if t.id === 'plan' && a.todos?.total}<span class="cnt num">{a.todos.done}/{a.todos.total}</span>{/if}
            </button>
          {/each}
        </div>
        <div class="pbody">
          {#if ui.panel.tab === 'changes'}
            {#await loadChanges() then m}<m.default agent={a} {project} />{/await}
          {:else if ui.panel.tab === 'browser'}
            {#await loadBrowser() then m}<m.default agent={a} {project} />{/await}
          {:else if ui.panel.tab === 'terminal'}
            {#await loadTerminal() then m}<m.default {host} cwd={a.cwd} />{/await}
          {:else if ui.panel.tab === 'plan'}
            <PlanPanel {detail} agent={a} />
          {:else}
            <InfoPanel agent={a} {detail} {project} />
          {/if}
        </div>
      </aside>
    {/if}
  </div>
{/if}

<style>
  .agent {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
  }
  .agent.panel-open {
    grid-template-columns: minmax(380px, 1fr) 0 var(--panel-w);
  }
  .agent.resizing {
    user-select: none;
    cursor: col-resize;
  }
  .chat {
    display: flex;
    flex-direction: column;
    min-width: 0;
    container-type: inline-size;
    min-height: 0;
  }
  .ahead {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 44px;
    padding: 0 10px 0 16px;
    border-bottom: 1px solid var(--border);
    flex: none;
    min-width: 0;
  }
  h1 {
    display: flex;
    min-width: 0;
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .state {
    flex: none;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--bg-2);
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .state[data-state='working'] {
    background: var(--blue-soft);
    color: var(--blue);
  }
  .state[data-state='needs'] {
    background: var(--amber-soft);
    color: var(--amber);
  }
  .state[data-state='error'] {
    background: var(--red-soft);
    color: var(--red);
  }
  .state[data-state='review'] {
    background: var(--green-soft);
    color: var(--green);
  }
  .meta {
    display: flex;
    gap: 5px;
    margin-left: auto;
    color: var(--text-2);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
  }
  .meta .sep {
    color: var(--text-3);
  }
  @container (max-width: 600px) {
    .meta {
      display: none;
    }
    .hacts {
      margin-left: auto;
    }
  }
  .hot {
    color: var(--amber);
  }
  .hacts {
    display: flex;
    gap: 2px;
    flex: none;
  }
  .errbar {
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--red-soft);
    color: var(--red);
    font-size: var(--fs-sm);
  }
  .reqs {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 55vh;
    overflow-y: auto;
    border-radius: var(--r-lg);
  }
  .bottom {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 16px 14px;
    max-width: 920px;
    width: 100%;
    margin: 0 auto;
    flex: none;
  }

  .resizer {
    position: relative;
    z-index: 2;
    width: 0;
    cursor: col-resize;
  }
  .resizer::before {
    content: '';
    position: absolute;
    inset: 0 -3px;
  }
  .resizer:hover::after,
  .resizing .resizer::after {
    content: '';
    position: absolute;
    inset: 0 -1px;
    background: var(--accent);
  }
  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-left: 1px solid var(--border);
    background: var(--bg-1);
  }
  .tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    height: 44px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .tabs button,
  .mtabs button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .tabs button:hover,
  .mtabs button:hover {
    color: var(--text);
  }
  .tabs button.on,
  .mtabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .cnt {
    padding: 0 5px;
    border-radius: 99px;
    background: var(--bg-3);
    color: var(--text-2);
    font-size: 10.5px;
  }
  .pbody {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .mtabs {
    display: none;
  }

  @media (max-width: 860px) {
    /* Phone: header + pane switcher on top, one pane fills the rest. */
    .agent,
    .agent.panel-open {
      display: flex;
      flex-direction: column;
    }
    .chat {
      flex: 1;
    }
    .agent:not([data-mobile='chat']) .chat {
      flex: none;
    }
    .agent:not([data-mobile='chat']) .chat > :global(:not(.ahead):not(.mtabs)) {
      display: none;
    }
    .toggle,
    .resizer,
    .meta {
      display: none;
    }
    .mtabs {
      display: flex;
      height: 38px;
      padding: 0 6px;
      border-bottom: 1px solid var(--border);
      overflow-x: auto;
      flex: none;
    }
    .mtabs button {
      padding: 0 9px;
    }
    .panel {
      flex: 1;
      min-height: 0;
      border-left: 0;
      display: none;
      background: var(--bg);
    }
    .panel .tabs {
      display: none;
    }
    .agent:not([data-mobile='chat']) .panel {
      display: flex;
    }
    .bottom {
      padding: 0 10px 10px;
    }
    .ahead {
      padding: 0 6px 0 12px;
    }
  }
</style>
