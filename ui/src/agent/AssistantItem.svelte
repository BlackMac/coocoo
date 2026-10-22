<script lang="ts">
  import { untrack } from 'svelte';
  import { renderMarkdown } from '../lib/markdown';
  import type { TimelineItem } from '../lib/protocol';

  let { item }: { item: TimelineItem } = $props();

  // Streaming text re-renders at most ~10×/s; finished items render once (memoized).
  const THROTTLE_MS = 100;
  let html = $state('');
  let timer: number | undefined;

  $effect(() => {
    const text = item.text ?? '';
    if (!item.partial) {
      clearTimeout(timer);
      timer = undefined;
      html = renderMarkdown(text, `${item.id}:${text.length}`);
      return;
    }
    if (timer === undefined) {
      if (!untrack(() => html)) html = renderMarkdown(text);
      timer = setTimeout(() => {
        timer = undefined;
        html = renderMarkdown(item.text ?? '');
      }, THROTTLE_MS);
    }
  });

  $effect(() => () => clearTimeout(timer));
</script>

<div class="assistant md" class:partial={item.partial}>{@html html}</div>

<style>
  .assistant {
    min-width: 0;
  }
  .partial :global(> :last-child::after) {
    content: '';
    display: inline-block;
    width: 7px;
    height: 13px;
    margin-left: 2px;
    vertical-align: -2px;
    background: var(--text-2);
    animation: caret 1s steps(2) infinite;
  }
  @keyframes caret {
    50% {
      opacity: 0;
    }
  }
</style>
