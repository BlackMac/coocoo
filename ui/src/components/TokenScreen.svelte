<script lang="ts">
  import { storeToken } from '../lib/bootstrap';
  import { KeyRound } from '../lib/icons';

  let { rejected, onsubmit }: { rejected: boolean; onsubmit: (token: string) => void } = $props();

  let value = $state('');

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const t = value.trim().replace(/^.*[#&?]token=/, '');
    if (!t) return;
    storeToken(t);
    onsubmit(t);
  }
</script>

<div class="wrap">
  <form class="box" onsubmit={submit}>
    <div class="logo" aria-hidden="true">
      <svg viewBox="0 0 32 32" width="40" height="40"><rect width="32" height="32" rx="8" fill="var(--accent)" /><circle cx="12.5" cy="16" r="5" fill="none" stroke="#fff" stroke-width="2.6" /><circle cx="20.5" cy="16" r="5" fill="none" stroke="#fff" stroke-width="2.6" opacity=".85" /></svg>
    </div>
    <h1>Connect to Coocoo</h1>
    <p class="muted">
      {#if rejected}
        The daemon rejected the access token. Paste a valid one to continue.
      {:else}
        Paste the access token or pairing link shown by <code>coocoo serve</code> or in the desktop app under Hosts &amp; Settings.
      {/if}
    </p>
    <label class="field">
      <span class="sr-only">Access token</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" bind:value placeholder="Token or http://…/#token=…" autocomplete="off" spellcheck="false" autofocus />
    </label>
    <button class="btn primary" type="submit" disabled={!value.trim()}><KeyRound size={14} /> Connect</button>
  </form>
</div>

<style>
  .wrap {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .box {
    width: min(380px, 100%);
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 14px;
    text-align: center;
  }
  .logo {
    display: flex;
    justify-content: center;
  }
  h1 {
    margin: 4px 0 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  p {
    margin: 0;
    line-height: 1.55;
  }
  .btn {
    height: 32px;
  }
  .input {
    height: 34px;
  }
</style>
