<script lang="ts">
  import { openExternal } from '../lib/external';
  import { PR_MARK, prSeverity, prSummary } from '../lib/git.svelte';
  import type { PullRequestInfo } from '../lib/protocol';

  // A pull request's number with its state as colour and mark (`#42 ✓`); neutral when only the
  // number is known. With `url` it is a link that opens outside Coocoo.
  let { pr, number, url, prefix = '' }: { pr?: PullRequestInfo; number?: number; url?: string; prefix?: string } = $props();

  const sev = $derived(pr ? prSeverity(pr) : undefined);
  const mark = $derived(sev ? PR_MARK[sev] : '');
  const text = $derived(`${prefix}#${number ?? pr?.number}${mark ? ` ${mark}` : ''}`);
  const title = $derived(pr ? prSummary(pr) : `Pull request #${number}`);
</script>

{#if url}
  <button type="button" class="prb mono" data-sev={sev ?? 'none'} {title} onclick={() => openExternal(url)}>{text}</button>
{:else}
  <span class="prb mono" data-sev={sev ?? 'none'} {title}>{text}</span>
{/if}

<style>
  .prb {
    display: inline-flex;
    flex: none;
    align-items: center;
    height: 16px;
    padding: 0 6px;
    border: 0;
    border-radius: 99px;
    background: var(--bg-3);
    color: var(--text-2);
    font-size: 10.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  button.prb {
    cursor: pointer;
  }
  button.prb:hover {
    text-decoration: underline;
  }
  [data-sev='ok'] {
    background: var(--green-soft);
    color: var(--green);
  }
  [data-sev='pending'] {
    background: var(--amber-soft);
    color: var(--amber);
  }
  [data-sev='bad'] {
    background: var(--red-soft);
    color: var(--red);
  }
  [data-sev='merged'] {
    background: var(--violet-soft);
    color: var(--violet);
  }
  [data-sev='closed'] {
    text-decoration: line-through;
  }
</style>
