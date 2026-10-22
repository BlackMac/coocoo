<script lang="ts">
  import type { Dialog as DialogState } from '../lib/ui.svelte';
  import { ui } from '../lib/ui.svelte';
  import Dialog from './Dialog.svelte';

  let { dialog }: { dialog: Extract<DialogState, { kind: 'confirm' }> } = $props();
  let busy = $state(false);

  async function confirm() {
    busy = true;
    try {
      await dialog.run();
      ui.dialog = null;
    } catch (err) {
      ui.error(`${dialog.action} failed`, err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={dialog.title} width={420} onclose={() => (ui.dialog = null)}>
  <p class="msg">{dialog.message}</p>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" class:danger={dialog.danger} class:solid={dialog.danger} disabled={busy} onclick={confirm}>
      {#if busy}<span class="spinner"></span>{/if}{dialog.action}
    </button>
  {/snippet}
</Dialog>

<style>
  .msg {
    margin: 0;
    color: var(--text-2);
    line-height: 1.55;
  }
</style>
