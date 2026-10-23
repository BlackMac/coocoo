<script lang="ts">
  import { key } from '../lib/fleet.svelte';
  import { now } from '../lib/clock.svelte';
  import type { AgentSummary } from '../lib/protocol';
  import { OPERATION_LABEL, askToUpdate, dismiss, fetchedLabel, fetchProject, fetching, isDismissed, isStale, isWorking, needsBar, plural, type AgentGit } from '../lib/git.svelte';
  import { Bot, RefreshCw, TriangleAlert, X } from '../lib/icons';

  // Above the composer: git problems to sort out before the next prompt (a checkout behind its
  // upstream, conflicts, an operation in progress, a detached HEAD). Dismissed until it changes.
  let { agent: a, git: g }: { agent: AgentSummary; git: AgentGit } = $props();

  const visible = $derived(a.status !== 'stopped' && needsBar(g) && !isDismissed(a, g));
  const busyFetch = $derived(!!fetching[key(g.project.hostId, g.project.id)]);
  const answering = $derived(isWorking(a));
</script>

{#if visible}
  <div class="gbar" data-sev={g.severity}>
    <TriangleAlert size={13} />
    <span class="lbl">Git</span>
    <span class="txt ellipsis">
      {#if g.operation}
        <b class="mono">{g.label}</b>: {OPERATION_LABEL[g.operation]} in progress{#if g.conflicts}, {plural(g.conflicts, 'conflict')}{/if}
      {:else if g.conflicts}
        <b class="mono">{g.label}</b> has {plural(g.conflicts, 'unresolved conflict')}
      {:else if g.detached}
        Detached HEAD at <b class="mono">{g.label}</b>
      {:else}
        <b class="mono">{g.label}</b> is <b>{plural(g.behind, 'commit')} behind</b> <span class="mono">{g.against}</span>
        <span class="faint" class:stale={isStale(g.fetchedAt, now.value)}>· {fetchedLabel(g.fetchedAt, now.value)}</span>
      {/if}
    </span>
    {#if g.severity !== 'bad'}
      <button class="btn sm" onclick={() => fetchProject(g.project)} disabled={busyFetch}>
        <span class="ic" class:spin={busyFetch}><RefreshCw size={13} /></span><span class="wide">Fetch</span>
      </button>
    {/if}
    <button class="btn sm primary" disabled={answering} title={answering ? 'Unavailable while the agent is answering' : undefined} onclick={() => askToUpdate(a, g, now.value)}>
      <Bot size={13} /><span class="wide">{g.severity === 'bad' ? 'Ask agent to resolve' : 'Ask agent to update'}</span>
    </button>
    <button class="btn ghost icon sm" title="Hide until this changes" aria-label="Hide" onclick={() => dismiss(a, g)}><X size={13} /></button>
  </div>
{/if}

<style>
  .gbar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 4px 4px 4px 10px;
    border: 1px solid color-mix(in srgb, var(--amber) 45%, var(--border));
    border-radius: var(--r-lg);
    background: var(--amber-soft);
    color: var(--amber);
    font-size: var(--fs-sm);
  }
  .gbar[data-sev='bad'] {
    border-color: color-mix(in srgb, var(--red) 45%, var(--border));
    background: var(--red-soft);
    color: var(--red);
  }
  .gbar :global(svg) {
    flex: none;
  }
  .lbl {
    flex: none;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .txt {
    flex: 1;
    color: var(--text);
  }
  .stale {
    color: var(--amber);
  }
  .btn {
    flex: none;
    gap: 6px;
  }
  .ic {
    display: flex;
  }
  .spin {
    animation: spin 0.8s linear infinite;
  }
  @container (max-width: 600px) {
    .wide {
      display: none;
    }
  }
</style>
