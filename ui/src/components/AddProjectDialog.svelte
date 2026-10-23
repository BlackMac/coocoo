<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { FsEntry } from '../lib/protocol';
  import Dialog from './Dialog.svelte';
  import { ArrowUp, Folder, FolderGit2 } from '../lib/icons';

  let { hostId }: { hostId?: string } = $props();

  const online = $derived(fleet.hostList.filter((h) => h.online));
  let host = $state('');
  let path = $state('');
  let name = $state('');
  let dir = $state<{ path: string; parent?: string; entries: FsEntry[] } | null>(null);
  let listing = $state(false);
  let busy = $state(false);

  $effect.pre(() => {
    if (!host) host = hostId && fleet.host(hostId)?.online ? hostId : (online[0]?.id ?? '');
  });

  async function browse(p?: string) {
    if (!host) return;
    listing = true;
    try {
      dir = await fleet.call('fs.list', p ? { path: p } : {}, host);
      path = dir.path;
    } catch (err) {
      ui.error('Cannot list directory', err);
    } finally {
      listing = false;
    }
  }

  $effect(() => {
    if (host) void browse();
  });

  async function add(e: SubmitEvent) {
    e.preventDefault();
    if (!path.trim() || busy) return;
    busy = true;
    try {
      const p = await fleet.call('projects.add', { path: path.trim(), name: name.trim() || undefined }, host);
      fleet.projects[`${p.hostId}:${p.id}`] = p;
      ui.submitDialog();
      router.go({ name: 'project', host: p.hostId, id: p.id });
    } catch (err) {
      ui.error('Could not add project', err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Add project" width={560} onclose={() => (ui.dialog = null)}>
  <form id="add-project" class="form" onsubmit={add}>
    <div class="row2">
      <label class="field">
        <span>Host</span>
        <select class="select" bind:value={host}>
          {#each online as h (h.id)}<option value={h.id}>{h.name}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span>Name <span class="faint">(optional)</span></span>
        <input class="input" bind:value={name} placeholder="Directory name" />
      </label>
    </div>
    <label class="field">
      <span>Directory</span>
      <input class="input mono" bind:value={path} spellcheck="false" onkeydown={(e) => e.key === 'Tab' && !e.shiftKey && path && (e.preventDefault(), browse(path))} />
    </label>
    <div class="browser">
      <div class="bar">
        <button type="button" class="btn ghost sm" disabled={!dir?.parent || listing} onclick={() => browse(dir?.parent)}><ArrowUp size={13} /> Up</button>
        <span class="cur mono ellipsis">{dir?.path ?? ''}</span>
        {#if listing}<span class="spinner"></span>{/if}
      </div>
      <div class="entries">
        {#each dir?.entries.filter((e) => e.isDir) ?? [] as e (e.path)}
          <button type="button" class="entry" class:git={e.isGit} ondblclick={() => browse(e.path)} onclick={() => (path = e.path)} class:sel={path === e.path}>
            {#if e.isGit}<FolderGit2 size={14} />{:else}<Folder size={14} />{/if}
            <span class="ellipsis">{e.name}</span>
            {#if e.isGit}<span class="chip">git</span>{/if}
          </button>
        {:else}
          <div class="none">{listing ? '' : 'No subdirectories'}</div>
        {/each}
      </div>
    </div>
    <p class="hint">Click to select, double-click to open. Git is optional; projects never modify files on their own.</p>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="add-project" disabled={!path.trim() || busy}>
      {#if busy}<span class="spinner"></span>{/if}Add project
    </button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .browser {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    overflow: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 8px 0 4px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-2);
  }
  .cur {
    flex: 1;
    color: var(--text-2);
    font-size: 11.5px;
  }
  .entries {
    height: 220px;
    overflow-y: auto;
    padding: 4px;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    cursor: pointer;
  }
  .entry:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .entry.sel {
    background: var(--accent-soft);
    color: var(--text);
  }
  .entry.git {
    color: var(--text);
  }
  .entry .chip {
    margin-left: auto;
    height: 16px;
  }
  .none {
    padding: 20px;
    text-align: center;
    color: var(--text-3);
  }
  .hint {
    margin: 0;
  }
</style>
