<script lang="ts">
  import { tick } from 'svelte';
  import { key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import { agentState } from '../lib/status';
  import type { AgentSummary } from '../lib/protocol';
  import {
    OPERATION_LABEL,
    askToUpdate,
    fastForward,
    fastForwardBlocker,
    fetchedLabel,
    fetchProject,
    fetching,
    isStale,
    type AgentGit,
  } from '../lib/git.svelte';
  import { ArrowDownToLine, Bot, GitBranch, RefreshCw, Trash2 } from '../lib/icons';
  import StatusDot from './StatusDot.svelte';

  // Git status of the agent's directory in the agent header; the popover has the details and
  // the actions (a bottom sheet on phones).
  let { agent: a, git: g, oncleanup }: { agent: AgentSummary; git: AgentGit; oncleanup?: () => void } = $props();

  let open = $state(false);
  let sheet = $state(false);
  let place = $state('');
  let trigger = $state<HTMLButtonElement>();
  let pop = $state<HTMLDivElement>();

  const busyFetch = $derived(!!fetching[key(g.project.hostId, g.project.id)]);
  const stale = $derived(isStale(g.fetchedAt, now.value));
  const ffBlocker = $derived(fastForwardBlocker(g));
  const showFastForward = $derived(!!g.wt.upstream && g.wt.behind > 0);
  const live = $derived(a.status !== 'stopped');
  const askLabel = $derived(g.severity === 'bad' ? 'Ask agent to resolve' : 'Ask agent to update');
  const showAsk = $derived(live && (g.severity === 'bad' || g.behind > 0));
  /** The base the worktree was created from, when the project has moved to another branch. */
  const createdFrom = $derived(g.kind === 'worktree' && a.worktree && g.against && a.worktree.base !== g.against ? a.worktree.base : undefined);
  /** The one signal kept when the header is narrow. */
  const primary = $derived(g.operation || g.conflicts ? 'bad' : g.behind ? 'behind' : g.merged ? 'merged' : '');
  const title = $derived(
    [
      `${g.kind === 'checkout' ? 'Project checkout' : 'Worktree'} on ${g.detached ? `detached HEAD ${g.label}` : g.label}`,
      g.against ? `${g.ahead} ahead, ${g.behind} behind ${g.against}` : '',
      g.dirty ? `${g.dirty} uncommitted` : '',
      g.shared.length > 1 ? `shared by ${g.shared.length} agents` : '',
    ]
      .filter(Boolean)
      .join(' · '),
  );

  async function show() {
    sheet = matchMedia('(max-width: 860px)').matches;
    open = true;
    await tick();
    position();
  }

  function position() {
    if (!trigger || sheet) return;
    const r = trigger.getBoundingClientRect();
    const width = Math.min(380, window.innerWidth - 16);
    const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
    place = `left:${left}px;width:${width}px;top:${r.bottom + 6}px;max-height:${window.innerHeight - r.bottom - 18}px`;
  }

  /** Run an action from the popover and close it. */
  function run(fn: () => unknown) {
    open = false;
    void fn();
  }

  $effect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!pop?.contains(e.target as Node) && !trigger?.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      open = false;
      trigger?.focus();
    };
    const reflow = () => position();
    document.addEventListener('pointerdown', close);
    document.addEventListener('keydown', esc, true);
    window.addEventListener('resize', reflow);
    return () => {
      document.removeEventListener('pointerdown', close);
      document.removeEventListener('keydown', esc, true);
      window.removeEventListener('resize', reflow);
    };
  });

  /** Render at the end of `<body>`, so the header's overflow cannot clip it. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<button
  type="button"
  class="gchip mono num"
  data-sev={g.severity}
  class:on={open}
  bind:this={trigger}
  onclick={() => (open ? (open = false) : show())}
  aria-haspopup="dialog"
  aria-expanded={open}
  {title}
>
  <GitBranch size={12} />
  <span class="lbl ellipsis" class:detached={g.detached}>{g.label}</span>
  {#if g.operation}
    <span class="bad" class:minor={primary !== 'bad'}>⚠ {OPERATION_LABEL[g.operation]}</span>
  {:else if g.conflicts}
    <span class="bad" class:minor={primary !== 'bad'}>⚠ {g.conflicts}</span>
  {/if}
  {#if g.merged}<span class="ok" class:minor={primary !== 'merged'}>✓ merged</span>{/if}
  {#if g.ahead && !g.merged}<span class="minor">↑{g.ahead}</span>{/if}
  {#if g.behind && !g.merged}<span class="warn" class:minor={primary !== 'behind'}>↓{g.behind}</span>{/if}
  {#if g.dirty}<span class="dirty minor">{g.dirty}</span>{/if}
  {#if g.shared.length > 1}<span class="faint minor">×{g.shared.length}</span>{/if}
</button>

{#if open}
  {#if sheet}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="backdrop" use:portal onclick={() => (open = false)}></div>
  {/if}
  <div class="gpop" class:sheet style={sheet ? '' : place} bind:this={pop} use:portal role="dialog" aria-label="Git status">
    <div class="hd">
      <div class="row1 mono">
        <GitBranch size={14} /><span class="ellipsis" class:detached={g.detached}>{g.label}</span>
        <span class="tag">{g.kind === 'checkout' ? 'Project checkout' : 'Worktree'}</span>
      </div>
      <div class="sub mono ellipsis" title={g.wt.path}>
        {#if g.kind === 'worktree' && g.against}from {g.against} · {/if}{g.wt.path}{#if g.wt.head} · HEAD {g.wt.head}{/if}
      </div>
    </div>
    <dl>
      {#if g.operation || g.conflicts}
        <dt>State</dt>
        <dd>
          {#if g.operation}<span class="pill red">{OPERATION_LABEL[g.operation]} in progress</span>{/if}
          {#if g.conflicts}<span class="pill red">{g.conflicts} conflict{g.conflicts === 1 ? '' : 's'}</span>{/if}
        </dd>
      {/if}
      {#if g.kind === 'worktree'}
        <dt>vs {g.against ?? 'base'}</dt>
        <dd>
          {#if g.against}
            <span class="pill">↑ {g.ahead}</span><span class="pill" class:amber={g.behind > 0}>↓ {g.behind}</span>
            {#if g.merged}<span class="pill green">✓ fully merged</span>{/if}
          {:else}
            <span class="faint">no base branch</span>
          {/if}
        </dd>
        {#if createdFrom}
          <dt></dt>
          <dd class="faint note">Created from {createdFrom}; the project is on {g.against} now.</dd>
        {/if}
      {/if}
      <dt>Upstream</dt>
      <dd>
        {#if g.wt.upstream}
          <span class="mono">{g.wt.upstream}</span>
          <span class="pill">↑ {g.wt.ahead}</span><span class="pill" class:amber={g.wt.behind > 0}>↓ {g.wt.behind}{g.wt.behind ? ' behind' : ''}</span>
        {:else}
          <span class="faint">none (local branch)</span>
        {/if}
      </dd>
      <dt>Last fetch</dt>
      <dd>
        <span class:stale title={g.fetchedAt ? new Date(g.fetchedAt).toLocaleString() : undefined}>{fetchedLabel(g.fetchedAt, now.value)}</span>
        <button class="link" onclick={() => fetchProject(g.project)} disabled={busyFetch}>
          <span class="ic" class:spin={busyFetch}><RefreshCw size={12} /></span>Fetch
        </button>
      </dd>
      <dt>Uncommitted</dt>
      <dd>
        {#if g.dirty}
          <span class="pill accent">{g.dirty} file{g.dirty === 1 ? '' : 's'}</span>
          <button class="link" onclick={() => run(() => ui.showPanel('changes'))}>Open changes →</button>
        {:else}
          <span class="faint">none</span>
        {/if}
      </dd>
      {#if g.kind === 'checkout' && g.project.git?.lastCommit}
        {@const c = g.project.git.lastCommit}
        <dt>Last commit</dt>
        <dd class="commit"><span class="mono faint">{c.sha.slice(0, 7)}</span> <span class="ellipsis">{c.summary}</span> <span class="faint">· {ago(c.ts, now.value)}</span></dd>
      {/if}
      {#if g.shared.length > 1}
        <dt>Shared with</dt>
        <dd class="shared">
          {#each g.shared as x (x.id)}
            <span><StatusDot state={agentState(x)} size={6} /><span class="ellipsis">{x.title}</span>{#if x.id === a.id}<span class="faint">(this)</span>{/if}</span>
          {/each}
        </dd>
      {/if}
    </dl>
    {#if g.shared.length > 1}
      <p class="hint-box">
        {g.shared.length} agents work in this checkout, so its changes cannot be told apart. For parallel work, give each its own
        <button class="link" onclick={() => run(() => (ui.dialog = { kind: 'newAgent', hostId: g.project.hostId, projectId: g.project.id }))}>worktree</button>.
      </p>
    {/if}
    <div class="acts">
      <button class="btn sm" onclick={() => fetchProject(g.project)} disabled={busyFetch}>
        <span class="ic" class:spin={busyFetch}><RefreshCw size={13} /></span>Fetch
      </button>
      {#if showFastForward}
        <button class="btn sm" disabled={!!ffBlocker} onclick={() => run(() => fastForward(g))}><ArrowDownToLine size={13} />Fast-forward</button>
      {/if}
      {#if showAsk}
        <button class="btn sm primary" onclick={() => run(() => askToUpdate(a, g, now.value))}><Bot size={13} />{askLabel}</button>
      {/if}
      {#if g.merged && a.worktree && oncleanup}
        <button class="btn sm danger" onclick={() => run(oncleanup)}><Trash2 size={13} />Clean up worktree…</button>
      {/if}
      {#if showFastForward && ffBlocker}<span class="why">Fast-forward unavailable: {ffBlocker}.</span>{/if}
      {#if g.merged && a.worktree && oncleanup}<span class="why">Deletes this agent, its worktree and the merged branch.</span>{/if}
    </div>
  </div>
{/if}

<style>
  .gchip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 0 1 auto;
    min-width: 0;
    max-width: 300px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: var(--bg-1);
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
  }
  .gchip :global(svg) {
    flex: none;
  }
  .gchip:hover,
  .gchip.on {
    background: var(--bg-2);
    border-color: var(--border-strong);
  }
  .gchip[data-sev='warn'] {
    border-color: color-mix(in srgb, var(--amber) 45%, var(--border));
    background: var(--amber-soft);
  }
  .gchip[data-sev='bad'] {
    border-color: color-mix(in srgb, var(--red) 45%, var(--border));
    background: var(--red-soft);
  }
  .lbl {
    color: var(--text);
  }
  .detached {
    font-style: italic;
  }
  .warn {
    color: var(--amber);
  }
  .bad {
    color: var(--red);
  }
  .ok {
    color: var(--green);
  }
  .dirty {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .dirty::before {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  span {
    flex: none;
  }
  .lbl,
  .ellipsis {
    flex: 0 1 auto;
  }
  @container (max-width: 600px) {
    .gchip .lbl,
    .gchip .minor {
      display: none;
    }
    .gchip {
      padding: 0 6px;
    }
  }

  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
    background: var(--overlay);
  }
  .gpop {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
    animation: pop var(--t-fast) var(--ease);
  }
  .gpop.sheet {
    left: 0;
    right: 0;
    bottom: 0;
    max-height: 80vh;
    border-radius: 14px 14px 0 0;
    padding-bottom: env(safe-area-inset-bottom);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(2px);
    }
  }
  .hd {
    padding: 11px 13px 9px;
    border-bottom: 1px solid var(--border);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
    font-weight: 600;
  }
  .tag {
    flex: none;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 99px;
    color: var(--text-3);
    font-family: var(--font);
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .sub {
    margin-top: 3px;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  dl {
    display: grid;
    grid-template-columns: 92px minmax(0, 1fr);
    gap: 7px 10px;
    margin: 0;
    padding: 10px 13px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    min-width: 0;
    margin: 0;
  }
  dd.commit {
    flex-wrap: nowrap;
  }
  dd.shared {
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
  }
  dd.shared > span {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
  }
  .note {
    font-size: var(--fs-xs);
  }
  .pill {
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--bg-3);
    color: var(--text-2);
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 600;
  }
  .pill.amber {
    background: var(--amber-soft);
    color: var(--amber);
  }
  .pill.green {
    background: var(--green-soft);
    color: var(--green);
  }
  .pill.red {
    background: var(--red-soft);
    color: var(--red);
  }
  .pill.accent {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .stale {
    color: var(--amber);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .link:disabled {
    opacity: 0.6;
    cursor: default;
  }
  dd .link {
    margin-left: auto;
  }
  .hint-box {
    margin: 0 13px 10px;
    padding: 7px 9px;
    border-radius: var(--r);
    background: var(--bg-2);
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 10px 13px 12px;
    border-top: 1px solid var(--border);
  }
  .acts .btn {
    gap: 6px;
  }
  .why {
    flex-basis: 100%;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .ic {
    display: flex;
  }
  .spin {
    animation: spin 0.8s linear infinite;
  }
</style>
