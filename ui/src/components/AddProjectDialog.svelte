<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { FsEntry } from '../lib/protocol';
  import Dialog from './Dialog.svelte';
  import { ArrowUp, Folder, FolderGit2, FolderPlus, GitBranch } from '../lib/icons';

  let { hostId }: { hostId?: string } = $props();

  const online = $derived(fleet.hostList.filter((h) => h.online));
  /** `existing` adds `path` itself; `create` and `clone` make a new `folder` inside `path`. */
  let mode = $state<'existing' | 'create' | 'clone'>('existing');
  let host = $state('');
  let path = $state('');
  let name = $state('');
  let url = $state('');
  let folder = $state('');
  /** Whether the user typed the folder name; until then a clone takes it from the URL. */
  let folderEdited = $state(false);
  let dir = $state<{ path: string; parent?: string; entries: FsEntry[] } | null>(null);
  let listing = $state(false);
  let busy = $state(false);

  /** `https://host/org/repo.git`, `git@host:org/repo.git` → `repo`. */
  function repoName(u: string): string {
    return (
      u
        .trim()
        .replace(/[/\\]+$/, '')
        .split(/[/:\\]/)
        .pop()
        ?.replace(/\.git$/, '') ?? ''
    );
  }
  $effect.pre(() => {
    if (mode === 'clone' && !folderEdited) folder = repoName(url);
  });

  const folderOk = $derived(!!folder.trim() && !/[/\\]/.test(folder) && folder.trim() !== '.' && folder.trim() !== '..');
  const target = $derived(mode === 'existing' ? path.trim() : `${path.trim().replace(/\/+$/, '')}/${folder.trim()}`);
  const valid = $derived(!!path.trim() && (mode === 'existing' || (folderOk && (mode === 'create' || !!url.trim()))));

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
    if (!valid || busy) return;
    busy = true;
    try {
      const params = {
        path: target,
        name: name.trim() || undefined,
        create: mode === 'create' || undefined,
        cloneUrl: mode === 'clone' ? url.trim() : undefined,
      };
      // The daemon gives a clone up to 600 s.
      const p = await fleet.call('projects.add', params, host, mode === 'clone' ? 630_000 : undefined);
      fleet.projects[`${p.hostId}:${p.id}`] = p;
      ui.submitDialog();
      router.go({ name: 'project', host: p.hostId, id: p.id });
    } catch (err) {
      ui.error(mode === 'clone' ? 'Could not clone the repository' : 'Could not add project', err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Add project" width={560} onclose={() => (ui.dialog = null)}>
  <div class="tabs" role="tablist">
    <button role="tab" class:on={mode === 'existing'} aria-selected={mode === 'existing'} onclick={() => (mode = 'existing')}><Folder size={14} /> Existing directory</button>
    <button role="tab" class:on={mode === 'create'} aria-selected={mode === 'create'} onclick={() => (mode = 'create')}><FolderPlus size={14} /> New directory</button>
    <button role="tab" class:on={mode === 'clone'} aria-selected={mode === 'clone'} onclick={() => (mode = 'clone')}><GitBranch size={14} /> Clone from git</button>
  </div>
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
    {#if mode === 'clone'}
      <label class="field">
        <span>Repository URL</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value={url} placeholder="https://github.com/org/repo.git or git@host:org/repo.git" spellcheck="false" autocomplete="off" autofocus />
      </label>
    {/if}
    {#if mode === 'existing'}
      <label class="field">
        <span>Directory</span>
        <input class="input mono" bind:value={path} spellcheck="false" onkeydown={(e) => e.key === 'Tab' && !e.shiftKey && path && (e.preventDefault(), browse(path))} />
      </label>
    {:else}
      <div class="row2">
        <label class="field">
          <span>Parent directory</span>
          <input class="input mono" bind:value={path} spellcheck="false" onkeydown={(e) => e.key === 'Tab' && !e.shiftKey && path && (e.preventDefault(), browse(path))} />
        </label>
        <label class="field">
          <span>New folder</span>
          <input class="input mono" bind:value={folder} oninput={() => (folderEdited = true)} placeholder={mode === 'clone' ? 'From the URL' : 'my-project'} spellcheck="false" autocomplete="off" />
        </label>
      </div>
    {/if}
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
    {#if mode === 'existing'}
      <p class="hint">Click to select, double-click to open. Git is optional; projects never modify files on their own.</p>
    {:else if folder.trim() && !folderOk}
      <p class="hint err">The folder name must not contain <code>/</code>.</p>
    {:else}
      <p class="hint">
        {mode === 'clone' ? 'Clones into' : 'Creates'} <code>{folderOk && path.trim() ? target : '…'}</code>. Click to select the parent directory, double-click to open.
        {#if mode === 'clone'}Credentials come from git's helpers and SSH keys on the host; it never prompts.{/if}
      </p>
    {/if}
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="add-project" disabled={!valid || busy}>
      {#if busy}<span class="spinner"></span>{mode === 'clone' ? 'Cloning…' : 'Adding…'}{:else}Add project{/if}
    </button>
  {/snippet}
</Dialog>

<style>
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r);
    background: var(--bg-2);
  }
  .tabs button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 26px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    cursor: pointer;
  }
  .tabs button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--border-strong);
  }
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
  .hint.err {
    color: var(--red);
  }
  .hint code {
    overflow-wrap: anywhere;
  }
</style>
