<script lang="ts">
  import { tick } from 'svelte';
  import { fleet, key, type Outgoing, type Timeline } from '../lib/fleet.svelte';
  import type { AgentSummary, TimelineItem } from '../lib/protocol';
  import { base64Bytes } from '../lib/images';
  import { ui } from '../lib/ui.svelte';
  import { ArrowDown } from '../lib/icons';
  import ItemList from './ItemList.svelte';
  import UserItem from './UserItem.svelte';

  /** `at`: a moment (Unix ms) to scroll to and highlight once the timeline is loaded. */
  let { timeline, agent, at }: { timeline: Timeline; agent: AgentSummary; at?: number } = $props();

  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  /** Follow the tail while the user is at the bottom. */
  let stick = $state(true);

  const working = $derived(agent.status === 'running' || agent.status === 'starting');

  const agentKey = $derived(key(agent.hostId, agent.id));
  const outbox = $derived(fleet.outbox[agentKey] ?? []);
  /** Prompts starting a turn, shown provisionally until omp's `user` item replaces them. */
  const pending = $derived(outbox.filter((e) => e.target === 'transcript'));

  const asItem = (e: Outgoing): TimelineItem => ({
    seq: 0,
    id: `outbox:${e.id}`,
    ts: e.sentAt,
    kind: 'user',
    text: e.message.trim(),
    images: e.images.map((im, i) => ({ name: String(i), mime: im.mimeType, size: base64Bytes(im.data) })),
  });
  /** The images being sent, shown from the composer's data until the daemon stored them. */
  const localImages = (e: Outgoing) => e.images.map((im) => `data:${im.mimeType};base64,${im.data}`);

  function edit(e: Outgoing) {
    fleet.discard(agentKey, e.id);
    ui.restoreDraft(agentKey, e.draft);
  }

  // Sending jumps to the tail, even when scrolled up.
  let outboxSize = 0;
  $effect(() => {
    const n = outbox.length;
    if (n > outboxSize && scroller) {
      stick = true;
      const el = scroller;
      void tick().then(() => (el.scrollTop = el.scrollHeight));
    }
    outboxSize = n;
  });

  // Keep pinned to the bottom as content grows (new items, streaming text, expanding rows).
  $effect(() => {
    if (!content || !scroller) return;
    const el = scroller;
    const ro = new ResizeObserver(() => {
      if (stick) el.scrollTop = el.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  });

  $effect(() => {
    if (timeline.loaded && scroller) {
      void tick().then(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
    }
  });

  /** Older pages loaded at most while looking for `at`, so a far-off moment cannot load everything. */
  const MAX_AT_PAGES = 20;
  let highlight = $state<string>();
  let jumpedTo: number | undefined;

  $effect(() => {
    if (!at || at === jumpedTo || !timeline.loaded || !scroller) return;
    jumpedTo = at;
    void jumpTo(at);
  });

  async function jumpTo(ms: number) {
    stick = false;
    // The first item at or after `ms` may sit in an older page while the oldest loaded one is still later.
    for (let pages = 0; pages < MAX_AT_PAGES && timeline.hasMore && (timeline.items[0]?.ts ?? 0) >= ms; pages++) {
      if (!(await timeline.loadOlder())) break;
    }
    const target = timeline.items.find((it) => it.ts >= ms) ?? timeline.items.at(-1);
    if (!target) return;
    highlight = target.id;
    await tick();
    const el = scroller?.querySelector('.at');
    el?.scrollIntoView({ block: 'center' });
    // Items above render lazily (`content-visibility`), so their real height can shift the target once more.
    requestAnimationFrame(() => el?.scrollIntoView({ block: 'center' }));
    setTimeout(() => {
      if (highlight === target.id) highlight = undefined;
    }, 2600);
  }

  let lastTop = 0;

  async function onscroll() {
    const el = scroller;
    if (!el) return; // scroll events can still fire while the view is torn down
    // Content growth never moves scrollTop up, so only an upward scroll (the user) unpins.
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 40) stick = true;
    else if (el.scrollTop < lastTop) stick = false;
    lastTop = el.scrollTop;
    if (el.scrollTop < 300 && timeline.hasMore && !timeline.loading) {
      const before = el.scrollHeight;
      const added = await timeline.loadOlder();
      if (added) {
        await tick();
        el.scrollTop += el.scrollHeight - before;
      }
    }
  }

  function onclick(e: MouseEvent) {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('[data-copy]');
    if (!btn) return;
    const code = btn.closest('.md-code')?.querySelector('code')?.textContent ?? '';
    void navigator.clipboard.writeText(code).then(() => {
      btn.textContent = 'Copied';
      setTimeout(() => (btn.textContent = 'Copy'), 1200);
    });
  }

  function toBottom() {
    stick = true;
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' });
  }
</script>

<div class="wrap">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scroller" bind:this={scroller} {onscroll} {onclick}>
    <div class="content" bind:this={content}>
      {#if timeline.hasMore}
        <div class="older">{#if timeline.loading}<span class="spinner"></span>{:else}<button class="btn ghost sm" onclick={() => timeline.loadOlder()}>Load earlier messages</button>{/if}</div>
      {:else if timeline.loaded}
        <div class="start faint">Session started {new Date(agent.createdAt).toLocaleString()}</div>
      {/if}
      {#if !timeline.loaded && timeline.loading}
        <div class="loading"><span class="spinner"></span></div>
      {:else if timeline.error && !timeline.loaded}
        <div class="empty"><strong>Could not load the transcript</strong><span>{timeline.error}</span><button class="btn sm" onclick={() => timeline.load()}>Retry</button></div>
      {:else if timeline.loaded && !timeline.items.length && !pending.length}
        <div class="empty"><strong>No messages yet</strong><span>Send the first prompt below.</span></div>
      {/if}
      <ItemList
        items={timeline.items}
        host={agent.hostId}
        agentId={agent.id}
        projectId={agent.projectId}
        {highlight}
        onsubagent={(s) => s.sessionFile && (ui.dialog = { kind: 'subagent', hostId: agent.hostId, agentId: agent.id, subagentId: s.id, sessionFile: s.sessionFile })}
      />
      {#each pending as e (e.id)}
        <UserItem
          item={asItem(e)}
          local={localImages(e)}
          pending={e.state !== 'failed'}
          failed={e.state === 'failed' ? (e.error ?? 'Not sent') : undefined}
          onretry={() => fleet.retry(agentKey, e.id)}
          onedit={() => edit(e)}
          ondiscard={() => fleet.discard(agentKey, e.id)}
        />
      {/each}
      {#if working}
        <div class="working"><span class="dots"><i></i><i></i><i></i></span>{agent.todos?.current ?? 'Working…'}</div>
      {/if}
    </div>
  </div>
  {#if !stick}
    <button class="jump btn sm" onclick={toBottom}><ArrowDown size={13} /> Latest</button>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .scroller {
    flex: 1;
    overflow-y: auto;
    overflow-anchor: none;
  }
  .content {
    max-width: 920px;
    margin: 0 auto;
    padding: 16px 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .content > :global(*) {
    content-visibility: auto;
    contain-intrinsic-size: auto 60px;
  }
  .older,
  .start {
    display: flex;
    justify-content: center;
    font-size: var(--fs-xs);
  }
  .loading {
    display: grid;
    place-items: center;
    padding: 48px;
  }
  .working {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .dots {
    display: inline-flex;
    gap: 3px;
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
  @media (max-width: 640px) {
    .content {
      padding: 12px 10px 16px;
    }
  }
</style>
