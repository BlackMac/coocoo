<script lang="ts">
  import type { Subagent, TimelineItem } from '../lib/protocol';
  import UserItem from './UserItem.svelte';
  import AssistantItem from './AssistantItem.svelte';
  import ThinkingItem from './ThinkingItem.svelte';
  import ToolItem from './ToolItem.svelte';
  import RefinementItem from './RefinementItem.svelte';

  // Timeline items as rendered in a transcript (an agent's, or a subagent's). Renders no wrapper
  // element, so the parent's layout applies to each block. `host`/`projectId`: the agent's project,
  // whose board shows card refinements (`kanban_refine`) below the call.
  let {
    items,
    onsubagent,
    host,
    projectId,
  }: { items: TimelineItem[]; onsubagent?: (s: Subagent) => void; host?: string; projectId?: string } = $props();
  const isRefinement = (it: TimelineItem) => it.tool?.name === 'kanban_refine' && it.tool.status === 'ok';

  /** Groups consecutive tool calls so they render as one compact block. */
  type Block = { kind: 'tools'; items: TimelineItem[]; id: string } | { kind: 'item'; item: TimelineItem; id: string };
  const blocks = $derived.by<Block[]>(() => {
    const out: Block[] = [];
    for (const it of items) {
      const last = out.at(-1);
      if (it.kind === 'tool') {
        if (last?.kind === 'tools') last.items.push(it);
        else out.push({ kind: 'tools', items: [it], id: `t:${it.id}` });
      } else out.push({ kind: 'item', item: it, id: it.id });
    }
    return out;
  });
</script>

{#each blocks as b (b.id)}
  {#if b.kind === 'tools'}
    <div class="tools">
      {#each b.items as it (it.id)}<ToolItem item={it} {onsubagent} />{/each}
    </div>
    {#if host && projectId}
      {#each b.items.filter(isRefinement) as it (it.id)}<RefinementItem item={it} {host} {projectId} />{/each}
    {/if}
  {:else if b.item.kind === 'user'}
    <UserItem item={b.item} />
  {:else if b.item.kind === 'assistant'}
    <AssistantItem item={b.item} />
  {:else if b.item.kind === 'thinking'}
    <ThinkingItem item={b.item} />
  {:else if b.item.kind === 'notice'}
    <div class="notice"><span>{b.item.text}</span></div>
  {:else if b.item.kind === 'command'}
    <pre class="command mono">{b.item.text}</pre>
  {:else}
    <div class="error">{b.item.text}</div>
  {/if}
{/each}

<style>
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
  .command {
    margin: 0;
    max-height: 420px;
    overflow: auto;
    padding: 9px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--code-bg);
    color: var(--text-2);
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre;
  }
</style>
