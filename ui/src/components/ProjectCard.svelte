<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { ago, shortPath } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { href } from '../lib/router.svelte';
  import { agentState, STATE_RANK } from '../lib/status';
  import type { Project } from '../lib/protocol';
  import { FolderGit2, GitBranch, GitCommitHorizontal } from '../lib/icons';
  import HostDot from './HostDot.svelte';
  import StatusDot from './StatusDot.svelte';
  import PersonaAvatar from './PersonaAvatar.svelte';

  let { project: p }: { project: Project } = $props();

  const host = $derived(fleet.host(p.hostId));
  const agents = $derived(
    fleet
      .agentsOf(p.hostId, p.id)
      .filter((a) => a.status !== 'stopped')
      .map((a) => agentState(a))
      .sort((a, b) => STATE_RANK[a] - STATE_RANK[b]),
  );
</script>

<a class="pcard card" href={href({ name: 'project', host: p.hostId, id: p.id })}>
  <div class="top">
    {#if p.persona}<PersonaAvatar project={p} size={16} />{:else}<FolderGit2 size={15} />{/if}
    <span class="name ellipsis" title={p.persona ? `${p.persona.name} · project ${p.name}` : undefined}>{p.persona?.name ?? p.name}</span>
    <span class="chip"><HostDot online={host?.online ?? false} size={5} />{host?.name ?? p.hostId}</span>
  </div>
  <div class="path mono ellipsis" title={p.path}>{shortPath(p.path, 3)}</div>
  {#if p.git}
    <div class="git num">
      <span class="br mono"><GitBranch size={12} />{p.git.branch ?? p.git.head ?? 'detached'}</span>
      {#if p.git.ahead}<span title="Ahead of {p.git.upstream}">↑{p.git.ahead}</span>{/if}
      {#if p.git.behind}<span title="Behind {p.git.upstream}" class="behind">↓{p.git.behind}</span>{/if}
      {#if p.git.changed}<span class="changed">{p.git.changed} changed</span>{/if}
      {#if p.git.worktrees.length > 1}<span class="faint">{p.git.worktrees.length - 1} worktree{p.git.worktrees.length > 2 ? 's' : ''}</span>{/if}
    </div>
    {#if p.git.lastCommit}
      <div class="commit ellipsis" title={p.git.lastCommit.summary}>
        <GitCommitHorizontal size={12} />
        <span class="ellipsis">{p.git.lastCommit.summary}</span>
        <span class="faint num">{ago(p.git.lastCommit.ts, now.value)}</span>
      </div>
    {/if}
  {:else}
    <div class="git faint">Not a git repository</div>
  {/if}
  <div class="agents">
    {#if agents.length}
      <span class="dots">{#each agents.slice(0, 12) as s, i (i)}<StatusDot state={s} size={7} />{/each}</span>
      <span class="faint">{agents.length} agent{agents.length === 1 ? '' : 's'}</span>
    {:else}
      <span class="faint">No agents</span>
    {/if}
  </div>
</a>

<style>
  .pcard {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
    padding: 12px 14px;
    color: var(--text);
    transition: border-color var(--t-fast) var(--ease);
  }
  .pcard:hover {
    border-color: var(--border-strong);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .name {
    flex: 1;
    color: var(--text);
    font-weight: 600;
  }
  .chip {
    height: 18px;
    flex: none;
  }
  .path {
    color: var(--text-3);
    font-size: 11px;
  }
  .git {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .br {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text);
    font-size: 11px;
  }
  .behind {
    color: var(--amber);
  }
  .changed {
    color: var(--accent-text);
  }
  .commit {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .agents {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    padding-top: 9px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .dots {
    display: flex;
    gap: 4px;
  }
</style>
