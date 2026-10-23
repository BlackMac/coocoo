<script lang="ts">
  import { renderMarkdown } from '../lib/markdown';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import type { Attachment, Card } from '../lib/protocol';
  import { NotebookPen } from '../lib/icons';
  import Dialog from '../components/Dialog.svelte';
  import { cardFiles as loaded, loadCardFile } from '../lib/cardfiles.svelte';

  // A card's refinement: the agent's Markdown plan and its mocks (images, HTML). HTML mocks run
  // in sandboxed frames without the app's origin, so their scripts cannot reach the token.
  // `clamp`: show the text cut to a few lines with "Show all" (cards on the board are narrow).
  let { host, projectId, card, clamp = false }: { host: string; projectId: string; card: Card; clamp?: boolean } = $props();
  let expanded = $state(false);
  let mdEl = $state<HTMLDivElement>();
  let overflows = $state(false);
  $effect(() => {
    void html;
    if (mdEl) overflows = mdEl.scrollHeight > mdEl.clientHeight + 1;
  });

  const r = $derived(card.refinement!);
  const html = $derived(renderMarkdown(r.text, `refinement:${card.id}:${r.at}`));
  const keys = $derived((r.attachments ?? []).map((a) => ({ a, k: loadCardFile(host, projectId, card.id, 'attachment', a, r.at) })));
  let view = $state<{ a: Attachment; k: string } | null>(null);
  const isHtml = (a: Attachment) => a.mime === 'text/html';
</script>

<div class="refinement">
  <div class="rhead">
    <NotebookPen size={12} /><strong>Refinement</strong>
    <span class="faint ellipsis">{r.agentTitle ? `by ${r.agentTitle} · ` : ''}{ago(r.at, now.value)} ago</span>
  </div>
  <div class="md" class:clamped={clamp && !expanded} class:fade={clamp && !expanded && overflows} bind:this={mdEl}>{@html html}</div>
  {#if clamp && (overflows || expanded)}
    <button class="more" onclick={() => (expanded = !expanded)}>{expanded ? 'Show less' : 'Show all'}</button>
  {/if}
  {#if keys.length}
    <div class="atts">
      {#each keys as { a, k } (a.name)}
        <button class="att" onclick={() => (view = { a, k })} title="Open {a.name}">
          <span class="thumb">
            {#if !(k in loaded)}
              <span class="spinner"></span>
            {:else if !loaded[k]}
              <span class="faint">Not available</span>
            {:else if isHtml(a)}
              <iframe class="mini" srcdoc={loaded[k]} sandbox="allow-scripts" title={a.name} tabindex="-1" inert></iframe>
            {:else}
              <img src={loaded[k]} alt={a.name} />
            {/if}
          </span>
          <span class="aname ellipsis">{a.name}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

{#if view}
  <Dialog title={view.a.name} width={1200} onclose={() => (view = null)}>
    {#if isHtml(view.a)}
      <iframe class="full" srcdoc={loaded[view.k]} sandbox="allow-scripts allow-forms" title={view.a.name}></iframe>
    {:else}
      <img class="full" src={loaded[view.k]} alt={view.a.name} />
    {/if}
  </Dialog>
{/if}

<style>
  .refinement {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--r-lg);
    background: var(--bg-1);
  }
  .rhead {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-sm);
  }
  .rhead :global(svg) {
    flex: none;
    color: var(--accent);
  }
  .rhead .faint {
    font-size: var(--fs-xs);
  }
  .md {
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  .md.clamped {
    max-height: 180px;
    overflow: hidden;
  }
  .md.fade {
    mask-image: linear-gradient(to bottom, #000 70%, transparent);
  }
  .more {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .more:hover {
    text-decoration: underline;
  }
  .atts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .att {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 168px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg);
    color: var(--text-2);
    font-size: var(--fs-xs);
    text-align: left;
    cursor: zoom-in;
  }
  .att:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .thumb {
    position: relative;
    display: grid;
    place-items: center;
    height: 100px;
    overflow: hidden;
    border-radius: var(--r-sm);
    background: var(--bg-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top;
  }
  /* The page laid out at 800 px wide, shown at a fifth of that. */
  .mini {
    position: absolute;
    top: 0;
    left: 0;
    width: 800px;
    height: 500px;
    border: 0;
    background: #fff;
    transform: scale(0.2);
    transform-origin: 0 0;
    pointer-events: none;
  }
  .aname {
    padding: 0 2px;
  }
  .full {
    display: block;
    width: 100%;
    border: 0;
    border-radius: var(--r);
  }
  iframe.full {
    height: min(75vh, 800px);
    background: #fff;
  }
  img.full {
    max-height: 75vh;
    object-fit: contain;
  }
</style>
