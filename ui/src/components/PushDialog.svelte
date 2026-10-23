<script lang="ts">
  import { onMount } from 'svelte';
  import type { Dialog as DialogState } from '../lib/ui.svelte';
  import { ui } from '../lib/ui.svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import type { CommitInfo } from '../lib/protocol';
  import { agentGit, isWorking, plural, pushNow, pushTarget, pushing, wtKey } from '../lib/git.svelte';
  import { TriangleAlert } from '../lib/icons';
  import Dialog from './Dialog.svelte';

  // Push from the shared project checkout: lists what goes out (with the done cards the commits
  // belong to) and warns when another agent is still working in the checkout. Nothing is locked.
  let { dialog }: { dialog: Extract<DialogState, { kind: 'push' }> } = $props();

  const a = $derived(fleet.agent(dialog.hostId, dialog.agentId));
  const g = $derived(a ? agentGit(a, fleet.project(a.hostId, a.projectId), Object.values(fleet.agents)) : undefined);
  const board = $derived(a?.projectId ? fleet.boards[key(a.hostId, a.projectId)] : undefined);
  const target = $derived(g ? pushTarget(g) : undefined);
  /** Other agents in this checkout that may still commit. */
  const others = $derived(g && a ? g.shared.filter((x) => x.id !== a.id && isWorking(x)) : []);
  /** The agent itself is mid-turn and may still commit: the push waits for its answer. */
  const answering = $derived(!!a && isWorking(a));
  const busy = $derived(!!g && !!pushing[wtKey(g)]);

  let commits = $state<CommitInfo[] | null>(null);
  let error = $state('');

  onMount(() => {
    if (!a || !g) return;
    const { hostId, id: projectId } = g.project;
    if (!fleet.boards[key(hostId, projectId)]) fleet.loadBoard(hostId, projectId).catch((err) => ui.error('Could not load the board', err));
    fleet.call('git.outgoing', { projectId, path: g.wt.path }, hostId, 30_000).then(
      (c) => (commits = c),
      (err) => (error = err instanceof Error ? err.message : String(err)),
    );
  });

  async function confirm() {
    if (!a || !g) return;
    await pushNow(a, g);
    ui.submitDialog();
  }
</script>

<Dialog title={g ? `Push ${g.label} to ${target ?? 'the remote'}?` : 'Push'} width={540} onclose={() => (ui.dialog = null)}>
  {#if !g}
    <p class="msg">This agent's checkout is no longer known.</p>
  {:else if error}
    <p class="msg err">Could not list the commits: {error}</p>
  {:else if !commits}
    <div class="loading"><span class="spinner"></span></div>
  {:else}
    <p class="msg">
      {#if commits.length}
        {plural(commits.length, 'commit')} will be pushed{g.wt.upstream ? '' : ` (creates ${target})`}:
      {:else if g.wt.upstream}
        Nothing to push: {target} has every commit.
      {:else}
        No new commits; the push creates {target}.
      {/if}
    </p>
    {#if commits.length}
      <div class="commits">
        {#each commits as c (c.sha)}
          {@const card = board?.cards.find((k) => k.commit && (k.commit.sha.startsWith(c.sha) || c.sha.startsWith(k.commit.sha)))}
          <div class="crow">
            <span class="mono faint">{c.sha.slice(0, 7)}</span>
            <span class="subj" title={c.summary}>{c.summary}</span>
            <span class="faint meta">{c.author} · {ago(c.ts, now.value)}</span>
            <span class="ref">{#if card}<span class="card-ref" title={card.title}>{card.title}</span>{/if}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
  {#if answering}
    <div class="warnbox">
      <TriangleAlert size={13} />
      <span><b>{a?.title}</b> is answering right now. Push once it is done.</span>
    </div>
  {/if}
  {#if others.length}
    <div class="warnbox">
      <TriangleAlert size={13} />
      <span>
        {#each others as x, i (x.id)}{i ? (i === others.length - 1 ? ' and ' : ', ') : ''}<b>{x.title}</b>{/each}
        {others.length === 1 ? 'is' : 'are'} working in this checkout right now. {others.length === 1 ? 'Its' : 'Their'} later commits are not included,
        but uncommitted work in progress might be committed by {others.length === 1 ? 'it' : 'them'} after the push.
      </span>
    </div>
  {/if}
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" disabled={!g || !commits || busy || answering || (!commits.length && !!g.wt.upstream)} onclick={confirm}>
      {#if busy}<span class="spinner"></span>{/if}Push{commits?.length ? ` ${plural(commits.length, 'commit')}` : ''}
    </button>
  {/snippet}
</Dialog>

<style>
  .msg {
    margin: 0;
    color: var(--text-2);
  }
  .err {
    color: var(--red);
    overflow-wrap: anywhere;
  }
  .loading {
    display: flex;
    justify-content: center;
    padding: 12px;
  }
  .commits {
    max-height: 300px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--r);
  }
  .crow {
    display: grid;
    grid-template-columns: 58px minmax(0, 1fr) auto minmax(0, 110px);
    align-items: center;
    gap: 8px;
    padding: 5px 9px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-sm);
  }
  .crow:first-child {
    border-top: 0;
  }
  .subj {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    white-space: nowrap;
  }
  .ref {
    display: flex;
    justify-content: flex-end;
    min-width: 0;
  }
  .card-ref {
    max-width: 100%;
    overflow: hidden;
    padding: 0 6px;
    border-radius: 99px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 10.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warnbox {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 7px 9px;
    border: 1px solid color-mix(in srgb, var(--amber) 40%, var(--border));
    border-radius: var(--r);
    background: var(--amber-soft);
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .warnbox :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--amber);
  }
  .btn .spinner {
    width: 11px;
    height: 11px;
  }
  @media (max-width: 640px) {
    .crow {
      grid-template-columns: 58px minmax(0, 1fr);
      row-gap: 2px;
    }
    .meta,
    .ref {
      grid-column: 2;
      justify-content: flex-start;
    }
  }
</style>
