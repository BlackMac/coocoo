<script lang="ts" module>
  import { Terminal, type ITheme } from '@xterm/xterm';

  /** Keys phone keyboards lack, laid out like Termux's extra keys (two rows of seven). */
  const KEYS: { id: string; label: string; aria?: string; repeat?: boolean }[] = [
    { id: 'esc', label: 'Esc', aria: 'Escape' },
    { id: '|', label: '|', aria: 'Pipe' },
    { id: '~', label: '~', aria: 'Tilde' },
    { id: 'home', label: 'Home' },
    { id: 'up', label: '↑', aria: 'Up', repeat: true },
    { id: 'end', label: 'End' },
    { id: 'pgup', label: 'PgUp', aria: 'Page up', repeat: true },
    { id: 'tab', label: 'Tab' },
    { id: 'ctrl', label: 'Ctrl' },
    { id: 'alt', label: 'Alt' },
    { id: 'left', label: '←', aria: 'Left', repeat: true },
    { id: 'down', label: '↓', aria: 'Down', repeat: true },
    { id: 'right', label: '→', aria: 'Right', repeat: true },
    { id: 'pgdn', label: 'PgDn', aria: 'Page down', repeat: true },
  ];
  /** Final byte of the cursor keys (`ESC [ x`, or `ESC O x` in application cursor mode). */
  const CURSOR: Record<string, string> = { up: 'A', down: 'B', right: 'C', left: 'D', home: 'H', end: 'F' };
  /** Parameter of the `ESC [ n ~` keys. */
  const TILDE: Record<string, number> = { pgup: 5, pgdn: 6 };
  const FONT_MIN = 8;
  const FONT_MAX = 24;

  /** The control character Ctrl+`ch` sends, if there is one. */
  function ctrlChar(ch: string): string | undefined {
    if (ch === ' ') return '\x00';
    if (ch === '?') return '\x7f';
    if (ch === '/') return '\x1f';
    const c = (/[a-z]/.test(ch) ? ch.toUpperCase() : ch).charCodeAt(0);
    return c >= 64 && c <= 95 ? String.fromCharCode(c - 64) : undefined;
  }

  /** The whole buffer (scrollback and screen) as text, with soft-wrapped lines joined. */
  function bufferText(t: Terminal): string {
    const buf = t.buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buf.length; i++) {
      const line = buf.getLine(i);
      if (!line) continue;
      const text = line.translateToString();
      if (line.isWrapped && lines.length) lines[lines.length - 1] += text;
      else lines.push(text);
    }
    return lines
      .map((l) => l.trimEnd())
      .join('\n')
      .replace(/\n+$/, '');
  }
