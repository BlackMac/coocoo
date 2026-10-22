<script lang="ts">
  import type { TimelineItem } from '../lib/protocol';
  import { MousePointerClick } from '../lib/icons';

  let { item }: { item: TimelineItem } = $props();

  /** Text split into plain runs and design-mode element blocks (see Composer.buildMessage). */
  type Part = { kind: 'text'; text: string } | { kind: 'element'; selector: string; body: string };

  const ELEMENT = /<selected-element url="[^"]*" selector="([^"]*)">\n?([\s\S]*?)\n?<\/selected-element>/g;
  const LONG = 900;

  const parts = $derived.by<Part[]>(() => {
    const text = item.text ?? '';
    const out: Part[] = [];
    let last = 0;
    for (const m of text.matchAll(ELEMENT)) {
      const before = text.slice(last, m.index).trim();
      if (before) out.push({ kind: 'text', text: before });
      out.push({ kind: 'element', selector: m[1], body: m[2] });
      last = m.index + m[0].length;
    }
    const rest = text.slice(last).trim();
    if (rest) out.push({ kind: 'text', text: rest });
    return out;
  });

  let expanded = $state(false);
  let openEl = $state<Record<number, boolean>>({});
  const long = $derived((item.text?.length ?? 0) > LONG && parts.length === 1);
</script>

<div class="user">
  <div class="bubble">
    {#each parts as p, i (i)}
      {#if p.kind === 'text'}
        <div class="txt" class:clamp={long && !expanded}>{p.text}</div>
      {:else}
        <button class="el" onclick={() => (openEl[i] = !openEl[i])} aria-expanded={!!openEl[i]}>
          <MousePointerClick size={12} /><span class="mono ellipsis">{p.selector}</span>
        </button>
        {#if openEl[i]}<pre class="mono elbody">{p.body}</pre>{/if}
      {/if}
    {/each}
    {#if long}<button class="more" onclick={() => (expanded = !expanded)}>{expanded ? 'Show less' : 'Show more'}</button>{/if}
  </div>
  <time class="faint" datetime={new Date(item.ts).toISOString()}>{new Date(item.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
</div>

<style>
  .user {
    align-self: flex-end;
    max-width: min(85%, 680px);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 3px;
  }
  .bubble {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    max-width: 100%;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 10px 10px 3px 10px;
    background: var(--bg-2);
  }
  .txt {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.55;
  }
  .txt.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 10;
    line-clamp: 10;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .el {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg);
    color: var(--accent-text);
    font-size: 11px;
    cursor: pointer;
  }
  .elbody {
    max-width: 100%;
    max-height: 240px;
    overflow: auto;
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--r);
    background: var(--code-bg);
    font-size: 11px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .more {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  time {
    font-size: 10.5px;
  }
</style>
