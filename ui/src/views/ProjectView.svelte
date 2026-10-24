<script lang="ts" module>
  let terminalMod: Promise<typeof import('../panels/TerminalPanel.svelte')> | undefined;
</script>

<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { copyText } from '../lib/clipboard';
  import { ago, shortPath } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { href, router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import { track } from '../lib/track';
  import type { CommitInfo } from '../lib/protocol';
  import { ArrowDownToLine, Copy, ExternalLink, GitBranch, GitCommitHorizontal, Plus, RefreshCw, Terminal, Trash2, X, Globe, Ellipsis } from '../lib/icons';
  import { OPERATION_LABEL, fetchProject, fetchedLabel, fetching, isStale } from '../lib/git.svelte';
  import AgentRow from '../components/AgentRow.svelte';
  import BoardPanel from '../panels/BoardPanel.svelte';
  import SchedulesPanel from '../panels/SchedulesPanel.svelte';
  import HostDot from '../components/HostDot.svelte';
  import Menu from '../components/Menu.svelte';
  import InlineEdit from '../components/InlineEdit.svelte';
  import ActionsButton from '../components/ActionsButton.svelte';

  let { host, id }: { host: string; id: string } = $props();

  const p = $derived(fleet.project(host, id));
  const h = $derived(fleet.host(host));
  const agents = $derived(fleet.agentsOf(host, id));
  const active = $derived(agents.filter((a) => a.status !== 'stopped'));
  const busyFetch = $derived(!!fetching[`${host}:${id}`]);
  const archived = $derived(agents.filter((a) => a.status === 'stopped'));
  const schedules = $derived(fleet.schedules[key(host, id)]);

  let commits = $state<CommitInfo[]>([]);
  let refreshing = $state(false);
  let newUrl = $state('');
  let showArchived = $state(false);
  let showTerminal = $state(false);
  const loadTerminal = () => (terminalMod ??= import('../panels/TerminalPanel.svelte'));

  function toggleTerminal() {
    showTerminal = !showTerminal;
    if (showTerminal) track('panel', { projectId: id, meta: { panel: 'terminal' } });
  }

  // An action started here opens the terminal section, which then shows its run.
  $effect(() => {
    const f = ui.terminalFocus;
    if (f && f.host === host && p && fleet.terminals[key(host, f.id)]?.action?.base === p.path) showTerminal = true;
  });

  $effect(() => {
    const path = p?.path;
    if (!path || !p?.git) return;
    fleet
      .call('git.log', { cwd: path, limit: 8 }, host)
      .then((c) => (commits = c))
      .catch(() => (commits = []));
  });

  async function refresh() {
    refreshing = true;
    try {
      const np = await fleet.call('projects.refresh', { id }, host);
      fleet.projects[`${host}:${id}`] = np;
    } catch (err) {
      ui.error('Refresh failed', err);
    } finally {
      refreshing = false;
    }
  }

  async function saveUrls(devUrls: string[]) {
    try {
      const np = await fleet.call('projects.update', { id, devUrls }, host);
      fleet.projects[`${host}:${id}`] = np;
    } catch (err) {
      ui.error('Could not save dev URLs', err);
    }
  }

  function addUrl(e: SubmitEvent) {
    e.preventDefault();
    let u = newUrl.trim();
    if (!u || !p) return;
    if (!/^[a-z]+:\/\//i.test(u)) u = `http://${u}`;
    newUrl = '';
    void saveUrls([...(p.devUrls ?? []), u]);
  }

  function rename(name: string) {
    fleet
      .call('projects.update', { id, name }, host)
      .then((np) => (fleet.projects[`${host}:${id}`] = np))
      .catch((err) => ui.error('Rename failed', err));
  }

  function remove() {
    ui.dialog = {
      kind: 'confirm',
      title: 'Remove project',
      message: `Remove “${p?.name}” from Coocoo? Files on disk are not touched.`,
      action: 'Remove',
      danger: true,
      run: async () => {
        await fleet.call('projects.remove', { id }, host);
        delete fleet.projects[`${host}:${id}`];
        router.go({ name: 'overview' });
      },
    };
  }

  function removeWorktree(path: string, branch?: string | null) {
    ui.dialog = {
      kind: 'confirm',
      title: 'Remove worktree',
      message: `Delete the worktree at ${path}${branch ? ` and its branch ${branch}` : ''}? Uncommitted changes in it are lost.`,
      action: 'Remove worktree',
      danger: true,
      run: async () => {
        await fleet.call('git.worktreeRemove', { projectId: id, path, deleteBranch: !!branch }, host);
        await refresh();
      },
    };
  }

  function copy(text: string) {
    copyText(text).then(() => ui.toast('success', 'Copied'), (err) => ui.error('Copy failed', err));
  }
</script>

<div class="scroll">
  {#if !p}
    <div class="empty"><strong>Project not found</strong><span>It may have been removed, or its host is offline.</span></div>
  {:else}
    <div class="page">
      <header class="phead">
        <div class="ttl">
          <h1><InlineEdit value={p.name} label="Project name" onsave={rename} /></h1>
          <span class="chip"><HostDot online={h?.online ?? false} size={5} />{h?.name ?? host}</span>
          <div class="acts">
            <button class="btn ghost icon" title="Refresh git status" aria-label="Refresh" onclick={refresh} disabled={refreshing}>
              <span class:spin={refreshing}><RefreshCw size={14} /></span>
            </button>
            {#if p.git}
              <button class="btn ghost icon" title="Fetch from the remotes ({fetchedLabel(p.git.fetchedAt, now.value)})" aria-label="Fetch" onclick={() => fetchProject(p!)} disabled={busyFetch}>
                <span class:pulse={busyFetch}><ArrowDownToLine size={14} /></span>
              </button>
            {/if}
            <button class="btn ghost icon" class:active={showTerminal} title="Terminal in the project directory" aria-label="Terminal" aria-pressed={showTerminal} onclick={toggleTerminal}>
              <Terminal size={14} />
            </button>
            <ActionsButton project={p} />
            <Menu
              label="Project actions"
              items={[
                { label: 'Copy path', icon: Copy, run: () => copy(p!.path) },
                { label: 'Remove project', icon: Trash2, danger: true, run: remove },
              ]}
            >
              {#snippet trigger()}<Ellipsis size={15} />{/snippet}
            </Menu>
            <button class="btn primary" title="New agent" aria-label="New agent" onclick={() => (ui.dialog = { kind: 'newAgent', hostId: host, projectId: id })}><Plus size={14} /><span class="lbl">New agent</span></button>
          </div>
        </div>
        <button class="path mono" title="Copy path" onclick={() => copy(p!.path)}>{p.path}<Copy size={12} /></button>
        {#if p.git}
          <div class="stats num">
            <span class="stat mono"><GitBranch size={13} />{p.git.branch ?? p.git.head ?? 'detached'}</span>
            {#if p.git.upstream}<span class="stat faint">⇄ {p.git.upstream}</span>{/if}
            <span class="stat faint" class:warn={isStale(p.git.fetchedAt, now.value)} title={p.git.fetchedAt ? new Date(p.git.fetchedAt).toLocaleString() : undefined}>{fetchedLabel(p.git.fetchedAt, now.value)}</span>
            <span class="stat" class:dim={!p.git.ahead}>↑ {p.git.ahead} ahead</span>
            <span class="stat" class:warn={p.git.behind > 0} class:dim={!p.git.behind}>↓ {p.git.behind} behind</span>
            <span class="stat" class:accent={p.git.changed > 0} class:dim={!p.git.changed}>{p.git.changed} changed</span>
            {#if p.git.lastCommit}
              <span class="stat ellipsis last"><GitCommitHorizontal size={13} /><span class="mono faint">{p.git.lastCommit.sha.slice(0, 7)}</span> {p.git.lastCommit.summary}</span>
            {/if}
          </div>
        {/if}
        <div class="urls">
          <Globe size={13} class="faint" />
          {#each p.devUrls ?? [] as u, i (u)}
            <span class="url">
              <a href={u} target="_blank" rel="noopener noreferrer" class="mono">{u.replace(/^https?:\/\//, '')}<ExternalLink size={11} /></a>
              <button aria-label="Remove {u}" onclick={() => saveUrls((p!.devUrls ?? []).filter((_, j) => j !== i))}><X size={11} /></button>
            </span>
          {/each}
          <form onsubmit={addUrl}>
            <input class="input mono addurl" bind:value={newUrl} placeholder="+ dev URL (e.g. localhost:5173)" spellcheck="false" />
          </form>
        </div>
      </header>

      {#if showTerminal}
        <section class="card term-sec">
          {#await loadTerminal() then m}<m.default {host} cwd={p.path} />{/await}
        </section>
      {/if}

      <section class="agents-sec">
        <h2 class="section-title">Agents <span class="count">{active.length}</span></h2>
        {#if active.length}
          <div class="card list">{#each active as a (a.id)}<AgentRow agent={a} showProject={false} />{/each}</div>
        {:else}
          <div class="card empty">
            <strong>No agents in this project</strong>
            <button class="btn primary" onclick={() => (ui.dialog = { kind: 'newAgent', hostId: host, projectId: id })}><Plus size={14} /> New agent</button>
          </div>
        {/if}
        {#if archived.length}
          <button class="btn ghost sm more" onclick={() => (showArchived = !showArchived)}>
            {showArchived ? 'Hide' : 'Show'} {archived.length} archived
          </button>
          {#if showArchived}
            <div class="card list">{#each archived as a (a.id)}<AgentRow agent={a} showProject={false} />{/each}</div>
          {/if}
        {/if}
      </section>

      <section>
        <h2 class="section-title">
          Schedules {#if schedules}<span class="count">{schedules.length}</span>{/if}
          {#if schedules?.length}
            <button class="btn ghost sm add" onclick={() => (ui.dialog = { kind: 'schedule', hostId: host, projectId: id })}><Plus size={13} /> New schedule</button>
          {/if}
        </h2>
        <div class="card"><SchedulesPanel {host} projectId={id} /></div>
      </section>

      <section class="board-sec">
        <h2 class="section-title">Board</h2>
        <div class="card board-card"><BoardPanel {host} projectId={id} /></div>
      </section>

      <div class="cols">
        {#if p.git}
          <section>
            <h2 class="section-title">Worktrees <span class="count">{p.git.worktrees.length}</span></h2>
            <div class="card table">
              {#each p.git.worktrees as w (w.path)}
                {@const user = agents.find((a) => a.worktree?.path === w.path)}
                <div class="tr">
                  <span class="mono br"><GitBranch size={12} />{w.branch ?? w.head ?? 'detached'}</span>
                  <span class="mono faint ellipsis" title={w.path}>{shortPath(w.path, 2)}</span>
                  {#if w.isMain}<span class="chip">main</span>{/if}
                  {#if w.changed}<span class="accent num">{w.changed} changed</span>{/if}
                  {#if w.operation || w.conflicts}<span class="warn-red">⚠ {w.operation ? OPERATION_LABEL[w.operation] : `${w.conflicts} conflicts`}</span>{/if}
                  {#if w.baseAhead === 0 && !w.changed && !w.operation}
                    <span class="merged" title="Everything on this branch is in {p.git.branch}">✓ merged</span>
                  {:else if w.baseAhead != null}
                    <span class="num faint" title="Against {p.git.branch}">↑{w.baseAhead}</span>
                    <span class="num" class:warn={!!w.baseBehind} class:faint={!w.baseBehind} title="Against {p.git.branch}">↓{w.baseBehind}</span>
                  {/if}
                  <span class="grow"></span>
                  {#if user}<a class="agent-link ellipsis" href={href({ name: 'agent', host, id: user.id })}>{user.title}</a>{/if}
                  {#if !w.isMain}
                    <button class="btn ghost icon sm" title="Remove worktree" aria-label="Remove worktree" onclick={() => removeWorktree(w.path, w.branch)}><Trash2 size={13} /></button>
                  {/if}
                </div>
              {/each}
            </div>
          </section>
          <section>
            <h2 class="section-title">Recent commits</h2>
            <div class="card table">
              {#each commits as c (c.sha)}
                <div class="tr">
                  <span class="mono faint sha">{c.sha.slice(0, 7)}</span>
                  <span class="ellipsis grow">{c.summary}</span>
                  <span class="faint ellipsis author">{c.author}</span>
                  <span class="faint num when">{ago(c.ts, now.value)}</span>
                </div>
              {:else}
                <div class="tr faint">No commits</div>
              {/each}
            </div>
          </section>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .scroll {
    flex: 1;
    overflow-y: auto;
  }
  .term-sec {
    height: min(440px, 60vh);
    overflow: hidden;
    padding: 0;
  }
  .page {
    max-width: 1280px;
    margin: 0 auto;
    padding: 20px 24px 48px;
    display: flex;
    flex-direction: column;
    gap: 26px;
  }
  .phead {
    display: flex;
    flex-direction: column;
    gap: 8px;
    /* The Run and New agent buttons drop their labels when the header gets narrow. */
    container-type: inline-size;
  }
  .ttl {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  h1 {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .spin {
    display: flex;
    animation: spin 0.8s linear infinite;
  }
  .path {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    max-width: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-3);
    font-size: 11.5px;
    cursor: pointer;
    overflow-wrap: anywhere;
    text-align: left;
  }
  .path:hover {
    color: var(--text-2);
  }
  .stats {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .stat {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .stat.mono {
    color: var(--text);
  }
  .stat.last {
    max-width: 420px;
  }
  .dim {
    color: var(--text-3);
  }
  .warn {
    color: var(--amber);
  }
  .warn-red {
    color: var(--red);
    white-space: nowrap;
  }
  .merged {
    color: var(--green);
    white-space: nowrap;
  }
  .pulse {
    display: flex;
    animation: pulse 1s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  .accent {
    color: var(--accent-text);
    white-space: nowrap;
  }
  .urls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
  }
  .url {
    display: inline-flex;
    align-items: center;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg-1);
    font-size: 11.5px;
  }
  .url a {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0 6px 0 8px;
    color: var(--accent-text);
  }
  .url button {
    display: grid;
    place-items: center;
    width: 20px;
    height: 100%;
    border: 0;
    border-left: 1px solid var(--border);
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
  }
  .url button:hover {
    color: var(--red);
  }
  .addurl {
    width: 230px;
    height: 24px;
    font-size: 11.5px;
    border-style: dashed;
    background: transparent;
  }
  .agents-sec {
    container-type: inline-size;
  }
  .board-card {
    padding: 0;
  }
  .add {
    margin-left: auto;
    font-weight: 500;
  }
  .list {
    padding: 4px;
  }
  .more {
    margin-top: 8px;
  }
  .more + .list {
    margin-top: 8px;
  }
  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 20px;
  }
  .table {
    overflow: hidden;
  }
  .tr {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    padding: 4px 12px;
    font-size: var(--fs-sm);
  }
  .tr + .tr {
    border-top: 1px solid var(--border);
  }
  .br {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    flex: none;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .agent-link {
    max-width: 180px;
    color: var(--accent-text);
  }
  .sha {
    font-size: 11px;
    flex: none;
  }
  .author {
    max-width: 120px;
    flex: none;
  }
  .when {
    width: 30px;
    text-align: right;
    flex: none;
  }
  @media (max-width: 960px) {
    /* minmax(0, …): a plain 1fr track grows to the min-content width of the
       nowrap commit/worktree rows and makes the page scroll sideways. */
    .cols {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  /* Smallest phones (~320 px): the action row no longer fits with the label. */
  @container (max-width: 320px) {
    .acts .lbl {
      display: none;
    }
  }
  @media (max-width: 640px) {
    .page {
      padding: 16px 12px 40px;
    }
    .ttl {
      flex-wrap: wrap;
    }
    .author {
      display: none;
    }
  }
</style>
