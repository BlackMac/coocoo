<script lang="ts">
  // Small badge for how a commit was attributed and for agents that no longer exist as they were.
  // The tooltip says what the badge means, so a reader never has to guess.
  import type { CommitMatch } from '../../lib/protocol';

  let { kind }: { kind: CommitMatch | 'deleted' | 'archived' } = $props();

  const TAGS: Record<CommitMatch | 'deleted' | 'archived', { label: string; title: string }> = {
    exact: { label: 'exact · Kanban', title: 'Recorded by the Kanban board when the agent finished its card (kanban_done).' },
    turn: { label: 'during turn', title: "Made in the agent's repository or worktree while one of its turns ran (±60 s)." },
    shared: { label: 'shared', title: 'Made while turns of several agents of the same repository ran; counted for each of them.' },
    deleted: { label: 'deleted', title: 'Deleted in Coocoo; its numbers stay counted.' },
    archived: { label: 'archived', title: 'Archived in Coocoo.' },
  };
  const tag = $derived(TAGS[kind]);
</script>

<span class="tag {kind}" title={tag.title}>{tag.label}</span>

<style>
  .tag {
    display: inline-block;
    flex: none;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--bg-3);
    color: var(--text-2);
    font-size: 10px;
    font-weight: 500;
    line-height: 1.5;
    white-space: nowrap;
    vertical-align: 1px;
  }
  .exact {
    background: var(--green-soft);
    color: var(--add-text);
  }
  .turn {
    background: var(--amber-soft);
    color: var(--amber);
  }
  .shared {
    background: var(--violet-soft);
    color: var(--violet);
  }
  .deleted {
    background: var(--red-soft);
    color: var(--del-text);
  }
</style>
