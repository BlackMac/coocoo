<script lang="ts">
  import type { Component } from 'svelte';
  import { duration } from '../lib/format';
  import type { Subagent, TimelineItem } from '../lib/protocol';
  import { Check, ChevronRight, FileText, Globe, Pencil, Search, Terminal, Users, Wrench, X, Link, ListTodo, MessageSquareText } from '../lib/icons';
  import SubagentList from './SubagentList.svelte';

  let { item, onsubagent }: { item: TimelineItem; onsubagent?: (s: Subagent) => void } = $props();
  let open = $state(false);

  const ICONS: Record<string, Component<{ size?: number }>> = {
    read: FileText,
    write: Pencil,
    edit: Pencil,
    ast_edit: Pencil,
    bash: Terminal,
    eval: Terminal,
    grep: Search,
    glob: Search,
    find: Search,
    ast_grep: Search,
    browser: Globe,
    web_search: Globe,
    fetch: Link,
    task: Users,
    todo_write: ListTodo,
    ask: MessageSquareText,
  };

  const tool = $derived(item.tool!);
  const Icon = $derived(ICONS[tool.name] ?? Wrench);

  /** Pretty-printed args, falling back to the raw (possibly truncated) JSON. */
  const args = $derived.by(() => {
    if (!open) return '';
    try {
      return JSON.stringify(JSON.parse(tool.args), null, 2);
    } catch {
      return tool.args;
    }
  });

  /** One-line summary when the tool carries no intent: the most telling argument. */
  const summary = $derived.by(() => {
    if (tool.intent) return tool.intent;
    try {
      const a = JSON.parse(tool.args);
      return String(a.path ?? a.command ?? a.pattern ?? a.url ?? a.query ?? a.description ?? '');
    } catch {
      return '';
    }
  });
</script>

<div class="tool" data-status={tool.status} class:open>
  <button class="row" onclick={() => (open = !open)} aria-expanded={open}>
    <span class="ic"><Icon size={13} /></span>
    <span class="name mono">{tool.name}</span>
    <span class="intent ellipsis">{summary}</span>
    <span class="dur num">{duration(tool.durationMs)}</span>
    <span class="st">
      {#if tool.status === 'running'}<span class="spinner"></span>{:else if tool.status === 'ok'}<Check size={13} />{:else}<X size={13} />{/if}
    </span>
    <span class="chev"><ChevronRight size={12} /></span>
  </button>
  {#if open}
    <div class="detail">
      <div class="lbl">Arguments</div>
      <pre class="mono">{args}</pre>
      {#if tool.output}
        <div class="lbl">Output</div>
        <pre class="mono out">{tool.output}</pre>
      {/if}
    </div>
  {/if}
  {#if tool.subagents?.length}<SubagentList subagents={tool.subagents} {onsubagent} />{/if}
</div>

<style>
  .tool + :global(.tool) {
    border-top: 1px solid var(--border);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border: 0;
    background: transparent;
    color: var(--text-2);
    text-align: left;
    cursor: pointer;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .ic {
    display: flex;
    color: var(--text-3);
    flex: none;
  }
  .name {
    flex: none;
    color: var(--text);
    font-size: 11.5px;
    font-weight: 500;
  }
  .intent {
    flex: 1;
    font-size: var(--fs-sm);
  }
  .dur {
    flex: none;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .st {
    display: grid;
    place-items: center;
    width: 14px;
    flex: none;
    color: var(--green);
  }
  .st .spinner {
    width: 11px;
    height: 11px;
    border-top-color: var(--blue);
  }
  [data-status='error'] .st,
  [data-status='error'] .name {
    color: var(--red);
  }
  .chev {
    display: flex;
    flex: none;
    color: var(--text-3);
    transition: transform var(--t-fast) var(--ease);
  }
  .open .chev {
    transform: rotate(90deg);
  }
  .detail {
    padding: 2px 10px 10px 31px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .lbl {
    margin-top: 4px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  pre {
    margin: 0;
    max-height: 320px;
    overflow: auto;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--code-bg);
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  [data-status='error'] .out {
    color: var(--del-text);
  }
</style>
