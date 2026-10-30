<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { fleet } from '../lib/fleet.svelte';
  import { cost, duration } from '../lib/format';
  import { copyText } from '../lib/clipboard';
  import type { Subagent, TimelineImage, TimelineItem } from '../lib/protocol';
  import { ui } from '../lib/ui.svelte';
  import { ArrowDown, ChevronLeft, ChevronRight, MessageSquareText } from '../lib/icons';
  import ItemList from '../agent/ItemList.svelte';

  // One subagent in the Subagents tab: its state (live from the parent's timeline) and its
  // transcript, read from its omp session file. While it runs, the transcript is read again on
  // every progress report (each tool step, at most once a second) and every few seconds besides,
  // and follows the newest message unless scrolled up. Images its tool calls returned show on
  // those calls and open in the Images tab.
  let {
    host,
    agentId,
    item,
    sub,
    position,
    onback,
    onstep,
    onchat,
  }: {
    host: string;
    agentId: string;
    /** The tool item (`task` call) that started it. */
    item: TimelineItem;
    sub: Subagent;
    /** `[index, count]` among the agent's subagents, for the step buttons. */
    position: [number, number];
    onback: () => void;
    onstep: (delta: number) => void;
    onchat: () => void;
  } = $props();

  const POLL_MS = 3000;

  const live = $derived(sub.status === 'running');
  let items = $state<TimelineItem[]>([]);
  let hasMore = $state(false);
  let error = $state('');
  let loaded = $state(false);

  let busy = false;
  let again = false;
  async function load() {
    const file = sub.sessionFile;
    if (!file) return;
    if (busy) {
      again = true;
      return;
    }
    busy = true;
    try {
      const page = await fleet.call('agents.subagentTimeline', { id: agentId, sessionFile: file, live }, host);
      items = page.items;
      hasMore = page.hasMore;
      error = '';
      loaded = true;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
    if (again) {
      again = false;
      void load();
    }
  }

  // Every change of its state (a tool step, its end, its session file becoming known) reads the
  // transcript again; the last read after it ended settles unanswered calls.
  const pulse = $derived(`${sub.sessionFile}|${sub.status}|${sub.toolCount ?? 0}|${sub.activity ?? ''}`);
  $effect(() => {
    void pulse;
    untrack(load);
  });
  // Assistant text between tool steps comes without a progress report.
  $effect(() => {
    if (!live) return;
    const timer = setInterval(() => document.visibilityState === 'visible' && load(), POLL_MS);
    return () => clearInterval(timer);
  });

  /** Images the subagent's calls returned (stored on the `task` call), by call id. */
  const shown = $derived.by(() => {
    const byCall = new Map<string, TimelineImage[]>();
    for (const img of item.images ?? []) {
      if (img.subagent !== sub.id || !img.toolCall) continue;
      const list = byCall.get(img.toolCall);
      if (list) list.push(img);
      else byCall.set(img.toolCall, [img]);
    }
    if (!byCall.size) return items;
    return items.map((it) => (it.kind === 'tool' && byCall.has(it.id) ? { ...it, images: byCall.get(it.id) } : it));
  });

  // ─── Follow the newest message ───────────────────────────────────────────────────────────────
  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  let stick = $state(true);
  let lastTop = 0;
  $effect(() => {
    if (!content || !scroller) return;
    const el = scroller;
    const ro = new ResizeObserver(() => {
      if (stick) el.scrollTop = el.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  });
  function onscroll() {
    const el = scroller;
    if (!el) return;
    // Content growth never moves scrollTop up, so only an upward scroll (the user) unpins.
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 40) stick = true;
    else if (el.scrollTop < lastTop) stick = false;
    lastTop = el.scrollTop;
  }
  function toBottom() {
    stick = true;
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' });
  }
  $effect(() => {
    if (loaded && scroller) void tick().then(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
  });

  function onclick(e: MouseEvent) {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('[data-copy]');
    if (!btn) return;
    const code = btn.closest('.md-code')?.querySelector('code')?.textContent ?? '';
    copyText(code).then(
      () => {
        btn.textContent = 'Copied';
        setTimeout(() => (btn.textContent = 'Copy'), 1200);
      },
      (err) => ui.error('Copy failed', err),
    );
  }

  const ran = $derived(sub.endedAt ? duration(Math.max(0, sub.endedAt - sub.startedAt)) : '');
</script>

<div class="detail">
  <div class="bar">
    <button class="btn ghost sm back" onclick={onback} title="All subagents"><ChevronLeft size={14} />Subagents</button>
    <span class="pos faint num">{position[0] + 1}/{position[1]}</span>
    <button class="btn ghost icon sm" disabled={position[1] < 2} onclick={() => onstep(-1)} aria-label="Previous subagent" title="Previous subagent"><ChevronLeft size={15} /></button>
    <button class="btn ghost icon sm" disabled={position[1] < 2} onclick={() => onstep(1)} aria-label="Next subagent" title="Next subagent"><ChevronRight size={15} /></button>
    <button class="btn ghost icon sm" onclick={onchat} aria-label="Show in chat" title="Show its task call in the chat"><MessageSquareText size={14} /></button>
  </div>
  <div class="head">
    <div class="title">
      <span class="name mono">{sub.id}</span>
      <span class="status" data-status={sub.status}>{sub.status}</span>
    </div>
    <div class="info faint num">
      <span>{sub.agent}</span>
      {#if sub.model}<span class="mono">{sub.model}</span>{/if}
      {#if ran}<span>{ran}</span>{/if}
      {#if sub.toolCount}<span>{sub.toolCount} tool{sub.toolCount === 1 ? '' : 's'}</span>{/if}
      {#if sub.costUsd}<span>{cost(sub.costUsd)}</span>{/if}
    </div>
    {#if sub.description && sub.description !== sub.id}<div class="desc">{sub.description}</div>{/if}
  </div>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scroller" bind:this={scroller} {onscroll} {onclick}>
    <div class="content" bind:this={content}>
      {#if hasMore}<div class="faint more">Earlier messages are not shown.</div>{/if}
      {#if !sub.sessionFile}
        <div class="faint">Waiting for its session…</div>
      {:else if error && !loaded}
        <div class="faint">Could not load the transcript: {error}</div>
      {:else if !loaded}
        <div class="loading"><span class="spinner"></span></div>
      {:else if !items.length}
        <div class="faint">No messages yet.</div>
      {/if}
      <ItemList items={shown} {host} {agentId} />
      {#if live}<div class="working"><span class="dots"><i></i><i></i><i></i></span><span class="ellipsis">{sub.activity ?? 'Working…'}</span></div>{/if}
    </div>
  </div>
  {#if !stick}
    <button class="jump btn sm" onclick={toBottom}><ArrowDown size={13} /> Latest</button>
  {/if}
</div>

<style>
  .detail {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .back {
    margin-right: auto;
    gap: 2px;
    padding-left: 4px;
  }
  .pos {
    font-size: var(--fs-xs);
    margin-right: 4px;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    color: var(--text);
  }
  .status {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--green);
  }
  .status[data-status='running'] {
    color: var(--blue);
  }
  .status[data-status='failed'] {
    color: var(--red);
  }
  .status[data-status='aborted'] {
    color: var(--text-3);
  }
  .info {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    font-size: var(--fs-xs);
  }
  .desc {
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-anchor: none;
  }
  .content {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 12px 14px 16px;
  }
  .more {
    text-align: center;
    font-size: var(--fs-xs);
  }
  .loading {
    display: grid;
    place-items: center;
    padding: 32px;
  }
  .working {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .dots {
    display: inline-flex;
    gap: 3px;
    flex: none;
  }
  .dots i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--blue);
    animation: blink 1.2s infinite both;
  }
  .dots i:nth-child(2) {
    animation-delay: 0.2s;
  }
  .dots i:nth-child(3) {
    animation-delay: 0.4s;
  }
  @keyframes blink {
    0%,
    80%,
    100% {
      opacity: 0.25;
    }
    40% {
      opacity: 1;
    }
  }
  .jump {
    position: absolute;
    left: 50%;
    bottom: 10px;
    transform: translateX(-50%);
    box-shadow: var(--shadow-pop);
    border: 0;
  }
</style>
