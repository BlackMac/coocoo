<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { Terminal, type ITheme } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { fleet, key } from '../lib/fleet.svelte';
  import { shortPath } from '../lib/format';
  import { ui } from '../lib/ui.svelte';
  import { closeRun, restartRun, stopRun } from '../lib/actions';
  import { Plus, RotateCw, Square, X } from '../lib/icons';
  import type { TerminalInfo } from '../lib/protocol';

  // The terminals of `cwd` on `host`: its shell, and the runs of project actions started for this
  // directory (chips switch between them). Both live on the daemon: closing the panel only
  // detaches, and opening it again re-attaches (one shell per directory) and replays the output.
  let { host, cwd }: { host: string; cwd: string } = $props();

  let el = $state<HTMLDivElement>();
  let term = $state<Terminal>();
  let fit: FitAddon | undefined;
  /** Attached terminal id. */
  let termId = $state<string | null>(null);
  let shell = $state('');
  /** Exit code once the process ended (`null` = unknown); `undefined` while it runs. */
  let exited = $state<number | null | undefined>(undefined);
  let error = $state('');
  /** Bumped to start a fresh shell instead of re-attaching. */
  let generation = $state(0);
  let forceNew = false;

  const online = $derived(fleet.conn === 'open' && (fleet.hosts[host]?.online ?? false));

  /** Action runs for this directory, oldest first. */
  const runs = $derived(
    Object.entries(fleet.terminals)
      .filter(([k, t]) => k.startsWith(`${host}:`) && t.action?.base === cwd)
      .map(([, t]) => t)
      .sort((a, b) => a.createdAt - b.createdAt),
  );
  /** Shown terminal: `null` = the shell, else an action run's id. */
  let selected = $state<string | null>(null);
  const run = $derived<TerminalInfo | undefined>(selected ? runs.find((r) => r.id === selected) : undefined);

  // A run that was started from the Run button is shown at once.
  $effect(() => {
    const f = ui.terminalFocus;
    if (f && f.host === host && runs.some((r) => r.id === f.id)) {
      selected = f.id;
      ui.terminalFocus = null;
    }
  });

  // A run that went away (closed, or replaced by a restart whose reply is still on its way) falls
  // back to the shell unless something else got selected meanwhile.
  $effect(() => {
    if (!selected || run) return;
    const gone = selected;
    const timer = setTimeout(() => {
      if (selected === gone && !runs.some((r) => r.id === gone)) selected = null;
    }, 800);
    return () => clearTimeout(timer);
  });

  function theme(): ITheme {
    const css = getComputedStyle(document.documentElement);
    const v = (name: string) => css.getPropertyValue(name).trim();
    return {
      background: v('--bg-1'),
      foreground: v('--text'),
      cursor: v('--text'),
      cursorAccent: v('--bg-1'),
      selectionBackground: v('--accent-soft'),
    };
  }

  onMount(() => {
    const t = new Terminal({
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--mono').trim() || 'monospace',
      fontSize: 12.5,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 5000,
      theme: theme(),
    });
    fit = new FitAddon();
    t.loadAddon(fit);
    t.open(el!);
    t.onData((data) => {
      if (termId) fleet.call('terminal.input', { id: termId, data }, host).catch(() => {});
    });
    t.onResize(({ cols, rows }) => {
      if (termId) fleet.call('terminal.resize', { id: termId, cols, rows }, host).catch(() => {});
    });
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        // A hidden pane (other tab, phone layout) has no size: keep the last one.
        if (el && el.clientWidth > 0 && el.clientHeight > 0) fit?.fit();
      });
    });
    ro.observe(el!);
    if (el!.clientWidth > 0) fit.fit();
    term = t;
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
      t.dispose();
    };
  });

  // Follow the app theme (the CSS variables change after `data-theme` does).
  $effect(() => {
    void ui.theme;
    const t = term;
    if (t) requestAnimationFrame(() => (t.options.theme = theme()));
  });

  // Attach while connected and the host is online; re-attach (replaying the scrollback) after a
  // reconnect, since output sent while the link was down is lost.
  $effect(() => {
    const t = term;
    const client = fleet.client;
    if (!t || !client || !online) return;
    void generation;
    const [h, dir, want] = [host, cwd, selected];
    let id: string | null = null;
    let cancelled = false;
    // Output that arrives before the attach reply (with the scrollback) is applied after it.
    let early: string[] | null = [];
    const offOutput = client.on('terminal.output', (d, from) => {
      if (from !== h || d.id !== id) return;
      if (early) early.push(d.data);
      else t.write(d.data);
    });
    const offExit = client.on('terminal.exit', (d, from) => {
      if (from !== h || d.id !== id) return;
      exited = d.code ?? null;
      termId = null;
      id = null;
    });
    (async () => {
      try {
        let info: TerminalInfo | undefined;
        if (want) {
          info = untrack(() => fleet.terminals[key(h, want)]);
          if (!info) return;
        } else {
          const fresh = forceNew;
          forceNew = false;
          // Newest shell for this directory (the list is oldest first); action runs are not shells.
          const shells = fresh ? [] : (await fleet.call('terminal.list', undefined, h)).filter((x) => x.cwd === dir && !x.action);
          info = shells.pop() ?? (await fleet.call('terminal.open', { cwd: dir, cols: t.cols, rows: t.rows }, h));
        }
        if (cancelled) return;
        id = info.id;
        const { data } = await fleet.call('terminal.attach', { id: info.id, cols: t.cols, rows: t.rows }, h);
        if (cancelled) return;
        t.reset();
        t.write(data);
        for (const chunk of early ?? []) t.write(chunk);
        early = null;
        if (!want) shell = info.shell.split('/').pop() ?? info.shell;
        const ended = untrack(() => (want ? fleet.terminals[key(h, want)]?.exitedAt : undefined));
        termId = ended ? null : info.id;
        exited = ended ? (untrack(() => fleet.terminals[key(h, want!)]?.exitCode) ?? null) : undefined;
        error = '';
        if (matchMedia('(pointer: fine)').matches) t.focus();
      } catch (err) {
        if (!cancelled) error = err instanceof Error ? err.message : String(err);
      }
    })();
    return () => {
      cancelled = true;
      offOutput();
      offExit();
      if (id) fleet.call('terminal.detach', { id }, h).catch(() => {});
      termId = null;
      exited = undefined;
    };
  });

  /** Replace the shell: one per directory, so the running one is stopped. */
  function newShell() {
    const old = selected ? null : termId;
    selected = null;
    forceNew = true;
    exited = undefined;
    generation++;
    if (old) fleet.call('terminal.kill', { id: old }, host).catch(() => {});
  }

  function kill() {
    if (termId) fleet.call('terminal.kill', { id: termId }, host).catch((err) => ui.error('Could not stop the shell', err));
  }

  function close(r: TerminalInfo) {
    selected = null;
    closeRun(host, r);
  }
