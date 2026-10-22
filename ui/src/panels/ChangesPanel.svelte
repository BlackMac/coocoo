<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary, DiffFile, DiffResult, Project } from '../lib/protocol';
  import { ChevronRight, GitCommitHorizontal, GitMerge, RefreshCw } from '../lib/icons';
  import DiffView from './DiffView.svelte';

  let { agent: a, project }: { agent: AgentSummary; project?: Project } = $props();

  const BIG = 1500;

  let diff = $state<DiffResult | null>(null);
  let loading = $state(false);
  let error = $state('');
  let message = $state('');
  let committing = $state(false);
  let merging = $state(false);
  let collapsed = $state<Record<string, boolean>>({});
  let scroller = $state<HTMLDivElement>();

  const base = $derived(a.worktree?.base);
  const totals = $derived(
    (diff?.files ?? []).reduce((t, f) => ({ add: t.add + f.additions, del: t.del + f.deletions }), { add: 0, del: 0 }),
  );

  async function load() {
    loading = true;
    error = '';
    try {
      const d = await fleet.call('git.diff', { cwd: a.cwd, base }, a.hostId, 30_000);
      for (const f of d.files) if (!(f.path in collapsed) && f.patch.length > 0 && f.patch.split('\n').length > BIG) collapsed[f.path] = true;
      diff = d;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  // Refetch when the agent reports a new diff stat (after each turn) — debounced, event-driven.
  let timer: number | undefined;
  $effect(() => {
    void [a.diff?.files, a.diff?.additions, a.diff?.deletions, a.cwd];
    clearTimeout(timer);
    timer = setTimeout(load, diff ? 600 : 0);
    return () => clearTimeout(timer);
  });

  function jump(path: string) {
    collapsed[path] = false;
    scroller?.querySelector(`[data-file="${CSS.escape(path)}"]`)?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  }

  async function commit(e: SubmitEvent) {
    e.preventDefault();
    if (!message.trim() || committing) return;
    committing = true;
    try {
      const c = await fleet.call('git.commit', { cwd: a.cwd, message: message.trim() }, a.hostId);
      ui.toast('success', `Committed ${c.sha.slice(0, 7)}`, c.summary);
      message = '';
      await load();
    } catch (err) {
      ui.error('Commit failed', err);
    } finally {
      committing = false;
    }
  }

  function merge() {
    if (!a.worktree || !project) return;
    const wt = a.worktree;
    ui.dialog = {
      kind: 'confirm',
      title: `Merge into ${wt.base}`,
      message: `Merge branch ${wt.branch} into ${project.git?.branch ?? wt.base} in ${project.path}? Uncommitted changes in the worktree are not included — commit them first.`,
      action: 'Merge',
      run: async () => {
        merging = true;
        try {
          const r = await fleet.call('git.merge', { projectId: project.id, branch: wt.branch }, a.hostId, 60_000);
          if (r.ok) ui.toast('success', `Merged ${wt.branch}`, r.output.split('\n')[0]);
          else ui.toast('error', 'Merge failed — aborted', r.output);
        } finally {
          merging = false;
        }
      },
    };
  }

  function split(p: string) {
    const i = p.lastIndexOf('/');
    return i < 0 ? ['', p] : [p.slice(0, i + 1), p.slice(i + 1)];
  }

  const STATUS: Record<string, string> = { M: 'Modified', A: 'Added', D: 'Deleted', R: 'Renamed', '?': 'Untracked' };
</script>

{#snippet fileName(f: DiffFile)}
  {@const [dir, name] = split(f.path)}
  <span class="fname ellipsis" title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}><span class="dir">{dir}</span>{name}</span>
{/snippet}

<div class="changes">
  <div class="toolbar">
    <span class="what">
      {#if diff}
        <strong class="num">{diff.files.length}</strong> file{diff.files.length === 1 ? '' : 's'}
        <span class="add num">+{totals.add}</span><span class="del num">−{totals.del}</span>
        <span class="faint">vs {base ? `${base} (${diff.base.slice(0, 7)})` : 'HEAD'}</span>
      {:else if loading}
        <span class="faint">Loading diff…</span>
      {/if}
    </span>
    <button class="btn ghost icon sm" title="Refresh" aria-label="Refresh" onclick={load} disabled={loading}>
      <span class:spin={loading}><RefreshCw size={13} /></span>
    </button>
  </div>

  {#if error}
    <div class="empty"><strong>Could not load changes</strong><span>{error}</span></div>
  {:else if diff && !diff.files.length}
    <div class="empty"><strong>No changes</strong><span>The working tree matches {base ? `the merge-base with ${base}` : 'HEAD'}.</span></div>
  {:else if diff}
    <div class="files">
      {#each diff.files as f (f.path)}
        <button class="frow" onclick={() => jump(f.path)}>
          <span class="st" data-s={f.status} title={STATUS[f.status] ?? f.status}>{f.status === '?' ? 'U' : f.status}</span>
          {@render fileName(f)}
          <span class="num cnt"><span class="add">+{f.additions}</span> <span class="del">−{f.deletions}</span></span>
        </button>
      {/each}
    </div>
    <div class="diffs" bind:this={scroller}>
      {#each diff.files as f (f.path)}
        <section class="file" data-file={f.path}>
          <button class="fhead" onclick={() => (collapsed[f.path] = !collapsed[f.path])} aria-expanded={!collapsed[f.path]}>
            <span class="chev" class:open={!collapsed[f.path]}><ChevronRight size={13} /></span>
            <span class="st" data-s={f.status}>{f.status === '?' ? 'U' : f.status}</span>
            {@render fileName(f)}
            <span class="num cnt"><span class="add">+{f.additions}</span> <span class="del">−{f.deletions}</span></span>
          </button>
          {#if !collapsed[f.path]}
            {#if f.binary}
              <div class="note">Binary file</div>
            {:else if !f.patch}
              <div class="note">{f.status === 'D' ? 'File deleted' : 'No textual changes'}</div>
            {:else}
              <DiffView patch={f.patch} />
            {/if}
          {/if}
        </section>
      {/each}
      {#if diff.truncated}<div class="note">Diff truncated to stay within the size budget.</div>{/if}
    </div>
  {:else}
    <div class="empty"><span class="spinner"></span></div>
  {/if}

  <form class="commit" onsubmit={commit}>
    <GitCommitHorizontal size={14} class="faint" />
    <input class="input" bind:value={message} placeholder="Commit message" disabled={!diff?.files.length} />
    <button class="btn sm" type="submit" disabled={!message.trim() || committing || !diff?.files.length}>
      {#if committing}<span class="spinner"></span>{/if}Commit all
    </button>
    {#if a.worktree && project}
      <button class="btn primary sm" type="button" onclick={merge} disabled={merging} title="Merge {a.worktree.branch} into {a.worktree.base}">
        <GitMerge size={13} /> Merge into {a.worktree.base}
      </button>
    {/if}
  </form>
</div>

<style>
  .changes {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 8px 0 14px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-sm);
    flex: none;
  }
  .what {
    display: flex;
    align-items: baseline;
    gap: 7px;
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
  }
  .spin {
    display: flex;
    animation: spin 0.8s linear infinite;
  }
  .files {
    max-height: 30%;
    overflow-y: auto;
    padding: 4px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .frow,
  .fhead {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .frow:hover {
    background: var(--bg-hover);
  }
  .st {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex: none;
    border-radius: 3px;
    font-family: var(--mono);
    font-size: 10px;
    font-weight: 700;
    color: var(--amber);
    background: var(--amber-soft);
  }
  .st[data-s='A'],
  .st[data-s='?'] {
    color: var(--green);
    background: var(--green-soft);
  }
  .st[data-s='D'] {
    color: var(--red);
    background: var(--red-soft);
  }
  .st[data-s='R'] {
    color: var(--blue);
    background: var(--blue-soft);
  }
  .fname {
    flex: 1;
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .dir {
    color: var(--text-3);
  }
  .cnt {
    flex: none;
    font-family: var(--mono);
    font-size: 11px;
  }
  .diffs {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--bg);
  }
  .file {
    border-bottom: 1px solid var(--border);
    content-visibility: auto;
    contain-intrinsic-size: auto 200px;
  }
  .fhead {
    position: sticky;
    top: 0;
    z-index: 1;
    height: 32px;
    border-radius: 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg-1);
  }
  .chev {
    display: flex;
    color: var(--text-3);
    transition: transform var(--t-fast) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .note {
    padding: 12px 14px;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .empty {
    flex: 1;
  }
  .commit {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    background: var(--bg-1);
    flex: none;
  }
  .commit .input {
    flex: 1;
    min-width: 80px;
    height: 26px;
  }
</style>
