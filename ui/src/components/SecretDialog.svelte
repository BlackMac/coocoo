<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import Dialog from './Dialog.svelte';
  import SecretInput from './SecretInput.svelte';

  // Create (without `name`) or replace a project's secret. The value stays in this component
  // until `secrets.set` took it and is cleared when the dialog closes.
  let { hostId, projectId, name: existing }: { hostId: string; projectId: string; name?: string } = $props();

  const NAME = /^[A-Z_][A-Z0-9_]*$/;
  let name = $state(untrack(() => existing ?? ''));
  let value = $state('');
  let multiline = $state(false);
  let path = $state('');
  let format = $state<'dotenv' | 'raw'>('dotenv');
  /** The user picked the format; stop guessing it from the file name. */
  let formatPicked = false;
  let busy = $state(false);
  onDestroy(() => (value = ''));

  const info = $derived(existing ? fleet.secrets[key(hostId, projectId)]?.find((s) => s.name === existing) : undefined);

  $effect(() => {
    const guess = /\.(json|pem|key|p12|crt)$/i.test(path.trim()) ? 'raw' : 'dotenv';
    if (!formatPicked) format = guess;
  });

  async function save(e?: SubmitEvent) {
    e?.preventDefault();
    if (busy) return;
    if (!NAME.test(name)) return ui.toast('error', 'Name: A–Z, 0–9 and _, not starting with a digit');
    if (!value) return ui.toast('error', 'Enter the value');
    busy = true;
    try {
      const file = path.trim() ? { file: { path: path.trim(), format } } : {};
      await fleet.call('secrets.set', { projectId, name, value, ...file }, hostId, 60_000);
      value = '';
      ui.submitDialog();
    } catch (err) {
      ui.error(existing ? 'Could not replace the secret' : 'Could not save the secret', err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={existing ? `Replace ${existing}` : 'New secret'} width={480} onclose={() => (ui.dialog = null)}>
  <form class="form" id="secret" onsubmit={save}>
    <label class="field">
      <span>Name (env var)</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input mono"
        value={name}
        oninput={(e) => (name = e.currentTarget.value.toUpperCase())}
        disabled={!!existing}
        placeholder="SENTRY_AUTH_TOKEN"
        autocomplete="off"
        autocapitalize="characters"
        spellcheck="false"
        autofocus={!existing}
      />
    </label>
    <div class="field">
      <div class="label-row">
        <span class="label">Value</span>
        <label class="switch small">
          <input type="checkbox" bind:checked={multiline} />
          <span class="hint">Multiline</span>
        </label>
      </div>
      <SecretInput bind:value {multiline} autofocus={!!existing} onsubmit={() => save()} />
    </div>
    {#if info?.files.length}
      <div class="hint">Also rewrites {info.files.map((f) => f.relPath + (f.branch ? ` (${f.branch})` : '')).join(', ')}.</div>
    {/if}
    <div class="file">
      <label class="field path">
        <span>Also write to file (optional, relative to the project)</span>
        <input class="input mono" bind:value={path} placeholder=".env" autocomplete="off" spellcheck="false" />
      </label>
      <label class="field fmt">
        <span>Format</span>
        <select class="select" bind:value={format} onchange={() => (formatPicked = true)} disabled={!path.trim()}>
          <option value="dotenv">NAME=… line</option>
          <option value="raw">Whole file</option>
        </select>
      </label>
    </div>
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="secret" disabled={busy || !value || !NAME.test(name)}>
      {#if busy}<span class="spinner"></span>{/if}
      Save
    </button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .file {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .file .path {
    flex: 2 1 220px;
    min-width: 0;
  }
  .file .fmt {
    flex: 1 1 120px;
  }
</style>
