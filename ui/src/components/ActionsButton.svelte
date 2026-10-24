<script lang="ts">
  // ▶ Run split button for the project's `.coocoo/actions.toml`: the main part starts the default
  // action (or shows its terminal while it runs), the caret lists every action of this view's
  // scope with its run status and the file's problems.
  import { now } from '../lib/clock.svelte';
  import { ago, shortPath } from '../lib/format';
  import { ChevronDown, RotateCw, Square } from '../lib/icons';
  import { keepInView } from '../lib/popover';
  import type { AgentSummary, Project, ProjectAction, TerminalInfo } from '../lib/protocol';
  import {
    actionDetail,
    actionDisabled,
    actionIcon,
    actionRun,
    actionsFor,
    baseDir,
    type ActionContext,
    showRun,
    startAction,
    stopRun,
  } from '../lib/actions';

  let { project, agent }: { project: Project; agent?: AgentSummary } = $props();

  const ctx = $derived<ActionContext>({ project, agent });
  const host = $derived(project.hostId);
  const list = $derived(actionsFor(ctx));
  const errors = $derived(project.actionErrors ?? []);
  const main = $derived(list.find((a) => a.default) ?? list[0]);
  const runs = $derived(new Map(list.map((a) => [a.id, actionRun(ctx, a)] as const)));
  const live = (r: TerminalInfo | undefined): r is TerminalInfo => !!r && !r.exitedAt;
  const mainRun = $derived(main ? runs.get(main.id) : undefined);
  const otherLive = $derived(list.some((a) => a !== main && live(runs.get(a.id))));

  let open = $state(false);
  let root = $state<HTMLDivElement>();
  /** Opens above the button when there is no room below. */
  let up = $state(false);

  function toggle() {
    if (!open && root) {
      const r = root.getBoundingClientRect();
      up = r.bottom + 320 > innerHeight && r.top > innerHeight - r.bottom;
    }
    open = !open;
  }

  function pick(a: ProjectAction) {
    open = false;
    const r = runs.get(a.id);
    if (live(r)) showRun(host, r, !!agent);
    else startAction(a, ctx);
  }

  $effect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root?.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        open = false;
      }
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', esc, true);
    };
  });
</script>