</script>

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import '@xterm/xterm/css/xterm.css';
  import { fleet, key } from '../lib/fleet.svelte';
  import { shortPath } from '../lib/format';
  import { ui } from '../lib/ui.svelte';
  import { closeRun, isLoopbackUrl, openInBrowserPanel, restartRun, stopRun } from '../lib/actions';
  import { copyText } from '../lib/clipboard';
  import { openExternal } from '../lib/external';
  import { ClipboardPaste, Copy, Keyboard, KeyboardOff, Plus, RotateCw, Square, X } from '../lib/icons';
  import type { TerminalInfo } from '../lib/protocol';

  // The terminals of `cwd` on `host`: its shell, and the runs of project actions started for this
  // directory (chips switch between them). Both live on the daemon: closing the panel only
  // detaches, and opening it again re-attaches (one shell per directory) and replays the output.
  // `browserProfile`: where links to the host's own ports (localhost) open; without it, and for
  // every other link, they open outside.
  let { host, cwd, browserProfile }: { host: string; cwd: string; browserProfile?: string } = $props();

  let el = $state<HTMLDivElement>();
  let screenEl = $state<HTMLDivElement>();
  let keysEl = $state<HTMLDivElement>();
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

  /** The terminal has the keyboard focus (on a phone: the on-screen keyboard is up). */
  let focused = $state(false);
  /** One-shot modifiers from the extra keys, applied to the next key. */
  let ctrl = $state(false);
  let alt = $state(false);
  /** Extra key held down (shown pressed; arrows repeat while held). */
  let held = $state<string | null>(null);
  let repeatTimer: ReturnType<typeof setTimeout> | undefined;
  /** Buffer text in the select sheet; `null` = closed. */
  let selectText = $state<string | null>(null);
  let selectEl = $state<HTMLPreElement>();
  /** Text selected in the select sheet ('' = none: the button copies everything). */
  let picked = $state('');
  /** The paste sheet, for when the clipboard cannot be read directly. */
  let pasting = $state(false);
  let pasteText = $state('');
  let pasteEl = $state<HTMLTextAreaElement>();

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

  /** Links in the output: the host's own ports in the agent's browser panel, the rest outside. */
  function openLink(uri: string) {
    if (browserProfile && isLoopbackUrl(uri)) openInBrowserPanel(host, browserProfile, uri).catch((err) => ui.error('Could not open the page', err));
    else openExternal(uri);
  }

  function send(data: string) {
    if (termId) fleet.call('terminal.input', { id: termId, data }, host).catch(() => {});
  }

  /** `data` with the armed modifiers applied (Ctrl to its first character, Alt as an ESC prefix). */
  function withMods(data: string): string {
    let out = data;
    if (ctrl) {
      const c = ctrlChar(data[0]);
      if (c) out = c + data.slice(1);
    }
    return alt ? `\x1b${out}` : out;
  }

  /** Typed input: armed modifiers apply to it and are used up. Escape sequences (hardware keys,
   *  terminal replies) carry their own modifiers and pass unchanged. */
  function applyMods(data: string): string {
    if ((!ctrl && !alt) || data.startsWith('\x1b')) return data;
    const out = withMods(data);
    ctrl = alt = false;
    return out;
  }

  /** What an extra key sends, with the armed modifiers (xterm's encoding). */
  function keySeq(id: string): string {
    const mod = 1 + (alt ? 2 : 0) + (ctrl ? 4 : 0);
    const final = CURSOR[id];
    if (final) return mod > 1 ? `\x1b[1;${mod}${final}` : term?.modes.applicationCursorKeysMode ? `\x1bO${final}` : `\x1b[${final}`;
    const n = TILDE[id];
    if (n) return mod > 1 ? `\x1b[${n};${mod}~` : `\x1b[${n}~`;
    return withMods(id === 'esc' ? '\x1b' : id === 'tab' ? '\t' : id);
  }

  function stopRepeat() {
    clearTimeout(repeatTimer);
    clearInterval(repeatTimer);
    held = null;
  }

  // Keys act on press (and repeat while held); `preventDefault` keeps the focus, and so the
  // on-screen keyboard, where it is.
  function press(e: PointerEvent, k: (typeof KEYS)[number]) {
    e.preventDefault();
    stopRepeat();
    if (k.id === 'ctrl') ctrl = !ctrl;
    else if (k.id === 'alt') alt = !alt;
    else {
      const seq = keySeq(k.id);
      ctrl = alt = false;
      // As user input: scrolls to the bottom like typing does.
      const fire = () => term?.input(seq, true);
      fire();
      held = k.id;
      if (k.repeat) repeatTimer = setTimeout(() => (repeatTimer = setInterval(fire, 60)), 400);
    }
  }

  // On release, not on press: browsers only raise the on-screen keyboard for a focus() during a
  // user activation, which a touch's pointerup is and its pointerdown is not.
  function toggleKeyboard() {
    if (focused) term?.blur();
    else term?.focus();
  }

  /** Copies xterm's selection (mouse); without one, opens the buffer as selectable text. */
  function copy() {
    const t = term;
    if (!t) return;
    if (t.hasSelection()) {
      copyText(t.getSelection()).then(() => ui.toast('success', 'Copied'), (err) => ui.error('Copy failed', err));
      return;
    }
    picked = '';
    selectText = bufferText(t);
  }

  /** The sheet's selection; also read on the button's pointerdown, before the tap can clear it. */
  function pickSelection() {
    const s = document.getSelection();
    picked = s && !s.isCollapsed && s.anchorNode && selectEl?.contains(s.anchorNode) ? s.toString() : '';
  }

  function copyFromSheet() {
    const text = picked || selectText || '';
    copyText(text).then(
      () => {
        ui.toast('success', picked ? 'Copied' : 'Copied everything');
        selectText = null;
      },
      (err) => ui.error('Copy failed', err),
    );
  }

  // The sheet opens at the end, like the terminal; the copy button follows the selection.
  $effect(() => {
    const pre = selectEl;
    if (!pre) return;
    pre.scrollTop = pre.scrollHeight;
    document.addEventListener('selectionchange', pickSelection);
    return () => document.removeEventListener('selectionchange', pickSelection);
  });

  function insert(text: string) {
    ctrl = alt = false;
    // paste(): newlines become Enter, and bracketed paste mode is honoured.
    term?.paste(text);
  }

  /** Pastes the clipboard; where it cannot be read (plain http, denied), opens the paste sheet. */
  async function paste() {
    if (window.isSecureContext && navigator.clipboard?.readText) {
      try {
        const text = await navigator.clipboard.readText();
        if (text) insert(text);
        else ui.toast('info', 'The clipboard is empty');
        return;
      } catch {
        /* not allowed: paste by hand */
      }
    }
    pasteText = '';
    pasting = true;
  }

  function sendPaste() {
    if (pasteText) insert(pasteText);
    pasting = false;
    // Back to typing in the terminal (the keyboard stays up on a phone).
    term?.focus();
  }

  $effect(() => {
    if (pasting) pasteEl?.focus();
  });

  onMount(() => {
    const t = new Terminal({
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--mono').trim() || 'monospace',
      fontSize: ui.termFontSize,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 5000,
      theme: theme(),
      // OSC 8 hyperlinks (only http(s): xterm drops the rest).
      linkHandler: { activate: (_e, uri) => openLink(uri) },
    });
    fit = new FitAddon();
    t.loadAddon(fit);
    t.loadAddon(new WebLinksAddon((_e, uri) => openLink(uri)));
    t.open(el!);
    t.onData((data) => send(applyMods(data)));
    // The pty follows once the size settles: a pinch or the on-screen keyboard sliding in resize
    // the grid many times in a row, and every resize makes the shell redraw its prompt.
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    t.onResize(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        if (termId) fleet.call('terminal.resize', { id: termId, cols: t.cols, rows: t.rows }, host).catch(() => {});
      }, 150);
    });
    const ta = t.textarea!;
    const onFocus = () => (focused = true);
    const onBlur = () => (focused = false);
    ta.addEventListener('focus', onFocus);
    ta.addEventListener('blur', onBlur);

    // Extra keys: a touch on them must not move the focus (that would drop the on-screen
    // keyboard), scroll, zoom or open the callout. Svelte's touch handlers are passive.
    const keys = keysEl!;
    const noTouchDefault = (e: TouchEvent) => e.preventDefault();
    keys.addEventListener('touchstart', noTouchDefault, { passive: false });

    // Two-finger pinch on the terminal sets the font size (instead of zooming the page or
    // scrolling the output, so it is caught before xterm's own touch handling).
    const screen = screenEl!;
    let pinch: { dist: number; size: number } | null = null;
    const spread = (e: TouchEvent) => Math.hypot(e.touches[0].clientX - e.touches[1].clientX, e.touches[0].clientY - e.touches[1].clientY);
    const onTouchStart = (e: TouchEvent) => {
      if (e.touches.length === 2) pinch = { dist: spread(e), size: ui.termFontSize };
    };
    const onTouchMove = (e: TouchEvent) => {
      if (!pinch || e.touches.length !== 2) return;
      e.preventDefault();
      e.stopPropagation();
      const size = Math.min(FONT_MAX, Math.max(FONT_MIN, (pinch.size * spread(e)) / pinch.dist));
      ui.termFontSize = Math.round(size * 2) / 2;
    };
    const onTouchEnd = (e: TouchEvent) => {
      if (e.touches.length < 2) pinch = null;
    };
    // Safari's own pinch gesture.
    const noGesture = (e: Event) => e.preventDefault();
    screen.addEventListener('touchstart', onTouchStart, { capture: true, passive: true });
    screen.addEventListener('touchmove', onTouchMove, { capture: true, passive: false });
    screen.addEventListener('touchend', onTouchEnd, { capture: true, passive: true });
    screen.addEventListener('touchcancel', onTouchEnd, { capture: true, passive: true });
    screen.addEventListener('gesturestart', noGesture);

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
      stopRepeat();
      clearTimeout(resizeTimer);
      ta.removeEventListener('focus', onFocus);
      ta.removeEventListener('blur', onBlur);
      keys.removeEventListener('touchstart', noTouchDefault);
      screen.removeEventListener('touchstart', onTouchStart, { capture: true });
      screen.removeEventListener('touchmove', onTouchMove, { capture: true });
      screen.removeEventListener('touchend', onTouchEnd, { capture: true });
      screen.removeEventListener('touchcancel', onTouchEnd, { capture: true });
      screen.removeEventListener('gesturestart', noGesture);
      t.dispose();
    };
  });

  // Font size (pinch, or another terminal panel's pinch): the grid changes, the box does not.
  $effect(() => {
    const size = ui.termFontSize;
    const t = term;
    if (!t || t.options.fontSize === size) return;
    t.options.fontSize = size;
    if (el && el.clientWidth > 0 && el.clientHeight > 0) fit?.fit();
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
    <!-- pointerdown keeps the focus (and a phone's on-screen keyboard) in the terminal. -->
    <button class="btn ghost icon sm" title="Copy" aria-label="Copy" disabled={!term} onpointerdown={(e) => e.preventDefault()} onclick={copy}><Copy size={13} /></button>
    <button class="btn ghost icon sm" title="Paste" aria-label="Paste" disabled={!termId} onpointerdown={(e) => e.preventDefault()} onclick={paste}><ClipboardPaste size={13} /></button>
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
  <div class="screen" data-capture-keys bind:this={screenEl}>
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
    {#if selectText !== null}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div class="sheet" role="dialog" aria-label="Copy text" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && (selectText = null)}>
        <div class="sheet-bar">
          <span class="faint ellipsis">{picked ? 'Copy the selected text' : 'Select text, or copy all of it'}</span>
          <span class="grow"></span>
          <button
            class="btn sm primary"
            onpointerdown={(e) => {
              e.preventDefault();
              pickSelection();
            }}
            onclick={copyFromSheet}><Copy size={13} /> {picked ? 'Copy' : 'Copy all'}</button
          >
          <button class="btn ghost icon sm" title="Close" aria-label="Close" onclick={() => (selectText = null)}><X size={14} /></button>
        </div>
        <pre class="seltext" bind:this={selectEl} style:font-size="{ui.termFontSize}px">{selectText}</pre>
      </div>
    {:else if pasting}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div class="sheet" role="dialog" aria-label="Paste text" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && (pasting = false)}>
        <div class="sheet-bar">
          <span class="faint ellipsis">Paste here, then send it to the terminal</span>
          <span class="grow"></span>
          <button class="btn sm primary" disabled={!pasteText} onclick={sendPaste}>Send</button>
          <button class="btn ghost icon sm" title="Cancel" aria-label="Cancel" onclick={() => (pasting = false)}><X size={14} /></button>
        </div>
        <textarea
          class="pastetext"
          bind:this={pasteEl}
          bind:value={pasteText}
          placeholder="Long-press here and choose Paste"
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
          onkeydown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              sendPaste();
            }
          }}
        ></textarea>
      </div>
    {/if}
  </div>
  <!-- Touch screens only (see the styles): keys phone keyboards lack, and the keyboard toggle. -->
  <div class="keys" class:off={selectText !== null || pasting} bind:this={keysEl} role="toolbar" aria-label="Extra keys">
    {#each KEYS as k (k.id)}
      <button
        type="button"
        tabindex="-1"
        class="key"
        class:on={(k.id === 'ctrl' && ctrl) || (k.id === 'alt' && alt)}
        class:held={held === k.id}
        aria-label={k.aria}
        aria-pressed={k.id === 'ctrl' ? ctrl : k.id === 'alt' ? alt : undefined}
        disabled={!termId}
        onpointerdown={(e) => press(e, k)}
        onpointerup={stopRepeat}
        onpointercancel={stopRepeat}
        onpointerleave={stopRepeat}>{k.label}</button
      >
    {/each}
    <button
      type="button"
      tabindex="-1"
      class="key toggle"
      aria-label={focused ? 'Hide the keyboard' : 'Show the keyboard'}
      onpointerdown={(e) => e.preventDefault()}
      onpointerup={toggleKeyboard}
    >
      {#if focused}<KeyboardOff size={16} />{:else}<Keyboard size={16} />{/if}
    </button>
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
  .sheet {
    position: absolute;
    inset: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    background: var(--bg-1);
  }
  .sheet-bar {
    display: flex;
    flex: none;
    align-items: center;
    gap: 8px;
    min-height: 38px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .seltext {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 8px 10px;
    overflow: auto;
    font-family: var(--mono);
    line-height: 1.15;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .pastetext {
    flex: 1;
    min-height: 0;
    padding: 10px 12px;
    border: 0;
    outline: none;
    resize: none;
    background: transparent;
    font-family: var(--mono);
    font-size: 16px;
  }
  .keys {
    display: none;
  }
  @media (pointer: coarse) {
    .keys {
      display: grid;
      flex: none;
      grid-template-columns: repeat(7, minmax(0, 1fr)) 46px;
      grid-template-rows: 34px 34px;
      gap: 4px;
      padding: 5px 6px;
      border-top: 1px solid var(--border);
      background: var(--bg-2);
      touch-action: none;
      user-select: none;
      -webkit-user-select: none;
      -webkit-touch-callout: none;
    }
    .keys.off {
      display: none;
    }
  }
  .key {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: var(--bg-3);
    color: var(--text);
    font-family: var(--mono);
    font-size: 12px;
    -webkit-tap-highlight-color: transparent;
  }
  .key:disabled {
    color: var(--text-3);
  }
  .key.held {
    background: var(--border-strong);
  }
  .key.on {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .toggle {
    grid-row: 1 / span 2;
    grid-column: 8;
    color: var(--text-2);
  }
</style>
