<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { compact, cost, dateTime } from '../lib/format';
  import { href } from '../lib/router.svelte';
  import { agentState } from '../lib/status';
  import { ui } from '../lib/ui.svelte';
  import type { AgentDetail, AgentSummary, Project } from '../lib/protocol';
  import { Copy } from '../lib/icons';
  import StatusDot from '../components/StatusDot.svelte';

  let { agent: a, detail, project }: { agent: AgentSummary; detail?: AgentDetail; project?: Project } = $props();

  const parent = $derived(a.parentId ? fleet.agent(a.hostId, a.parentId) : undefined);
  const children = $derived(fleet.childrenOf(a.hostId, a.id));
  const host = $derived(fleet.host(a.hostId));

  function copy(t: string) {
    void navigator.clipboard.writeText(t).then(() => ui.toast('success', 'Copied'));
  }
</script>

{#snippet path(label: string, value: string | undefined)}
  <dt>{label}</dt>
  <dd>
    {#if value}
      <button class="pv mono" title="Copy" onclick={() => copy(value)}><span>{value}</span><Copy size={11} /></button>
    {:else}<span class="faint">—</span>{/if}
  </dd>
{/snippet}

<div class="info">
  <dl>
    <dt>Host</dt>
    <dd>{host?.name ?? a.hostId}</dd>
    <dt>Project</dt>
    <dd>{#if project}<a href={href({ name: 'project', host: a.hostId, id: project.id })}>{project.name}</a>{:else}<span class="faint">—</span>{/if}</dd>
    {@render path('Working dir', a.cwd)}
    {#if a.worktree}
      <dt>Worktree</dt>
      <dd><span class="mono">{a.worktree.branch}</span> <span class="faint">from</span> <span class="mono">{a.worktree.base}</span></dd>
    {/if}
    {@render path('Session file', a.sessionFile)}
    <dt>Model</dt>
    <dd class="mono">{a.model ?? 'default'}</dd>
    <dt>Thinking</dt>
    <dd>{a.thinking ?? 'default'}</dd>
    <dt>Context</dt>
    <dd class="num">
      {a.contextPercent != null ? `${Math.round(a.contextPercent)}%` : '—'}
      {#if detail?.contextWindow}<span class="faint">of {compact(detail.contextWindow)} tokens</span>{/if}
    </dd>
    <dt>Tokens</dt>
    <dd class="num">{compact(a.tokens)}</dd>
    <dt>Cost</dt>
    <dd class="num">{cost(a.costUsd)}</dd>
    <dt>Created</dt>
    <dd>{dateTime(a.createdAt)}</dd>
    <dt>Last activity</dt>
    <dd>{dateTime(a.lastActivityAt)}</dd>
    <dt>Agent id</dt>
    <dd class="mono">{a.id}</dd>
  </dl>

  {#if parent || children.length}
    <h3 class="section-title">Related agents</h3>
    <div class="rel">
      {#if parent}
        <a class="ra" href={href({ name: 'agent', host: a.hostId, id: parent.id })}>
          <span class="faint">Parent</span><StatusDot state={agentState(parent)} size={7} /><span class="ellipsis">{parent.title}</span>
        </a>
      {/if}
      {#each children as c (c.id)}
        <a class="ra" href={href({ name: 'agent', host: a.hostId, id: c.id })}>
          <span class="faint">Child</span><StatusDot state={agentState(c)} size={7} /><span class="ellipsis">{c.title}</span>
        </a>
      {/each}
    </div>
  {/if}
</div>

<style>
  .info {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px 24px;
  }
  dl {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    gap: 9px 12px;
    margin: 0 0 22px;
    font-size: var(--fs-sm);
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  dd a,
  .ra:hover .ellipsis {
    color: var(--accent-text);
  }
  .mono {
    font-size: 11.5px;
  }
  .pv {
    display: inline-flex;
    align-items: flex-start;
    gap: 6px;
    max-width: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .pv span {
    overflow-wrap: anywhere;
  }
  .pv :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--text-3);
  }
  .rel {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ra {
    display: grid;
    grid-template-columns: 44px 10px minmax(0, 1fr);
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r);
    font-size: var(--fs-sm);
  }
  .ra:hover {
    background: var(--bg-hover);
  }
</style>
