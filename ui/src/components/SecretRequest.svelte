<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fleet } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { SecretUiPayload, UiRequest } from '../lib/protocol';
  import { Check, ShieldCheck } from '../lib/icons';
  import SecretInput from './SecretInput.svelte';

  // Body of a `secret` request card: what the agent wants and where it goes, the masked input
  // and Save. The value goes to the daemon with `secrets.provide` and lives only in this
  // component until then (cleared after saving and when the card goes away).
  let {
    request,
    host,
    compact = false,
    ondone,
  }: { request: UiRequest; host: string; compact?: boolean; ondone: () => void } = $props();

  const p = $derived(request.payload as SecretUiPayload);
  let value = $state('');
  let busy = $state(false);
  onDestroy(() => (value = ''));
  // On touch screens a focused field pops up the keyboard before the card is even read (and
  // squeezes the transcript away), so the field waits for a tap there.
  const touch = matchMedia('(pointer: coarse)').matches;

  async function save() {
    if (busy || !value) return;
    busy = true;
    try {
      await fleet.call('secrets.provide', { id: request.agentId, requestId: request.id, value }, host, 60_000);
      value = '';
      ondone();
    } catch (err) {
      ui.error('Could not save the secret', err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="title"><code class="name">{p.name}</code> <span class="muted">for project</span> <strong>{p.projectName}</strong></div>
<blockquote class="why">{p.description}</blockquote>
<ul class="targets">
  <li><Check size={12} /><span>Environment variable <code>${p.name}</code> for all agents of this project</span></li>
  {#if p.file}
    <li>
      <Check size={12} />
      <span>
        {#if p.file.format === 'raw'}File <code>{p.file.path}</code> (the whole content){:else}Line <code>{p.name}=…</code> in <code>{p.file.path}</code>{/if}
        {#if p.worktree && p.branch}(worktree <code>{p.branch}</code>){/if}
      </span>
    </li>
  {/if}
</ul>
{#if p.file?.exclude}
  <div class="hint">{p.file.path} is not ignored; Coocoo adds it to .git/info/exclude so no commit picks it up.</div>
{/if}
{#if p.replace}
  <div class="hint">Replaces the value saved before, also in the files it was written into.</div>
{/if}
<SecretInput bind:value multiline={p.multiline} autofocus={!compact && !touch} onsubmit={save} />
{#if !compact}
  <div class="ok">
    <ShieldCheck size={14} />
    <span>The value goes straight to Coocoo, not to the agent. If the agent reads the file, the model only sees a placeholder. Exception: a deliberately re-encoded value (e.g. base64).</span>
  </div>
{/if}
<div class="actions">
  <span class="hint">{p.multiline ? '⌘↵' : '↵'} to save</span>
  <button class="btn primary sm" disabled={!value || busy} onclick={save}>
    {#if busy}<span class="spinner"></span>{/if}
    Save secret
  </button>
</div>

<style>
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 6px;
  }
  .name {
    font-family: var(--mono);
    font-weight: 600;
  }
  .why {
    margin: 0;
    color: var(--text-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .why::before {
    content: '“';
  }
  .why::after {
    content: '”';
  }
  .targets {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .targets li {
    display: flex;
    align-items: baseline;
    gap: 7px;
  }
  .targets li :global(svg) {
    flex: none;
    color: var(--green);
    transform: translateY(1px);
  }
  code {
    font-family: var(--mono);
    font-size: 0.92em;
    overflow-wrap: anywhere;
  }
  .ok {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 7px 10px;
    border: 1px solid color-mix(in srgb, var(--green) 30%, var(--border));
    border-radius: var(--r);
    background: var(--green-soft);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .ok :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--green);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 6px;
  }
  .actions .hint {
    margin-right: auto;
  }
</style>
