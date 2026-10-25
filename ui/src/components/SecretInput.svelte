<script lang="ts">
  import { Eye, EyeOff, TriangleAlert } from '../lib/icons';

  // Masked entry of a secret value (password field, or a masked text area for PEM/JSON) with a
  // show/hide toggle. The value only lives in the parent's local state: never prefilled, no
  // autocomplete, no password-manager capture.
  let {
    value = $bindable(''),
    multiline = false,
    autofocus = false,
    onsubmit,
  }: { value?: string; multiline?: boolean; autofocus?: boolean; onsubmit?: () => void } = $props();

  let shown = $state(false);
  /** omp ignores shorter values, so they reach the model unmasked. */
  const short = $derived(value.length > 0 && [...value].length < 8);

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onsubmit?.();
    }
  }
</script>

<div class="wrap">
  <div class="row">
    {#if multiline}
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="textarea mono"
        class:masked={!shown}
        rows="5"
        bind:value
        {autofocus}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        data-1p-ignore
        data-lpignore="true"
        aria-label="Secret value"
        placeholder="Paste the value"
        {onkeydown}
      ></textarea>
    {:else}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input mono"
        type={shown ? 'text' : 'password'}
        bind:value
        {autofocus}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        data-1p-ignore
        data-lpignore="true"
        aria-label="Secret value"
        {onkeydown}
      />
    {/if}
    <button
      type="button"
      class="btn icon toggle"
      title={shown ? 'Hide the value' : 'Show the value'}
      aria-label={shown ? 'Hide the value' : 'Show the value'}
      aria-pressed={shown}
      onclick={() => (shown = !shown)}
    >
      {#if shown}<EyeOff size={14} />{:else}<Eye size={14} />{/if}
    </button>
  </div>
  {#if short}
    <div class="warn"><TriangleAlert size={13} /><span>omp cannot mask values shorter than 8 characters.</span></div>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }
  .row > :first-child {
    flex: 1;
    min-width: 0;
  }
  .toggle {
    flex: none;
    width: 30px;
    height: 30px;
  }
  .masked {
    -webkit-text-security: disc;
  }
  .warn {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 7px 10px;
    border: 1px solid color-mix(in srgb, var(--amber) 35%, var(--border));
    border-radius: var(--r);
    background: var(--amber-soft);
    color: var(--amber);
    font-size: var(--fs-sm);
  }
  .warn :global(svg) {
    flex: none;
    margin-top: 2px;
  }
</style>
