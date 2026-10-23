<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary, BrowserConsoleEntry, BrowserFrame, BrowserInputEvent, Project } from '../lib/protocol';
  import { ArrowLeft, ArrowRight, Camera, Globe, MousePointerClick, Plus, RotateCw, Terminal, X } from '../lib/icons';

  let { agent: a, project }: { agent: AgentSummary; project?: Project } = $props();

  const host = $derived(a.hostId);
  const profileId = $derived(a.projectId ?? 'default');
  const profile = $derived(fleet.profiles[key(host, profileId)]);
  const tabs = $derived(profile?.running ? profile.tabs : []);

  let selected = $state<string | null>(null);
  const tab = $derived(tabs.find((t) => t.targetId === selected) ?? tabs.find((t) => t.active) ?? tabs[0]);
  const target = $derived(tab?.targetId);

  // A tab a URL action just opened is shown at once.
  $effect(() => {
    const f = ui.browserFocus;
    if (f && f.host === host && f.profile === profileId) {
      selected = f.targetId;
      ui.browserFocus = null;
    }
  });

  let url = $state('');
  let urlFocused = $state(false);
  let design = $state(false);
  let consoleOpen = $state(false);
  let logs = $state<BrowserConsoleEntry[]>([]);
  let frameSize = $state<{ w: number; h: number } | null>(null);
  let hasFrame = $state(false);
  let visible = $state(document.visibilityState === 'visible');
  let opening = $state(false);

  let viewer = $state<HTMLDivElement>();
  let img = $state<HTMLImageElement>();

  $effect(() => {
    if (!urlFocused) url = tab?.url ?? '';
  });

  // Refresh the profile list once if this host never reported the profile.
  $effect(() => {
    if (!profile) {
      fleet
        .call('browser.profiles', undefined, host)
        .then((list) => {
          for (const p of list) fleet.profiles[key(host, p.id)] = p;
        })
        .catch(() => {});
    }
  });

  const onVis = () => (visible = document.visibilityState === 'visible');
  document.addEventListener('visibilitychange', onVis);
  onDestroy(() => document.removeEventListener('visibilitychange', onVis));

  // ─── Frames: watch only while this panel shows the tab and the page is visible ──────────────
  let pending: BrowserFrame | null = null;
  let raf = 0;

  function draw() {
    raf = 0;
    const f = pending;
    pending = null;
    if (!f || !img) return;
    img.src = `data:image/jpeg;base64,${f.data}`;
    if (frameSize?.w !== f.width || frameSize?.h !== f.height) frameSize = { w: f.width, h: f.height };
    hasFrame = true;
  }

  $effect(() => {
    const t = target;
    const p = profileId;
    const h = host;
    const client = fleet.client;
    if (!t || !visible || !client || !viewer) return;
    hasFrame = false;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const r = viewer.getBoundingClientRect();
    const offFrame = client.on('browser.frame', (f, from) => {
      if (from !== h || f.profile !== p || f.targetId !== t) return;
      pending = f;
      if (!raf) raf = requestAnimationFrame(draw);
    });
    const offConsole = client.on('browser.console', (e, from) => {
      if (from !== h || e.profile !== p || e.targetId !== t) return;
      logs.push(e);
      if (logs.length > 500) logs.splice(0, logs.length - 500);
    });
    const w = Math.max(320, Math.floor(r.width));
    const hh = Math.max(240, Math.floor(r.height));
    fleet.call('browser.resize', { profile: p, targetId: t, width: w, height: hh }, h).catch(() => {});
    fleet
      .call('browser.watch', { profile: p, targetId: t, maxWidth: Math.round(w * dpr), maxHeight: Math.round(hh * dpr), quality: 70 }, h)
      .catch((err) => ui.error('Cannot stream the browser', err));
    return () => {
      offFrame();
      offConsole();
      cancelAnimationFrame(raf);
      raf = 0;
      pending = null;
      fleet.call('browser.unwatch', { profile: p, targetId: t }, h).catch(() => {});
    };
  });

  // Fit the page viewport to the pane (debounced).
  $effect(() => {
    if (!viewer) return;
    let timer: number | undefined;
    let first = true;
    const ro = new ResizeObserver(([entry]) => {
      if (first) return void (first = false);
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (!target) return;
        const { width, height } = entry.contentRect;
        fleet.send('browser.resize', { profile: profileId, targetId: target, width: Math.max(320, Math.floor(width)), height: Math.max(240, Math.floor(height)) }, host);
      }, 250);
    });
    ro.observe(viewer);
    return () => {
      ro.disconnect();
      clearTimeout(timer);
    };
  });

  $effect(() => {
    if (!consoleOpen || !target) return;
    fleet
      .call('browser.console', { profile: profileId, targetId: target }, host)
      .then((l) => (logs = l))
      .catch(() => {});
  });

  // ─── Input forwarding ──────────────────────────────────────────────────────────────────────
  /** Maps client coordinates to page CSS pixels (the image is letterboxed with object-fit: contain). */
  function toPage(e: MouseEvent): { x: number; y: number } | null {
    if (!img || !frameSize) return null;
    const r = img.getBoundingClientRect();
    const scale = Math.min(r.width / frameSize.w, r.height / frameSize.h);
    const ox = r.left + (r.width - frameSize.w * scale) / 2;
    const oy = r.top + (r.height - frameSize.h * scale) / 2;
    const x = (e.clientX - ox) / scale;
    const y = (e.clientY - oy) / scale;
    if (x < 0 || y < 0 || x > frameSize.w || y > frameSize.h) return null;
    return { x: Math.round(x), y: Math.round(y) };
  }

  const mods = (e: KeyboardEvent | MouseEvent) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
  const BUTTONS = ['left', 'middle', 'right'] as const;

  function input(event: BrowserInputEvent) {
    if (target) fleet.send('browser.input', { profile: profileId, targetId: target, event }, host);
  }

  // Mouse moves (and touch scrolls) are coalesced to one message per animation frame.
  let moveQueued: MouseEvent | null = null;
  let moveRaf = 0;
  let lastHighlight = 0;
  /** Active touch gesture: dragging scrolls the page, a short tap clicks. */
  let touch: { lx: number; ly: number; dx: number; dy: number; moved: boolean } | null = null;

  function flushMove() {
    moveRaf = 0;
    const ev = moveQueued;
    moveQueued = null;
    if (!ev || !target) return;
    const p = toPage(ev);
    if (!p) return;
    if (touch) {
      const scale = frameSize && img ? frameSize.w / img.getBoundingClientRect().width : 1;
      input({ type: 'mouse', action: 'wheel', ...p, deltaX: touch.dx * scale, deltaY: touch.dy * scale });
      touch.dx = touch.dy = 0;
    } else if (design) {
      const now = performance.now();
      if (now - lastHighlight < 80) return;
      lastHighlight = now;
      fleet.send('browser.highlight', { profile: profileId, targetId: target, ...p }, host);
    } else {
      input({ type: 'mouse', action: 'move', ...p, button: ev.buttons & 1 ? 'left' : 'none', modifiers: mods(ev) });
    }
  }

  function onpointermove(e: PointerEvent) {
    if (touch && e.pointerType === 'touch') {
      touch.dx += touch.lx - e.clientX;
      touch.dy += touch.ly - e.clientY;
      touch.lx = e.clientX;
      touch.ly = e.clientY;
      if (Math.abs(touch.dx) + Math.abs(touch.dy) > 6) touch.moved = true;
      if (!touch.moved) return;
    }
    moveQueued = e;
    if (!moveRaf) moveRaf = requestAnimationFrame(flushMove);
  }

  function onpointerdown(e: PointerEvent) {
    viewer?.focus({ preventScroll: true });
    if (e.pointerType === 'touch') {
      touch = { lx: e.clientX, ly: e.clientY, dx: 0, dy: 0, moved: false };
      return;
    }
    const p = toPage(e);
    if (!p || design) return;
    e.preventDefault();
    input({ type: 'mouse', action: 'down', ...p, button: BUTTONS[e.button] ?? 'left', clickCount: e.detail || 1, modifiers: mods(e) });
  }

  function onpointerup(e: PointerEvent) {
    const p = toPage(e);
    if (touch) {
      const tap = !touch.moved;
      touch = null;
      if (tap && p && !design) {
        input({ type: 'mouse', action: 'down', ...p, button: 'left', clickCount: 1 });
        input({ type: 'mouse', action: 'up', ...p, button: 'left', clickCount: 1 });
      }
      return;
    }
    if (!p || design) return;
    input({ type: 'mouse', action: 'up', ...p, button: BUTTONS[e.button] ?? 'left', clickCount: e.detail || 1, modifiers: mods(e) });
  }

  async function onclick(e: MouseEvent) {
    if (!design || !target) return;
    const p = toPage(e);
    if (!p) return;
    try {
      const r = await fleet.call('browser.inspect', { profile: profileId, targetId: target, ...p }, host);
      ui.attachElement(key(a.hostId, a.id), r, tab?.url ?? '');
      ui.toast('success', 'Element attached to prompt', r.selector);
    } catch (err) {
      ui.error('Inspect failed', err);
    }
  }

  function onwheel(e: WheelEvent) {
    const p = toPage(e);
    if (!p) return;
    e.preventDefault();
    const k = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
    input({ type: 'mouse', action: 'wheel', ...p, deltaX: e.deltaX * k, deltaY: e.deltaY * k, modifiers: mods(e) });
  }

  // Non-passive wheel listener so the page scrolls instead of the panel.
  $effect(() => {
    const el = viewer;
    if (!el) return;
    el.addEventListener('wheel', onwheel, { passive: false });
    return () => el.removeEventListener('wheel', onwheel);
  });

  function onkey(e: KeyboardEvent, action: 'down' | 'up') {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'v') return; // handled by paste
    if (e.key === 'Escape' && design) {
      setDesign(false);
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const text = action === 'down' ? (e.key.length === 1 ? e.key : e.key === 'Enter' ? '\r' : undefined) : undefined;
    input({ type: 'key', action, key: e.key, code: e.code, text: e.ctrlKey || e.metaKey ? undefined : text, keyCode: e.keyCode, modifiers: mods(e) });
  }

  function onpaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text/plain');
    if (!text) return;
    e.preventDefault();
    input({ type: 'text', text });
  }

  function setDesign(on: boolean) {
    design = on;
    if (!on && target) fleet.send('browser.highlight', { profile: profileId, targetId: target }, host);
  }

  // ─── Toolbar actions ───────────────────────────────────────────────────────────────────────
  function nav(action: 'back' | 'forward' | 'reload') {
    if (target) fleet.call('browser.navigate', { profile: profileId, targetId: target, action }, host).catch((e) => ui.error('Navigation failed', e));
  }

  function normalize(u: string) {
    const v = u.trim();
    if (/^[a-z][a-z0-9+.-]*:/i.test(v)) return v;
    if (/^(localhost|127\.|\d+\.\d+\.\d+\.\d+|\[)/.test(v) || /^[\w-]+(\.[\w-]+)+(:\d+)?(\/|$)/.test(v)) return `http://${v}`;
    return `https://www.google.com/search?q=${encodeURIComponent(v)}`;
  }

  async function go(e: SubmitEvent) {
    e.preventDefault();
    const u = normalize(url);
    if (!u) return;
    (document.activeElement as HTMLElement | null)?.blur();
    if (target) fleet.call('browser.navigate', { profile: profileId, targetId: target, url: u }, host).catch((err) => ui.error('Navigation failed', err));
    else await open(u);
  }

  async function open(u: string) {
    opening = true;
    try {
      const t = await fleet.call('browser.open', { profile: profileId, url: normalize(u) }, host, 45_000);
      selected = t.targetId;
    } catch (err) {
      ui.error('Could not open the page', err);
    } finally {
      opening = false;
    }
  }

  function closeTab(id: string) {
    fleet.call('browser.closeTab', { profile: profileId, targetId: id }, host).catch((e) => ui.error('Could not close tab', e));
  }

  function activate(id: string) {
    selected = id;
    fleet.send('browser.activate', { profile: profileId, targetId: id }, host);
  }

  async function screenshot() {
    if (!target) return;
    try {
      const { data } = await fleet.call('browser.screenshot', { profile: profileId, targetId: target }, host);
      ui.attachImage(key(a.hostId, a.id), `screenshot ${new Date().toLocaleTimeString()}.png`, { mimeType: 'image/png', data });
      ui.toast('success', 'Screenshot attached to prompt');
    } catch (err) {
      ui.error('Screenshot failed', err);
    }
  }

  const devUrls = $derived(project?.devUrls ?? []);
</script>

<div class="browser">
  {#if tabs.length}
    <div class="tabstrip" role="tablist">
      {#each tabs as t (t.targetId)}
        <div class="tab" class:on={t.targetId === target} role="tab" aria-selected={t.targetId === target}>
          <button class="tl ellipsis" title={t.url} onclick={() => activate(t.targetId)}>{t.title || t.url || 'New tab'}</button>
          <button class="tx" aria-label="Close tab" onclick={() => closeTab(t.targetId)}><X size={11} /></button>
        </div>
      {/each}
      <button class="btn ghost icon sm" title="New tab" aria-label="New tab" onclick={() => open(devUrls[0] ?? 'about:blank')}><Plus size={13} /></button>
    </div>
  {/if}
  <div class="toolbar">
    <button class="btn ghost icon sm" aria-label="Back" disabled={!target} onclick={() => nav('back')}><ArrowLeft size={14} /></button>
    <button class="btn ghost icon sm" aria-label="Forward" disabled={!target} onclick={() => nav('forward')}><ArrowRight size={14} /></button>
    <button class="btn ghost icon sm" aria-label="Reload" disabled={!target} onclick={() => nav('reload')}><RotateCw size={13} /></button>
    <form class="urlf" onsubmit={go}>
      <input
        class="input mono url"
        bind:value={url}
        placeholder="Enter a URL"
        spellcheck="false"
        autocomplete="off"
        onfocus={(e) => {
          urlFocused = true;
          e.currentTarget.select();
        }}
        onblur={() => (urlFocused = false)}
      />
    </form>
    <button class="btn ghost icon sm" class:active={design} aria-pressed={design} disabled={!target} title="Design mode: pick an element to attach to the prompt" aria-label="Design mode" onclick={() => setDesign(!design)}>
      <MousePointerClick size={14} />
    </button>
    <button class="btn ghost icon sm" disabled={!target} title="Attach screenshot to prompt" aria-label="Screenshot" onclick={screenshot}><Camera size={14} /></button>
    <button class="btn ghost icon sm" class:active={consoleOpen} disabled={!target} title="Console" aria-label="Console" onclick={() => (consoleOpen = !consoleOpen)}><Terminal size={14} /></button>
  </div>

  {#if target}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="viewer"
      class:design
      bind:this={viewer}
      tabindex="0"
      role="application"
      aria-label="Remote browser — click to interact, keys are forwarded"
      data-capture-keys
      {onpointermove}
      {onpointerdown}
      {onpointerup}
      {onclick}
      oncontextmenu={(e) => e.preventDefault()}
      onkeydown={(e) => onkey(e, 'down')}
      onkeyup={(e) => onkey(e, 'up')}
      {onpaste}
    >
      <img bind:this={img} alt="Live view of {tab?.title ?? 'the page'}" draggable="false" class:ready={hasFrame} />
      {#if !hasFrame}<div class="wait"><span class="spinner"></span></div>{/if}
      {#if design}<div class="dm-badge"><MousePointerClick size={12} /> Design mode — click an element · Esc to exit</div>{/if}
    </div>
  {:else}
    <div class="start">
      <Globe size={28} />
      <strong>{profile?.running ? 'No open tabs' : 'Browser not running'}</strong>
      <span class="faint">
        Profile <span class="mono">{profileId}</span> on {fleet.host(host)?.name ?? host}. The agent's browser tool drives this same Chromium — open a page to watch or take over.
      </span>
      <div class="quick">
        {#each devUrls as u (u)}
          <button class="btn sm" disabled={opening} onclick={() => open(u)}><Globe size={13} />{u.replace(/^https?:\/\//, '')}</button>
        {/each}
        <button class="btn sm" disabled={opening} onclick={() => open('about:blank')}>{#if opening}<span class="spinner"></span>{:else}<Plus size={13} />{/if}Blank tab</button>
      </div>
    </div>
  {/if}

  {#if consoleOpen && target}
    <div class="console mono">
      <div class="chead"><span>Console</span><span class="faint">{logs.length}</span><button class="btn ghost sm" onclick={() => (logs = [])}>Clear</button><button class="btn ghost icon sm" aria-label="Close console" onclick={() => (consoleOpen = false)}><X size={12} /></button></div>
      <div class="lines">
        {#each logs as l, i (i)}
          <div class="ll" data-l={l.level}><span class="lv">{l.level}</span><span class="lt">{l.text}</span></div>
        {:else}
          <div class="ll faint">No console output</div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .browser {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tabstrip {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 34px;
    padding: 0 6px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex: none;
  }
  .tab {
    display: flex;
    align-items: center;
    max-width: 200px;
    min-width: 80px;
    height: 26px;
    border-radius: var(--r);
    color: var(--text-2);
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.on {
    background: var(--bg-active);
    color: var(--text);
  }
  .tl {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 4px 0 10px;
    border: 0;
    background: transparent;
    color: inherit;
    font-size: var(--fs-xs);
    text-align: left;
    cursor: pointer;
  }
  .tx {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-right: 3px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
    opacity: 0;
  }
  .tab:hover .tx,
  .tab.on .tx {
    opacity: 1;
  }
  .tx:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 38px;
    padding: 0 6px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .urlf {
    flex: 1;
    min-width: 0;
    margin: 0 4px;
  }
  .url {
    height: 26px;
    border-radius: 99px;
    padding: 0 12px;
    background: var(--bg);
    border-color: var(--border);
    font-size: 11.5px;
  }
  .viewer {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--bg-3);
    overflow: hidden;
    outline: none;
    cursor: default;
    touch-action: none;
  }
  .viewer:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .viewer.design {
    cursor: crosshair;
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
    -webkit-user-drag: none;
    opacity: 0;
  }
  img.ready {
    opacity: 1;
  }
  .wait {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
  }
  .dm-badge {
    position: absolute;
    left: 50%;
    top: 10px;
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 99px;
    background: var(--accent);
    color: var(--accent-fg);
    font-size: var(--fs-xs);
    font-weight: 500;
    pointer-events: none;
    white-space: nowrap;
  }
  .start {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 24px;
    color: var(--text-3);
    text-align: center;
  }
  .start strong {
    color: var(--text);
    font-weight: 500;
  }
  .start > span {
    max-width: 360px;
    line-height: 1.55;
    font-size: var(--fs-sm);
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin-top: 6px;
  }
  .console {
    height: 34%;
    min-height: 120px;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    background: var(--bg-1);
    flex: none;
    font-size: 11.5px;
  }
  .chead {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--border);
    font-family: var(--font);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-2);
  }
  .chead .btn:first-of-type {
    margin-left: auto;
  }
  .lines {
    flex: 1;
    overflow-y: auto;
  }
  .ll {
    display: flex;
    gap: 10px;
    padding: 3px 12px;
    border-bottom: 1px solid var(--border);
  }
  .lv {
    flex: none;
    width: 56px;
    color: var(--text-3);
  }
  .lt {
    flex: 1;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ll[data-l='warning'] {
    background: var(--amber-soft);
  }
  .ll[data-l='warning'] .lv {
    color: var(--amber);
  }
  .ll[data-l='error'],
  .ll[data-l='exception'] {
    background: var(--red-soft);
  }
  .ll[data-l='error'] .lv,
  .ll[data-l='exception'] .lv,
  .ll[data-l='error'] .lt,
  .ll[data-l='exception'] .lt {
    color: var(--red);
  }
</style>
