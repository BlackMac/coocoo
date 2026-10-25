<script lang="ts">
  import { resolveEndpoint } from './lib/bootstrap';
  import { fleet } from './lib/fleet.svelte';
  import { router } from './lib/router.svelte';
  import { ui } from './lib/ui.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import Header from './components/Header.svelte';
  import Toasts from './components/Toasts.svelte';
  import CommandPalette from './components/CommandPalette.svelte';
  import TokenScreen from './components/TokenScreen.svelte';
  import NewAgentDialog from './components/NewAgentDialog.svelte';
  import AddHostDialog from './components/AddHostDialog.svelte';
  import AddProjectDialog from './components/AddProjectDialog.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import PushDialog from './components/PushDialog.svelte';
  import SubagentDialog from './components/SubagentDialog.svelte';
  import ScheduleDialog from './components/ScheduleDialog.svelte';
  import SecretDialog from './components/SecretDialog.svelte';
  import Overview from './views/Overview.svelte';
  import ProjectView from './views/ProjectView.svelte';
  import AgentView from './views/AgentView.svelte';
  import SettingsView from './views/SettingsView.svelte';
  import { track } from './lib/track';

  // Heavy and rarely opened: loaded on first visit.
  const StatsView = () => import('./views/StatsView.svelte');
  const WorkflowsView = () => import('./views/WorkflowsView.svelte');

  const endpoint = resolveEndpoint();
  let token = $state(endpoint.token);

  $effect(() => {
    if (token && !fleet.client) fleet.start(endpoint.wsUrl, token, endpoint.native);
  });

  const route = $derived(router.route);

  // Close the drawer whenever the route changes (mobile); record the page for the usage log.
  $effect(() => {
    track('route', { meta: { name: route.name } });
    ui.drawerOpen = false;
  });

  /** Context for "new agent": the project in view, if any. */
  function newAgent() {
    const r = router.route;
    if (r.name === 'project') ui.dialog = { kind: 'newAgent', hostId: r.host, projectId: r.id };
    else if (r.name === 'agent') {
      const a = fleet.agent(r.host, r.id);
      ui.dialog = { kind: 'newAgent', hostId: r.host, projectId: a?.projectId };
    } else ui.dialog = { kind: 'newAgent', hostId: fleet.hostFilter ?? undefined };
  }

  let pendingG = 0;

  function onkeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      track('shortcut', { meta: { key: 'mod+k' } });
      ui.paletteOpen = !ui.paletteOpen;
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
    if (e.key === 'Escape' && ui.drawerOpen) {
      e.preventDefault();
      ui.drawerOpen = false;
      return;
    }
    const t = e.target as HTMLElement;
    if (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) || t.closest('[data-capture-keys]')) return;
    if (ui.dialog || ui.paletteOpen) return;
    const now = Date.now();
    if (e.key === 'g') {
      pendingG = now;
      return;
    }
    if (now - pendingG < 900) {
      pendingG = 0;
      if (e.key === 'o') {
        track('shortcut', { meta: { key: 'g o' } });
        router.go({ name: 'overview' });
      } else if (e.key === 's') {
        track('shortcut', { meta: { key: 'g s' } });
        router.go({ name: 'settings' });
      }
      return;
    }
    if (e.key === 'n') {
      e.preventDefault();
      track('shortcut', { meta: { key: 'n' } });
      newAgent();
    } else if (e.key === '/') {
      e.preventDefault();
      track('shortcut', { meta: { key: '/' } });
      ui.paletteOpen = true;
    }
  }
</script>

<svelte:window {onkeydown} />

{#if !token || fleet.conn === 'unauthorized'}
  <TokenScreen
    rejected={fleet.conn === 'unauthorized'}
    onsubmit={(t) => {
      if (fleet.client) location.reload();
      token = t;
    }}
  />
{:else}
  <div class="shell" class:drawer={ui.drawerOpen}>
    <Sidebar />
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="scrim" onclick={() => (ui.drawerOpen = false)}></div>
    <main>
      <Header onnew={newAgent} />
      {#if fleet.conn === 'reconnecting'}
        <div class="banner"><span class="spinner"></span> Connection lost — reconnecting…</div>
      {/if}
      <div class="view">
        {#if !fleet.ready}
          <div class="loading"><span class="spinner"></span></div>
        {:else if route.name === 'overview'}
          <Overview />
        {:else if route.name === 'stats'}
          {#await StatsView() then m}<m.default />{/await}
        {:else if route.name === 'workflows'}
          {#await WorkflowsView() then m}<m.default />{/await}
        {:else if route.name === 'project'}
          {#key `${route.host}:${route.id}`}<ProjectView host={route.host} id={route.id} />{/key}
        {:else if route.name === 'agent'}
          {#key `${route.host}:${route.id}`}<AgentView host={route.host} id={route.id} />{/key}
        {:else if route.name === 'settings'}
          <SettingsView />
        {/if}
      </div>
    </main>
  </div>
  {#if ui.paletteOpen}<CommandPalette onnew={newAgent} />{/if}
  {#if ui.dialog?.kind === 'newAgent'}
    <NewAgentDialog hostId={ui.dialog.hostId} projectId={ui.dialog.projectId} />
  {:else if ui.dialog?.kind === 'addHost'}
    <AddHostDialog />
  {:else if ui.dialog?.kind === 'addProject'}
    <AddProjectDialog hostId={ui.dialog.hostId} />
  {:else if ui.dialog?.kind === 'subagent'}
    <SubagentDialog hostId={ui.dialog.hostId} agentId={ui.dialog.agentId} subagentId={ui.dialog.subagentId} sessionFile={ui.dialog.sessionFile} />
  {:else if ui.dialog?.kind === 'confirm'}
    <ConfirmDialog dialog={ui.dialog} />
  {:else if ui.dialog?.kind === 'push'}
    <PushDialog dialog={ui.dialog} />
  {:else if ui.dialog?.kind === 'schedule'}
    <ScheduleDialog hostId={ui.dialog.hostId} projectId={ui.dialog.projectId} scheduleId={ui.dialog.scheduleId} />
  {:else if ui.dialog?.kind === 'secret'}
    <SecretDialog hostId={ui.dialog.hostId} projectId={ui.dialog.projectId} name={ui.dialog.name} />
  {/if}
{/if}
<Toasts />

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    height: 100%;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
  }
  .view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .loading {
    flex: 1;
    display: grid;
    place-items: center;
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 28px;
    border-bottom: 1px solid var(--border);
    background: var(--amber-soft);
    color: var(--amber);
    font-size: var(--fs-sm);
  }
  .scrim {
    display: none;
  }
  @media (max-width: 860px) {
    .shell {
      grid-template-columns: minmax(0, 1fr);
    }
    .shell :global(.sidebar) {
      position: fixed;
      inset: 0 auto 0 0;
      z-index: 30;
      width: min(300px, 86vw);
      transform: translateX(-100%);
      transition: transform var(--t) var(--ease);
    }
    .shell.drawer :global(.sidebar) {
      transform: none;
      box-shadow: var(--shadow-pop);
    }
    .shell.drawer .scrim {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 29;
      background: var(--overlay);
    }
  }
</style>
