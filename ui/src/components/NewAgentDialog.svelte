<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet } from '../lib/fleet.svelte';
  import ModelPicker from './ModelPicker.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { ModelInfo } from '../lib/protocol';
  import Dialog from './Dialog.svelte';
  import { GitBranch } from '../lib/icons';

  let { hostId, projectId }: { hostId?: string; projectId?: string } = $props();

  const CUSTOM = '__cwd__';
  const THINKING = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'];

  const onlineProjects = $derived(fleet.projectList.filter((p) => fleet.host(p.hostId)?.online));
  // Dialog props are fixed for its lifetime; they only seed the form.
  const initial = untrack(() => {
    if (projectId && hostId) return `${hostId}:${projectId}`;
    const first = fleet.projectList.find((p) => (!hostId || p.hostId === hostId) && fleet.host(p.hostId)?.online);
    return first ? `${first.hostId}:${first.id}` : CUSTOM;
  });

  let target = $state(initial);
  let customHost = $state(untrack(() => hostId ?? fleet.homeId));
  let cwd = $state('');
  let title = $state('');
  let prompt = $state('');
  let worktree = $state(false);
  let branch = $state('');
  let base = $state('');
  let model = $state('');
  let thinking = $state('');
  let busy = $state(false);
  let models = $state<ModelInfo[]>([]);
  let modelsError = $state('');

  const project = $derived(target === CUSTOM ? undefined : fleet.projects[target]);
  const host = $derived(project?.hostId ?? customHost);
  const isGit = $derived(!!project?.git);

  $effect(() => {
    const h = host;
    models = [];
    modelsError = '';
    fleet
      .loadModels(h)
      .then((m) => {
        if (h === host) models = m;
      })
      .catch((e) => (modelsError = e.message));
  });

  async function create(e?: SubmitEvent) {
    e?.preventDefault();
    if (busy) return;
    if (!project && !cwd.trim()) return ui.toast('error', 'Pick a project or enter a directory');
    busy = true;
    try {
      const wt = worktree && isGit ? (branch.trim() || base.trim() ? { branch: branch.trim() || undefined, base: base.trim() || undefined } : true) : undefined;
      const a = await fleet.call(
        'agents.create',
        {
          projectId: project?.id,
          cwd: project ? undefined : cwd.trim(),
          title: title.trim() || undefined,
          prompt: prompt.trim() || undefined,
          worktree: wt,
          model: model || undefined,
          thinking: thinking || undefined,
        },
        host,
        60_000,
      );
      fleet.agents[`${a.hostId}:${a.id}`] = a;
      // Shown at once while omp starts; its `user` item replaces it.
      if (prompt.trim()) fleet.noteSent(a.hostId, a.id, prompt.trim());
      ui.dialog = null;
      router.go({ name: 'agent', host: a.hostId, id: a.id });
    } catch (err) {
      ui.error('Could not create agent', err);
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void create();
    }
  }
</script>

<Dialog title="New agent" width={560} onclose={() => (ui.dialog = null)}>
  <form class="form" id="new-agent" onsubmit={create}>
    <div class="row2">
      <label class="field">
        <span>Project</span>
        <select class="select" bind:value={target}>
          {#each fleet.hostList.filter((h) => h.online) as h (h.id)}
            {@const ps = onlineProjects.filter((p) => p.hostId === h.id)}
            {#if ps.length}
              <optgroup label={h.name}>
                {#each ps as p (p.id)}<option value="{p.hostId}:{p.id}">{p.name}</option>{/each}
              </optgroup>
            {/if}
          {/each}
          <option value={CUSTOM}>Other directory…</option>
        </select>
      </label>
      <label class="field">
        <span>Host</span>
        {#if project}
          <input class="input" value={fleet.host(project.hostId)?.name ?? project.hostId} disabled />
        {:else}
          <select class="select" bind:value={customHost}>
            {#each fleet.hostList.filter((h) => h.online) as h (h.id)}<option value={h.id}>{h.name}</option>{/each}
          </select>
        {/if}
      </label>
    </div>
    {#if !project}
      <label class="field">
        <span>Working directory</span>
        <input class="input mono" bind:value={cwd} placeholder="/home/me/src/repo" spellcheck="false" />
      </label>
    {/if}
    <label class="field">
      <span>Task</span>
      <!-- svelte-ignore a11y_autofocus -->
      <textarea class="textarea" rows="5" bind:value={prompt} {onkeydown} placeholder="What should the agent do? (optional — you can prompt later)" autofocus></textarea>
    </label>
    <label class="field">
      <span>Title <span class="faint">(optional)</span></span>
      <input class="input" bind:value={title} placeholder="Derived from the task when empty" />
    </label>

    <div class="wt" class:disabled={!isGit}>
      <label class="switch">
        <input type="checkbox" bind:checked={worktree} disabled={!isGit} />
        <span><GitBranch size={13} /> Run in a new git worktree</span>
      </label>
      {#if !isGit && project}<span class="hint">Not a git repository</span>{/if}
      {#if worktree && isGit}
        <div class="row2">
          <label class="field">
            <span>Branch</span>
            <input class="input mono" bind:value={branch} placeholder="coocoo/&lt;agent-id&gt;" spellcheck="false" />
          </label>
          <label class="field">
            <span>Base</span>
            <input class="input mono" bind:value={base} placeholder={project?.git?.branch ?? 'HEAD'} spellcheck="false" />
          </label>
        </div>
      {/if}
    </div>

    <div class="row2">
      <div class="field">
        <span>Model</span>
        <ModelPicker {models} value={model} onselect={(v) => (model = v)} defaultLabel="Default" variant="field" />
        {#if modelsError}<span class="hint">{modelsError}</span>{/if}
      </div>
      <label class="field">
        <span>Thinking</span>
        <select class="select" bind:value={thinking}>
          <option value="">Default</option>
          {#each THINKING as t (t)}<option value={t}>{t}</option>{/each}
        </select>
      </label>
    </div>
  </form>
  {#snippet footer()}
    <span class="hint grow">⌘↵ to create</span>
    <button class="btn" type="button" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="new-agent" disabled={busy}>
      {#if busy}<span class="spinner"></span>{/if} Create agent
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
  .wt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
  }
  .wt.disabled {
    opacity: 0.7;
  }
  .switch span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .grow {
    margin-right: auto;
    align-self: center;
  }
  textarea {
    min-height: 96px;
  }
  @media (max-width: 640px) {
    .row2 {
      grid-template-columns: 1fr;
    }
  }
</style>
