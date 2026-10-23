<script lang="ts">
  // The concrete cases behind a workflow number: links to the agent at that moment (or to the
  // card's project). Following one is recorded, so the log shows whether examples get used.
  import { fleet } from '../../lib/fleet.svelte';
  import type { ExampleRef } from '../../lib/protocol';
  import { href, type Route } from '../../lib/router.svelte';
  import { track } from '../../lib/track';

  let {
    examples,
    section,
    title,
  }: {
    examples: ExampleRef[];
    /** Recorded with a followed link (`ui.example {section}`). */
    section: string;
    title?: string;
  } = $props();

  const multiHost = $derived(fleet.hostList.length > 1);

  /** Deleted agents and removed projects have nothing to open. */
  function target(ex: ExampleRef): Route | null {
    if (ex.agentId) return fleet.agent(ex.hostId, ex.agentId) ? { name: 'agent', host: ex.hostId, id: ex.agentId, at: ex.ts } : null;
    if (ex.projectId && fleet.project(ex.hostId, ex.projectId)) return { name: 'project', host: ex.hostId, id: ex.projectId };
    return null;
  }

  function label(ex: ExampleRef): string {
    if (ex.label) return ex.label;
    if (ex.agentId) return fleet.agent(ex.hostId, ex.agentId)?.title ?? `Agent ${ex.agentId}`;
    return ex.cardId ? `Card ${ex.cardId}` : 'Untitled';
  }

  function when(ts: number): string {
    return new Date(ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }

  function follow(ex: ExampleRef) {
    track('example', { agentId: ex.agentId, projectId: ex.projectId, cardId: ex.cardId, meta: { section } });
  }
</script>

<div class="examples">
  {#if title}<div class="title">{title}</div>{/if}
  {#if examples.length}
    <ul>
      {#each examples as ex, i (i)}
        {@const t = target(ex)}
        {@const project = !ex.agentId && ex.projectId ? fleet.project(ex.hostId, ex.projectId) : undefined}
        <li>
          {#if t}
            <a class="ellipsis" href={href(t)} onclick={() => follow(ex)}>{label(ex)}</a>
          {:else}
            <span class="ellipsis">{label(ex)}</span><span class="faint gone">{ex.agentId ? 'deleted' : 'removed'}</span>
          {/if}
          {#if project}<span class="faint ellipsis">{project.name}</span>{/if}
          <span class="faint num when">{when(ex.ts)}{multiHost ? ` · ${fleet.host(ex.hostId)?.name ?? ex.hostId}` : ''}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <div class="faint">No examples recorded.</div>
  {/if}
</div>

<style>
  .examples {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .title {
    color: var(--text-2);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  a {
    color: var(--accent-text);
    text-decoration: none;
  }
  a:hover {
    text-decoration: underline;
  }
  .gone {
    flex: none;
    font-size: var(--fs-xs);
  }
  .when {
    margin-left: auto;
    flex: none;
    font-size: var(--fs-xs);
  }
  @media (max-width: 600px) {
    li {
      flex-wrap: wrap;
      gap: 0 8px;
    }
    .ellipsis {
      max-width: 100%;
    }
    .when {
      margin-left: 0;
    }
  }
</style>