{#if list.length || errors.length}
  <div class="run" bind:this={root}>
    <div class="split" class:live={live(mainRun)}>
      {#if main}
        {@const Icon = actionIcon(main)}
        {@const why = actionDisabled(main, ctx)}
        <button
          class="main"
          disabled={!!why}
          title={why ?? (live(mainRun) ? `${main.name} is running — show its terminal` : `${main.name}: ${actionDetail(main, ctx)}`)}
          onclick={() => main && pick(main)}
        >
          {#if live(mainRun)}<span class="dot on"></span>{:else}<Icon size={13} />{/if}
          <span class="lbl ellipsis">{main.name}</span>
        </button>
      {/if}
      <button class="caret" class:solo={!main} aria-label="Actions" aria-haspopup="menu" aria-expanded={open} title="Actions from .coocoo/actions.toml" onclick={toggle}>
        {#if errors.length}<span class="dot warn"></span>{:else if otherLive}<span class="dot on"></span>{/if}
        <ChevronDown size={13} />
      </button>
    </div>
    {#if open}
      <div class="pop" class:up role="menu" use:keepInView>
        <div class="grp">{agent ? 'Agent' : 'Project'} · <span class="mono">{shortPath(baseDir(ctx), 3)}</span></div>
        {#each list as a (a.id)}
          {@const Icon = actionIcon(a)}
          {@const r = runs.get(a.id)}
          {@const why = actionDisabled(a, ctx)}
          <div class="it">
            <button class="body" role="menuitem" disabled={!!why} title={why ?? actionDetail(a, ctx)} onclick={() => pick(a)}>
              <Icon size={14} />
              <span class="t">
                <span class="name ellipsis">{a.name}{#if a === main}<span class="def">default</span>{/if}</span>
                <span class="detail mono ellipsis">{actionDetail(a, ctx)}</span>
              </span>
            </button>
            {#if r && !r.exitedAt}
              <span class="st ok" title="Running since {new Date(r.createdAt).toLocaleTimeString()}"><span class="dot on"></span>{ago(r.createdAt, now.value)}</span>
              <button class="mini" title="Restart" aria-label="Restart {a.name}" onclick={() => ((open = false), startAction(a, ctx))}><RotateCw size={13} /></button>
              <button class="mini" title="Stop" aria-label="Stop {a.name}" onclick={() => stopRun(host, r)}><Square size={12} /></button>
            {:else if r}
              <button
                class="st mini-st"
                class:ok={r.exitCode === 0}
                class:bad={r.exitCode != null && r.exitCode !== 0}
                title="Show the output"
                onclick={() => ((open = false), showRun(host, r, !!agent))}
              >
                {#if r.exitCode === 0}✓ {ago(r.exitedAt ?? r.createdAt, now.value)}{:else if r.exitCode == null}stopped{:else}✗ exit {r.exitCode}{/if}
              </button>
            {/if}
          </div>
        {/each}
        {#if errors.length}
          <div class="errs mono">{#each errors as e (e)}<div>{e}</div>{/each}</div>
        {/if}
        <div class="foot">
          <span class="mono">.coocoo/actions.toml</span>
          <span>{list.length} action{list.length === 1 ? '' : 's'}{errors.length ? ` · ${errors.length} error${errors.length === 1 ? '' : 's'}` : ''}</span>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .run {
    position: relative;
    flex: none;
    min-width: 0;
    margin-right: 4px;
  }
  .split {
    display: flex;
    align-items: stretch;
    height: 26px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg-2);
  }
  .split button {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .split button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .split button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .main {
    max-width: 180px;
    padding: 0 9px;
    border-radius: var(--r) 0 0 var(--r);
  }
  .split.live .main {
    color: var(--green);
  }
  .caret {
    gap: 4px !important;
    padding: 0 6px;
    border-left: 1px solid var(--border-strong) !important;
    border-radius: 0 var(--r) var(--r) 0;
    color: var(--text-2) !important;
  }
  .caret.solo {
    border-left: 0 !important;
    border-radius: var(--r);
  }
  .dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
  }
  .dot.on {
    background: var(--green);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--green) 20%, transparent);
  }
  .dot.warn {
    background: var(--amber);
  }
  /* Narrow header (phone, slim chat pane): icon or status dot plus caret only. */
  @container (max-width: 720px) {
    .lbl {
      display: none;
    }
    .main {
      padding: 0 8px;
    }
  }
  .pop {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 40;
    width: min(360px, calc(100vw - 24px));
    max-height: min(70vh, 520px);
    overflow-y: auto;
    padding: 5px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    animation: pop var(--t-fast) var(--ease);
  }
  .pop.up {
    top: auto;
    bottom: calc(100% + 4px);
  }
  .grp {
    padding: 5px 8px 4px;
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .grp .mono {
    text-transform: none;
    letter-spacing: 0;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-right: 4px;
    border-radius: var(--r);
  }
  .it:hover {
    background: var(--bg-hover);
  }
  .body {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 9px;
    padding: 6px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .body:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* A long command line must not squeeze the icon. */
  .body :global(svg) {
    flex: none;
  }
  .t {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-weight: 500;
  }
  .def {
    margin-left: 6px;
    padding: 0 4px;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    font-size: 10px;
    font-weight: 400;
    color: var(--text-3);
  }
  .detail {
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .st {
    display: flex;
    flex: none;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    white-space: nowrap;
    color: var(--text-3);
  }
  .mini-st {
    padding: 2px 6px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    cursor: pointer;
  }
  .mini-st:hover {
    background: var(--bg-3);
  }
  .ok {
    color: var(--green);
  }
  .bad {
    color: var(--red);
  }
  .mini {
    display: grid;
    place-items: center;
    flex: none;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }
  .mini:hover {
    background: var(--bg-3);
    color: var(--text);
  }
  .errs {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 4px 4px 2px;
    padding: 7px 9px;
    border-radius: var(--r);
    background: color-mix(in srgb, var(--red) 9%, transparent);
    color: var(--red);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin-top: 4px;
    padding: 6px 8px 3px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
</style>
