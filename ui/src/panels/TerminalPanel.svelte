<script lang="ts">
  import { onMount } from 'svelte';
  import { Terminal, type ITheme } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { fleet } from '../lib/fleet.svelte';
  import { shortPath } from '../lib/format';
  import { ui } from '../lib/ui.svelte';
  import { Plus, X } from '../lib/icons';

  // A shell in `cwd` on `host`. The shell lives on the daemon: closing the panel only detaches,
  // and opening it again re-attaches to the same shell (one per directory) and replays its output.
  let { host, cwd }: { host: string; cwd: string } = $props();

  let el = $state<HTMLDivElement>();
  let term = $state<Terminal>();
  let fit: FitAddon | undefined;
  /** Attached terminal id. */
  let termId = $state<string | null>(null);
  let shell = $state('');
  /** Exit code once the shell ended (`null` = unknown); `undefined` while it runs. */
  let exited = $state<number | null | undefined>(undefined);
  let error = $state('');
  /** Bumped to start a fresh shell instead of re-attaching. */
  let generation = $state(0);
  let forceNew = false;

  const online = $derived(fleet.conn === 'open' && (fleet.hosts[host]?.online ?? false));

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
    const [h, dir] = [host, cwd];
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
        const fresh = forceNew;
        forceNew = false;
        // Newest shell for this directory (the list is oldest first).
        const existing = fresh ? undefined : (await fleet.call('terminal.list', undefined, h)).filter((x) => x.cwd === dir).pop();
        const info = existing ?? (await fleet.call('terminal.open', { cwd: dir, cols: t.cols, rows: t.rows }, h));
        if (cancelled) return;
        id = info.id;
        const { data } = await fleet.call('terminal.attach', { id: info.id, cols: t.cols, rows: t.rows }, h);
        if (cancelled) return;
        t.reset();
        t.write(data);
        for (const chunk of early ?? []) t.write(chunk);
        early = null;
        shell = info.shell.split('/').pop() ?? info.shell;
        termId = info.id;
        exited = undefined;
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
    };
  });

  /** Replace the shell: one per directory, so the running one is stopped. */
  function newShell() {
    const old = termId;
    forceNew = true;
    exited = undefined;
    generation++;
    if (old) fleet.call('terminal.kill', { id: old }, host).catch(() => {});
  }

  function kill() {
    if (termId) fleet.call('terminal.kill', { id: termId }, host).catch((err) => ui.error('Could not stop the shell', err));
  }
</script>

<div class="term">
  <div class="bar">
    <span class="mono path ellipsis" title={cwd}>{shortPath(cwd, 3)}</span>
    {#if shell}<span class="faint">{shell}</span>{/if}
    <span class="grow"></span>
    <button class="btn ghost icon sm" title="New shell" aria-label="New shell" onclick={newShell}><Plus size={14} /></button>
    <button class="btn ghost icon sm" title="Stop the shell" aria-label="Stop the shell" disabled={!termId} onclick={kill}><X size={14} /></button>
  </div>
  <div class="screen" data-capture-keys>
    <div class="xterm-host" bind:this={el}></div>
    {#if exited !== undefined}
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
