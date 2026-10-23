<script lang="ts">
  import type { TimelineImage, TimelineItem } from '../lib/protocol';
  import { ChevronLeft, ChevronRight, Image as ImageIcon, MousePointerClick } from '../lib/icons';
  import { bytes } from '../lib/format';
  import { files, loadAgentImage } from '../lib/files.svelte';
  import Dialog from '../components/Dialog.svelte';

  // `pending`/`failed`: a provisional prompt from the outbox (fleet.sendPrompt), not yet in omp's
  // transcript; `failed` is the error shown with Retry/Edit/Discard. Its images come as data URLs
  // (`local`); stored ones are fetched from the agent (`host`/`agentId`).
  let {
    item,
    host,
    agentId,
    local,
    pending = false,
    failed,
    onretry,
    onedit,
    ondiscard,
  }: {
    item: TimelineItem;
    host?: string;
    agentId?: string;
    local?: string[];
    pending?: boolean;
    failed?: string;
    onretry?: () => void;
    onedit?: () => void;
    ondiscard?: () => void;
  } = $props();

  // Messages from before images were stored end in `[N image(s)]` (mapper.rs `content_text`).
  const LEGACY = /(?:^|\n)\[(\d+) images?\]$/;
  const images = $derived(item.images ?? []);
  const legacy = $derived(images.length ? null : LEGACY.exec(item.text ?? ''));
  const legacyCount = $derived(legacy ? Number(legacy[1]) : 0);
  const text = $derived(legacy ? (item.text ?? '').slice(0, legacy.index) : (item.text ?? ''));

  const keys = $derived(local || !host || !agentId ? [] : images.map((img) => loadAgentImage(host, agentId, img)));
  /** Data URL of image `i`: `undefined` while loading, `''` when it could not be loaded. */
  const src = (i: number): string | undefined => (local ? local[i] : i < keys.length ? files[keys[i]] : '');
  const size = (img: TimelineImage) => (img.width && img.height ? `${img.width} × ${img.height} · ` : '') + bytes(img.size);
  const format = (img: TimelineImage) => img.mime.replace(/^image\//, '').toUpperCase();

  /** Image open in the dialog. */
  let view = $state<number | null>(null);
  const step = (d: number) => view !== null && (view = (view + d + images.length) % images.length);
  function onkeydown(e: KeyboardEvent) {
    if (view === null || images.length < 2 || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    e.preventDefault();
    step(e.key === 'ArrowLeft' ? -1 : 1);
  }

  /** Text split into plain runs and design-mode element blocks (see Composer.buildMessage). */
  type Part = { kind: 'text'; text: string } | { kind: 'element'; selector: string; body: string };

  const ELEMENT = /<selected-element url="[^"]*" selector="([^"]*)">\n?([\s\S]*?)\n?<\/selected-element>/g;
  const LONG = 900;

  const parts = $derived.by<Part[]>(() => {
    const out: Part[] = [];
    let last = 0;
    for (const m of text.matchAll(ELEMENT)) {
      const before = text.slice(last, m.index).trim();
      if (before) out.push({ kind: 'text', text: before });
      out.push({ kind: 'element', selector: m[1], body: m[2] });
      last = m.index + m[0].length;
    }
    const rest = text.slice(last).trim();
    if (rest) out.push({ kind: 'text', text: rest });
    return out;
  });

  let expanded = $state(false);
  let openEl = $state<Record<number, boolean>>({});
  const long = $derived(text.length > LONG && parts.length === 1);
</script>

<svelte:window {onkeydown} />

<div class="user" class:pending class:failed={failed !== undefined} aria-busy={pending || undefined}>
  <div class="bubble">
    {#if images.length}
      <div class="imgs" class:many={images.length > 1}>
        {#each images as img, i (img.name)}
          {@const s = src(i)}
          <button class="tile" onclick={() => (view = i)} title="Open image">
            <span class="th">
              {#if s}
                <img src={s} alt="Image {i + 1}" />
              {:else if s === undefined}
                <span class="spinner"></span>
              {:else}
                <span class="faint">Not available</span>
              {/if}
            </span>
            <span class="info"><ImageIcon size={12} /><span class="ellipsis">{size(img)}</span><span class="fmt">{format(img)}</span></span>
          </button>
        {/each}
      </div>
    {/if}
    {#each parts as p, i (i)}
      {#if p.kind === 'text'}
        <div class="txt" class:clamp={long && !expanded}>{p.text}</div>
      {:else}
        <button class="el" onclick={() => (openEl[i] = !openEl[i])} aria-expanded={!!openEl[i]}>
          <MousePointerClick size={12} /><span class="mono ellipsis">{p.selector}</span>
        </button>
        {#if openEl[i]}<pre class="mono elbody">{p.body}</pre>{/if}
      {/if}
    {/each}
    {#if long}<button class="more" onclick={() => (expanded = !expanded)}>{expanded ? 'Show less' : 'Show more'}</button>{/if}
    {#if legacyCount}
      <span class="legacy"><ImageIcon size={12} />{legacyCount} image{legacyCount === 1 ? '' : 's'} · not shown</span>
    {/if}
  </div>
  {#if failed !== undefined}
    <span class="err" role="alert">{failed}</span>
    <div class="actions">
      <button class="btn sm retry" onclick={onretry}>Retry</button>
      <button class="btn sm" onclick={onedit}>Edit</button>
      <button class="btn sm" onclick={ondiscard}>Discard</button>
    </div>
  {:else if pending}
    <span class="meta faint"><span class="spinner"></span>Sending…</span>
  {:else}
    <time class="faint" datetime={new Date(item.ts).toISOString()}>{new Date(item.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
  {/if}
</div>

{#if view !== null && images[view]}
  {@const img = images[view]}
  {@const s = src(view)}
  <Dialog title={images.length > 1 ? `Image ${view + 1} of ${images.length}` : 'Image'} width={1200} onclose={() => (view = null)}>
    {#if s}
      <img class="full" src={s} alt="Image {view + 1}" />
    {:else if s === undefined}
      <span class="spinner"></span>
    {:else}
      <span class="faint">Not available</span>
    {/if}
    {#snippet footer()}
      <span class="faint dinfo">{size(img)} · {format(img)}{local ? '' : ' · as the agent got it'}</span>
      {#if images.length > 1}
        <button class="btn ghost icon sm" onclick={() => step(-1)} aria-label="Previous image" title="Previous (←)"><ChevronLeft size={15} /></button>
        <button class="btn ghost icon sm" onclick={() => step(1)} aria-label="Next image" title="Next (→)"><ChevronRight size={15} /></button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .user {
    align-self: flex-end;
    max-width: min(85%, 680px);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 3px;
  }
  .bubble {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    max-width: 100%;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 10px 10px 3px 10px;
    background: var(--bg-2);
    transition:
      opacity var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease);
  }
  .pending .bubble {
    opacity: 0.6;
    border-style: dashed;
  }
  .failed .bubble {
    border-color: color-mix(in srgb, var(--red) 55%, var(--border));
    background: var(--red-soft);
  }
  .txt {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.55;
  }
  .txt.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 10;
    line-clamp: 10;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .el {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg);
    color: var(--accent-text);
    font-size: 11px;
    cursor: pointer;
  }
  .elbody {
    max-width: 100%;
    max-height: 240px;
    overflow: auto;
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--r);
    background: var(--code-bg);
    font-size: 11px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .more {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  time,
  .meta,
  .err {
    font-size: 10.5px;
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .meta .spinner {
    width: 10px;
    height: 10px;
    border-width: 1.5px;
  }
  .err {
    color: var(--red);
    overflow-wrap: anywhere;
    text-align: right;
  }
  .actions {
    display: flex;
    gap: 4px;
  }
  .actions .btn {
    height: 22px;
    font-size: var(--fs-xs);
  }
  .retry {
    color: var(--accent-text);
  }
  .imgs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-self: stretch;
  }
  .tile {
    display: flex;
    flex-direction: column;
    width: 216px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg);
    overflow: hidden;
    color: var(--text-2);
    font: inherit;
    text-align: left;
    cursor: zoom-in;
    transition: border-color var(--t-fast) var(--ease);
  }
  .tile:hover {
    border-color: var(--accent);
  }
  .th {
    display: grid;
    place-items: center;
    height: 132px;
    overflow: hidden;
    background: var(--bg-3);
    font-size: var(--fs-xs);
  }
  .th img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top;
  }
  .info {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 7px;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .info :global(svg) {
    flex: none;
  }
  .fmt {
    flex: none;
    margin-left: auto;
    padding: 0 5px;
    border-radius: 3px;
    background: var(--bg-3);
    color: var(--text-3);
    font-size: 10px;
    letter-spacing: 0.03em;
  }
  .many .tile {
    width: 126px;
  }
  .many .th {
    height: 88px;
  }
  .many .fmt,
  .many .info :global(svg) {
    display: none;
  }
  .legacy {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 8px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r);
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .full {
    display: block;
    width: 100%;
    /* The dialog is at most 80vh tall; header and footer take the rest. */
    max-height: calc(80vh - 130px);
    object-fit: contain;
    border-radius: var(--r);
  }
  .dinfo {
    margin-right: auto;
    align-self: center;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
</style>
