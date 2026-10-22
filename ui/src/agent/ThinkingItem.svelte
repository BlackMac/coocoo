<script lang="ts">
  import type { TimelineItem } from '../lib/protocol';
  import { Brain, ChevronRight } from '../lib/icons';

  let { item }: { item: TimelineItem } = $props();
  let open = $state(false);

  const firstLine = $derived((item.text ?? '').replace(/[*_#`]/g, '').trim().split('\n')[0]?.slice(0, 160) ?? '');
</script>

<div class="thinking" class:open>
  <button class="head" onclick={() => (open = !open)} aria-expanded={open}>
    <Brain size={13} />
    <span class="lbl" class:live={item.partial}>{item.partial ? 'Thinking…' : 'Thought'}</span>
    {#if !open}<span class="pv ellipsis">{firstLine}</span>{/if}
    <span class="chev"><ChevronRight size={13} /></span>
  </button>
  {#if open}<div class="body">{item.text}</div>{/if}
</div>

<style>
  .thinking {
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--text-3);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .head:hover {
    color: var(--text-2);
  }
  .lbl {
    flex: none;
    font-weight: 500;
  }
  .lbl.live {
    background: linear-gradient(90deg, var(--text-3) 0%, var(--text) 50%, var(--text-3) 100%);
    background-size: 200% 100%;
    background-clip: text;
    -webkit-background-clip: text;
    color: transparent;
    animation: shimmer 1.6s linear infinite;
  }
  .pv {
    flex: 1;
    font-style: italic;
    opacity: 0.85;
  }
  .chev {
    display: flex;
    flex: none;
    transition: transform var(--t-fast) var(--ease);
  }
  .open .chev {
    transform: rotate(90deg);
  }
  .body {
    margin: 6px 0 0 6px;
    padding: 2px 0 2px 14px;
    border-left: 2px solid var(--border);
    color: var(--text-2);
    font-size: var(--fs-sm);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
</style>