</script>

<div class="term">
  <div class="bar">
    {#if runs.length}
      <div class="chips" role="tablist" aria-label="Terminals">
        <button role="tab" class="chip" class:on={!selected} aria-selected={!selected} title="Shell in {cwd}" onclick={() => (selected = null)}>{shell || 'shell'}</button>
        {#each runs as r (r.id)}
          <button
            role="tab"
            class="chip"
            class:on={selected === r.id}
            aria-selected={selected === r.id}
            title={r.exitedAt ? (r.exitCode == null ? 'Stopped' : `Exited with code ${r.exitCode}`) : 'Running'}
            onclick={() => (selected = r.id)}
          >
            {#if !r.exitedAt}<span class="dot"></span>{:else if r.exitCode === 0}<span class="ok">✓</span>{:else if r.exitCode != null}<span class="bad">✗</span>{/if}
            <span class="ellipsis">{r.action?.name}</span>
          </button>
        {/each}
      </div>
    {:else}
      <span class="mono path ellipsis" title={cwd}>{shortPath(cwd, 3)}</span>
      {#if shell}<span class="faint">{shell}</span>{/if}
    {/if}
    <span class="grow"></span>
    {#if run?.action}
      {@const action = run.action}
      <button class="btn ghost icon sm" title="Restart {action.name}" aria-label="Restart" onclick={() => restartRun(host, action)}><RotateCw size={13} /></button>
      {#if run.exitedAt}
        <button class="btn ghost icon sm" title="Close this run" aria-label="Close" onclick={() => close(run)}><X size={14} /></button>
      {:else}
        <button class="btn ghost icon sm" title="Stop {action.name}" aria-label="Stop" onclick={() => stopRun(host, run)}><Square size={12} /></button>
      {/if}
    {:else}
      <button class="btn ghost icon sm" title="New shell" aria-label="New shell" onclick={newShell}><Plus size={14} /></button>
      <button class="btn ghost icon sm" title="Stop the shell" aria-label="Stop the shell" disabled={!termId} onclick={kill}><X size={14} /></button>
    {/if}
  </div>
  <div class="screen" data-capture-keys>
    <div class="xterm-host" bind:this={el}></div>
    {#if exited !== undefined && run?.action}
      {@const action = run.action}
      <div class="overlay">
        <span class:bad={exited != null && exited !== 0}>{exited == null ? 'Stopped' : exited === 0 ? 'Done' : `Exited with code ${exited}`}</span>
        <span class="acts">
          <button class="btn sm" onclick={() => close(run)}><X size={13} /> Close</button>
          <button class="btn sm" onclick={() => restartRun(host, action)}><RotateCw size={13} /> Restart</button>
        </span>
      </div>
    {:else if exited !== undefined && !selected}
      <div class="overlay">
        <span>Shell exited{exited != null ? ` (code ${exited})` : ''}</span>
        <button class="btn sm" onclick={newShell}><Plus size={13} /> New shell</button>
      </div>
    {:else if error}
      <div class="overlay err">{error}</div>
    {:else if !online}
      <div class="overlay faint">Host offline</div>
    {/if}
  </div>
</div>

<style>
  .chips {
    display: flex;
    gap: 4px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .chip {
    display: flex;
    flex: none;
    align-items: center;
    gap: 5px;
    max-width: 180px;
    height: 22px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .chip:hover {
    color: var(--text);
  }
  .chip.on {
    border-color: var(--border-strong);
    background: var(--bg-3);
    color: var(--text);
  }
  .dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
    background: var(--green);
  }
  .ok {
    color: var(--green);
  }
  .bad {
    color: var(--red);
  }
  .acts {
    display: flex;
    gap: 6px;
  }
  .term {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .path {
    min-width: 0;
    color: var(--text-2);
  }
  .grow {
    flex: 1;
  }
  .screen {
    position: relative;
    flex: 1;
    min-height: 0;
    padding: 6px 0 0 8px;
    background: var(--bg-1);
  }
  .xterm-host {
    width: 100%;
    height: 100%;
  }
  .overlay {
    position: absolute;
    inset: auto 12px 12px 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg-2);
    font-size: var(--fs-sm);
  }
  .overlay.err {
    color: var(--red);
  }
</style>
