<script lang="ts">
  import { tick } from 'svelte';
  import { key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import { agentState } from '../lib/status';
  import { openExternal } from '../lib/external';
  import type { AgentSummary, MergeMethod, PullRequestInfo } from '../lib/protocol';
  import {
    MERGE_LABEL,
    OPERATION_LABEL,
    askAgent,
    askToUpdate,
    createPr,
    fastForward,
    fastForwardBlocker,
    fetchedLabel,
    fetchProject,
    fetching,
    fixChecksPrompt,
    isStale,
    mergeBlocker,
    mergeMethodFor,
    mergeMethods,
    mergePr,
    plural,
    prBlocker,
    prBusy,
    prDraft,
    prErrors,
    prLoading,
    push,
    pushBlocker,
    pushHint,
    pushing,
    isWorking,
    refreshPr,
    rememberMergeMethod,
    showPush,
    wantsPrStatus,
    writePrPrompt,
    wtKey,
    type AgentGit,
  } from '../lib/git.svelte';
  import { ArrowDownToLine, ArrowUpFromLine, Bot, Check, ChevronDown, GitBranch, GitMerge, GitPullRequest, RefreshCw, Trash2 } from '../lib/icons';
  import StatusDot from './StatusDot.svelte';
  import PrBadge from './PrBadge.svelte';

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
  /** Mid-turn the agent may still commit, push or rewrite the branch: actions wait for the answer. */
  const answering = $derived(isWorking(a));
  const askLabel = $derived(g.severity === 'bad' ? 'Ask agent to resolve' : 'Ask agent to update');
  const showAsk = $derived(live && (g.severity === 'bad' || g.behind > 0));
  /** The base the worktree was created from, when the project has moved to another branch. */
  const createdFrom = $derived(g.kind === 'worktree' && a.worktree && g.against && a.worktree.base !== g.against ? a.worktree.base : undefined);
  /** The one signal kept when the header is narrow. */
  const primary = $derived(g.operation || g.conflicts ? 'bad' : g.behind ? 'behind' : g.merged ? 'merged' : '');
  /** ↑ in the chip: unpushed commits when there is an upstream, else ahead of the base. */
  const chipAhead = $derived(g.unpushed ?? g.ahead);
  const title = $derived(
    [
      `${g.kind === 'checkout' ? 'Project checkout' : 'Worktree'} on ${g.detached ? `detached HEAD ${g.label}` : g.label}`,
      g.against ? `${g.ahead} ahead, ${g.behind} behind ${g.against}` : '',
      g.unpushed && g.kind === 'worktree' ? `${plural(g.unpushed, 'unpushed commit')}` : '',
      g.dirty ? `${g.dirty} uncommitted` : '',
      g.shared.length > 1 ? `shared by ${g.shared.length} agents` : '',
      g.pr ? `PR #${g.pr.number} ${g.pr.state}` : '',
    ]
      .filter(Boolean)
      .join(' · '),
  );

  // ─── push / pull request ───────────────────────────────────────────────────────────────────
  const wk = $derived(wtKey(g));
  const pr = $derived(g.pr);
  const prOpen = $derived(pr?.state === 'open');
  /** Why the PR row has no PR features (only linked worktrees have one). */
  const prBlk = $derived(g.kind === 'worktree' ? prBlocker(g) : undefined);
  const pushVisible = $derived(showPush(g));
  const pushBlk = $derived(pushBlocker(g));
  const busyPush = $derived(!!pushing[wk]);
  const busyPr = $derived(prBusy[wk]);
  const loadingPr = $derived(!!prLoading[wk]);
  const prError = $derived(prErrors[wk]);
  const canCreatePr = $derived(g.kind === 'worktree' && !prBlk && !prOpen);
  const methods = $derived(mergeMethods(g.github));
  const mergeBlk = $derived(pr && prOpen ? mergeBlocker(pr) : undefined);
  let method = $state<MergeMethod>('squash');
  let methodsOpen = $state(false);
  let deleteBranch = $state(false);

  const MERGE_STATE: Record<string, [string, string]> = {
    CLEAN: ['mergeable', ''],
    HAS_HOOKS: ['mergeable', ''],
    UNSTABLE: ['unstable', 'amber'],
    BLOCKED: ['blocked', 'amber'],
    BEHIND: ['behind base', 'amber'],
    DIRTY: ['conflict', 'red'],
    DRAFT: ['draft', ''],
    UNKNOWN: ['unknown', ''],
  };
  const REVIEW: Record<NonNullable<PullRequestInfo['review']>, [string, string]> = {
    approved: ['approved', 'green'],
    changesRequested: ['changes requested', 'red'],
    reviewRequired: ['review required', 'amber'],
  };

  function pickMethod(m: MergeMethod) {
    method = m;
    methodsOpen = false;
    rememberMergeMethod(g.project, m);
  }

  // The "New pull request" form replaces the popover's content while open.
  let form = $state(false);
  let filling = $state(false);
  let prTitle = $state('');
  let prBody = $state('');
  let prDraftOnly = $state(false);

  async function openForm() {
    form = true;
    filling = true;
    prDraftOnly = false;
    try {
      const d = await prDraft(a, g);
      prTitle = d.title;
      prBody = d.body;
    } finally {
      filling = false;
    }
  }

  async function submitPr() {
    if (await createPr(g, { title: prTitle.trim(), body: prBody, draft: prDraftOnly })) form = false;
  }

  function letAgentWrite() {
    form = false;
    run(() => askAgent(a, writePrPrompt(g)));
  }

  async function show() {
    sheet = matchMedia('(max-width: 860px)').matches;
    open = true;
    method = mergeMethodFor(g.project, methods);
    deleteBranch = g.github?.deleteBranchOnMerge ?? false;
    methodsOpen = false;
    if (wantsPrStatus(g, Date.now())) void refreshPr(g);
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
  {#if g.merged && pr?.state !== 'merged'}<span class="ok" class:minor={primary !== 'merged'}>✓ merged</span>{/if}
  {#if chipAhead && !g.merged}<span class="minor">↑{chipAhead}</span>{/if}
  {#if g.behind && !g.merged}<span class="warn" class:minor={primary !== 'behind'}>↓{g.behind}</span>{/if}
  {#if pr}<PrBadge {pr} />{/if}
  {#if g.dirty}<span class="dirty minor">{g.dirty}</span>{/if}
  {#if g.shared.length > 1}<span class="faint minor">×{g.shared.length}</span>{/if}
</button>

{#if open}
  {#if sheet}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="backdrop" use:portal onclick={() => (open = false)}></div>
  {/if}
  <div class="gpop" class:sheet style={sheet ? '' : place} bind:this={pop} use:portal role="dialog" aria-label={form ? 'New pull request' : 'Git status'}>
    {#if form}
      <div class="hd">
        <div class="row1"><GitPullRequest size={14} />New pull request</div>
        <div class="sub mono ellipsis">{g.label} → {g.base ?? 'default branch'}{#if g.github}{' · '}{g.github.repo}{/if}</div>
      </div>
      <div class="prform">
        <label class="field"><span>Title</span><input class="input" bind:value={prTitle} disabled={filling} placeholder={filling ? 'Loading…' : ''} /></label>
        <label class="field"><span>Description</span><textarea class="textarea mono" rows="8" bind:value={prBody} disabled={filling}></textarea></label>
        <label class="chk"><input type="checkbox" bind:checked={prDraftOnly} /> Draft</label>
      </div>
      <div class="acts">
        <button class="btn sm primary" disabled={filling || !prTitle.trim() || !!busyPr || answering} onclick={submitPr}>
          {#if busyPr === 'create'}<span class="spinner"></span>{/if}Push &amp; create
        </button>
        {#if live}
          <button class="btn sm" disabled={!!busyPr || answering} onclick={letAgentWrite}><Bot size={13} />Let the agent write it</button>
        {/if}
        <button class="btn sm end" onclick={() => (form = false)}>Cancel</button>
        <span class="why">{#if answering}Unavailable while the agent is answering. {/if}Pushes first if needed. Base: <span class="mono">{g.base ?? 'the default branch'}</span> (the project checkout's branch).</span>
      </div>
    {:else}
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
          {:else if g.remote}
            <span class="faint">not pushed yet</span>
          {:else}
            <span class="faint">none (local branch)</span>
          {/if}
        </dd>
        {#if g.kind === 'worktree'}
          <dt>Pull request</dt>
          <dd>
            {#if prBlk}
              <span class="faint">{prBlk}</span>
            {:else if pr}
              <button class="link prl" title={pr.url} onclick={() => openExternal(pr.url)}>
                <span class="ellipsis" class:closed={pr.state === 'closed'}>#{pr.number} {pr.title}</span>↗
              </button>
            {:else if loadingPr && !g.wt.prCheckedAt}
              <span class="faint">checking…</span>
            {:else if prError}
              <span class="err">{prError}</span>
            {:else}
              <span class="faint">none</span>
            {/if}
            {#if loadingPr && !prBlk}<span class="ic spin faint" title="Updating the pull request status"><RefreshCw size={11} /></span>{/if}
          </dd>
          {#if pr && !prBlk}
            <dt></dt>
            <dd>
              {#if pr.state === 'merged'}
                <span class="pill violet">merged</span>
              {:else if pr.state === 'closed'}
                <span class="pill">closed</span>
              {:else}
                {#if pr.draft}<span class="pill">draft</span>{/if}
                {@const running = !pr.checks.failed && pr.checks.pending > 0}
                {#if pr.checks.total}
                  <span class="pill" class:red={pr.checks.failed > 0} class:accent={running} class:green={!pr.checks.failed && !pr.checks.pending}>
                    {running ? `◷ checks running ${pr.checks.passed}/${pr.checks.total}` : `checks ${pr.checks.passed}/${pr.checks.total}`}
                  </span>
                {/if}
                {#if pr.review}<span class="pill {REVIEW[pr.review][1]}">{REVIEW[pr.review][0]}</span>{/if}
                <!-- GitHub reports UNSTABLE/BLOCKED while checks run; the checks pill already says so. -->
                {#if !(running && (pr.mergeState === 'UNSTABLE' || pr.mergeState === 'BLOCKED'))}
                  {@const ms = MERGE_STATE[pr.mergeState] ?? [pr.mergeState.toLowerCase(), '']}
                  <span class="pill {ms[1]}">{ms[0]}</span>
                {/if}
              {/if}
            </dd>
            {#if prOpen && pr.checks.failed}
              <dt></dt>
              <dd class="checks">
                {#each pr.checks.failing as c (c.name)}
                  <span><span class="bad">✗</span><span class="ellipsis">{c.name}</span>{#if c.url}{@const url = c.url}<button class="link" onclick={() => openExternal(url)}>details ↗</button>{/if}</span>
                {/each}
                {#if pr.checks.pending}<span><span class="running">◷</span><span class="faint">{pr.checks.pending} running</span></span>{/if}
                {#if pr.checks.passed}<span><span class="ok">✓</span><span class="faint">{pr.checks.passed} {pr.checks.failing.length ? 'more ' : ''}passed</span></span>{/if}
              </dd>
            {/if}
            {#if prError}
              <dt></dt>
              <dd class="err note">Refreshing failed: {prError}</dd>
            {/if}
          {/if}
        {/if}
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
        {#if pushVisible}
          <button class="btn sm" disabled={!!pushBlk || busyPush || answering} onclick={() => (g.kind === 'checkout' ? run(() => push(a, g)) : push(a, g))}>
            {#if busyPush}<span class="spinner"></span>{:else}<ArrowUpFromLine size={13} />{/if}Push{g.kind === 'checkout' ? '…' : ''}
          </button>
        {/if}
        {#if canCreatePr}
          <button class="btn sm" class:primary={!showAsk && !pr} disabled={answering} onclick={openForm}><GitPullRequest size={13} />Create pull request…</button>
        {/if}
        {#if pr && prOpen && !prBlk}
          <span class="split">
            <button class="btn sm" class:primary={!mergeBlk} disabled={!!mergeBlk || !!busyPr || answering} onclick={() => mergePr(g, pr, method, deleteBranch)}>
              {#if busyPr === 'merge'}<span class="spinner"></span>{:else}<GitMerge size={13} />{/if}{MERGE_LABEL[method]}
            </button>
            {#if methods.length > 1}
              <button
                class="btn sm caret"
                class:primary={!mergeBlk}
                disabled={!!busyPr}
                aria-label="Merge method"
                aria-haspopup="menu"
                aria-expanded={methodsOpen}
                onclick={() => (methodsOpen = !methodsOpen)}><ChevronDown size={13} /></button
              >
            {/if}
          </span>
          {#if methodsOpen}
            <div class="methods" role="menu">
              {#each methods as m (m)}
                <button role="menuitemradio" aria-checked={m === method} onclick={() => pickMethod(m)}>
                  <span class="ic">{#if m === method}<Check size={12} />{/if}</span>{MERGE_LABEL[m]}
                </button>
              {/each}
            </div>
          {/if}
        {/if}
        {#if pr && prOpen && pr.checks.failed && live}
          <button class="btn sm primary" disabled={answering} onclick={() => run(() => askAgent(a, fixChecksPrompt(pr)))}><Bot size={13} />Ask agent to fix checks</button>
        {/if}
        {#if showFastForward}
          <button class="btn sm" disabled={!!ffBlocker} onclick={() => run(() => fastForward(g))}><ArrowDownToLine size={13} />Fast-forward</button>
        {/if}
        {#if showAsk}
          <button class="btn sm primary" disabled={answering} onclick={() => run(() => askToUpdate(a, g, now.value))}><Bot size={13} />{askLabel}</button>
        {/if}
        {#if g.cleanable && a.worktree && oncleanup}
          <button class="btn sm danger" disabled={answering} onclick={() => run(oncleanup)}><Trash2 size={13} />Clean up worktree…</button>
        {/if}
        {#if pr && prOpen && !prBlk}
          <label class="chk wide"><input type="checkbox" bind:checked={deleteBranch} /> Delete remote branch after merge</label>
        {/if}
        {#if answering}
          <span class="why">Git actions are unavailable while the agent is answering.</span>
        {/if}
        {#if pushVisible && pushBlk}
          <span class="why">Push unavailable: {pushBlk}.</span>
        {:else if pushVisible && g.kind === 'worktree'}
          <span class="why">{pushHint(g)}</span>
        {/if}
        {#if pr && prOpen && !prBlk}
          {#if mergeBlk}
            <span class="why">Merge unavailable: {mergeBlk}.</span>
          {:else}
            <span class="why">Merges on GitHub (<span class="mono">gh pr merge --{method}</span>). Afterwards: fetch, and “Clean up worktree” appears.</span>
          {/if}
        {/if}
        {#if showFastForward && ffBlocker}<span class="why">Fast-forward unavailable: {ffBlocker}.</span>{/if}
        {#if g.cleanable && a.worktree && oncleanup}<span class="why">Deletes this agent, its worktree and the merged branch.</span>{/if}
      </div>
    {/if}
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
  .running {
    color: var(--accent-text);
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
  .pill.violet {
    background: var(--violet-soft);
    color: var(--violet);
  }
  .err {
    color: var(--red);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .closed {
    text-decoration: line-through;
  }
  dd .link.prl {
    max-width: 100%;
    min-width: 0;
    margin-left: 0;
  }
  dd.checks {
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
  }
  dd.checks > span {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .prform {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 13px 8px;
  }
  .prform .textarea {
    font-size: 11.5px;
  }
  .chk {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .chk.wide {
    flex-basis: 100%;
  }
  .end {
    margin-left: auto;
  }
  .split {
    display: inline-flex;
  }
  .split .btn:not(:last-child) {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }
  .split .caret {
    padding: 0 5px;
    border-left-color: color-mix(in srgb, currentColor 30%, transparent);
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
  }
  .methods {
    display: flex;
    flex-direction: column;
    flex-basis: 100%;
    padding: 3px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg);
  }
  .methods button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .methods button:hover {
    background: var(--bg-hover);
  }
  .methods .ic {
    width: 12px;
  }
  .acts .spinner {
    width: 11px;
    height: 11px;
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
