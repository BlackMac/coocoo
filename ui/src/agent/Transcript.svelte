<script lang="ts">
  import { tick } from 'svelte';
  import type { Timeline } from '../lib/fleet.svelte';
  import type { AgentSummary, TimelineItem } from '../lib/protocol';
  import { ArrowDown } from '../lib/icons';
  import UserItem from './UserItem.svelte';
  import AssistantItem from './AssistantItem.svelte';
  import ThinkingItem from './ThinkingItem.svelte';
  import ToolItem from './ToolItem.svelte';

  let { timeline, agent }: { timeline: Timeline; agent: AgentSummary } = $props();

  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  /** Follow the tail while the user is at the bottom. */
  let stick = $state(true);

  const working = $derived(agent.status === 'running' || agent.status === 'starting');

  /** Groups consecutive tool calls so they render as one compact block. */
  type Block = { kind: 'tools'; items: TimelineItem[]; id: string } | { kind: 'item'; item: TimelineItem; id: string };
  const blocks = $derived.by<Block[]>(() => {
    const out: Block[] = [];
    for (const it of timeline.items) {
      const last = out.at(-1);
      if (it.kind === 'tool') {
        if (last?.kind === 'tools') last.items.push(it);
        else out.push({ kind: 'tools', items: [it], id: `t:${it.id}` });
      } else out.push({ kind: 'item', item: it, id: it.id });
    }
    return out;
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
      {:else if timeline.loaded && !timeline.items.length}
        <div class="empty"><strong>No messages yet</strong><span>Send the first prompt below.</span></div>
      {/if}
      {#each blocks as b (b.id)}
        {#if b.kind === 'tools'}
          <div class="tools">
            {#each b.items as it (it.id)}<ToolItem item={it} />{/each}
          </div>
        {:else if b.item.kind === 'user'}
          <UserItem item={b.item} />
        {:else if b.item.kind === 'assistant'}
          <AssistantItem item={b.item} />
        {:else if b.item.kind === 'thinking'}
          <ThinkingItem item={b.item} />
        {:else if b.item.kind === 'notice'}
          <div class="notice"><span>{b.item.text}</span></div>
        {:else}
          <div class="error">{b.item.text}</div>
        {/if}
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
  .tools {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg-1);
    overflow: hidden;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .notice::before,
  .notice::after {
    content: '';
    flex: 1;
    border-top: 1px solid var(--border);
  }
  .error {
    padding: 9px 12px;
    border: 1px solid color-mix(in srgb, var(--red) 30%, var(--border));
    border-radius: var(--r-lg);
    background: var(--red-soft);
    color: var(--red);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
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
