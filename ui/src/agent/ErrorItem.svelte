<script lang="ts">
  import type { ProviderError } from '../lib/providerError';
  import { Brain, CircleAlert, RotateCw } from '../lib/icons';

  // One failure in the transcript: the readable message, the raw response under "Details", and
  // for rate limits and quotas the ways out (`actions`: only on the agent's latest failed turn).
  let { error, actions }: { error: ProviderError; actions?: { model: () => void; retry: () => void } } = $props();
</script>

<div class="error" role="alert">
  <div class="head">
    <CircleAlert size={15} />
    <div class="text">
      <strong>{error.title}</strong>
      {#if error.message && error.message !== error.title}<span class="msg">{error.message}</span>{/if}
    </div>
  </div>
  {#if actions && error.limit}
    <div class="acts">
      <button class="btn sm" onclick={actions.model}><Brain size={13} />Choose another model</button>
      <button class="btn sm" onclick={actions.retry}><RotateCw size={13} />Retry</button>
    </div>
  {/if}
  {#if error.details.length}
    <details>
      <summary>Details</summary>
      <dl>
        {#each error.details as d (d.label + d.value)}
          <dt>{d.label}</dt>
          {#if d.code}<dd><pre class="mono">{d.value}</pre></dd>{:else}<dd class="mono">{d.value}</dd>{/if}
        {/each}
      </dl>
    </details>
  {/if}
</div>

<style>
  .error {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 9px 12px;
    border: 1px solid color-mix(in srgb, var(--red) 30%, var(--border));
    border-radius: var(--r-lg);
    background: var(--red-soft);
    color: var(--red);
    font-size: var(--fs-sm);
  }
  .head {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .head :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .msg {
    color: var(--text-2);
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-left: 23px;
  }
  details {
    padding-left: 23px;
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  summary {
    cursor: pointer;
    color: var(--text-3);
    width: max-content;
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 4px 12px;
    margin: 8px 0 0;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  pre {
    margin: 0;
    max-height: 240px;
    overflow: auto;
    padding: 6px 8px;
    border-radius: var(--r);
    background: var(--code-bg);
    font-size: 11px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
