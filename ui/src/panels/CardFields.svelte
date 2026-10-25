<script lang="ts" module>
  import type { Attachment } from '../lib/protocol';
  import type { PickedImage } from '../lib/images';

  /** An image in the editor: one stored with the card, or one picked and not uploaded yet. */
  export type EditImage = { stored: Attachment } | { id: number; picked: PickedImage };

  export const MAX_IMAGES = 10;
  const MAX_BYTES = 5 << 20;
  const TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
  let seq = 0;
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import { ui } from '../lib/ui.svelte';
  import { base64Bytes, imageFiles, onImagePaste, readImage } from '../lib/images';
  import { files, loadCardFile } from '../lib/files.svelte';
  import { ChevronRight, Image } from '../lib/icons';
  import ImageThumbs from '../components/ImageThumbs.svelte';

  // The card editor (new card and opened card): title, a description that grows with its text,
  // context images (paste, drop, pick) and webhooks behind "More options". Ctrl/Cmd+Enter submits,
  // Esc cancels; the actions themselves are the parent's.
  let {
    host,
    projectId,
    cardId,
    title = $bindable(),
    body = $bindable(),
    startUrl = $bindable(),
    doneUrl = $bindable(),
    images = $bindable(),
    autofocus = false,
    onsubmit,
    oncancel,
  }: {
    host: string;
    projectId: string;
    /** The stored card, whose images are shown; none for a new card. */
    cardId?: string;
    title: string;
    body: string;
    startUrl: string;
    doneUrl: string;
    images: EditImage[];
    autofocus?: boolean;
    onsubmit: () => void;
    oncancel: () => void;
  } = $props();

  let titleEl = $state<HTMLInputElement>();
  let bodyEl = $state<HTMLTextAreaElement>();
  let dragOver = $state(false);
  let more = $state(!!(startUrl || doneUrl));

  $effect(() => {
    if (autofocus) titleEl?.focus();
  });

  /** The description grows with its text instead of scrolling inside itself. */
  function grow() {
    if (!bodyEl) return;
    bodyEl.style.height = 'auto';
    bodyEl.style.height = `${bodyEl.scrollHeight + 2}px`;
  }
  $effect(() => {
    void body;
    void tick().then(grow);
  });

  function add(im: PickedImage) {
    if (!TYPES.includes(im.image.mimeType)) return ui.toast('error', `${im.name}: not attached`, 'Only PNG, JPEG, GIF and WebP images can be attached.');
    if (base64Bytes(im.image.data) > MAX_BYTES) return ui.toast('error', `${im.name}: not attached`, 'Images can be at most 5 MB.');
    if (images.length >= MAX_IMAGES) return ui.toast('error', `${im.name}: not attached`, `A card can have at most ${MAX_IMAGES} images.`);
    images = [...images, { id: ++seq, picked: im }];
  }
  const addFiles = (files: File[]) => files.forEach((f) => void readImage(f).then((im) => im && add(im)));
  const onpaste = (e: ClipboardEvent) => onImagePaste(e, add);

  const src = (im: EditImage): string | undefined =>
    'picked' in im
      ? `data:${im.picked.image.mimeType};base64,${im.picked.image.data}`
      : cardId
        ? files[loadCardFile(host, projectId, cardId, 'image', im.stored, im.stored.size)]
        : undefined;
  const keyOf = (im: EditImage) => ('picked' in im ? `p${im.id}` : `s${im.stored.name}`);
  const thumbs = $derived(images.map((im) => ({ key: keyOf(im), name: 'picked' in im ? im.picked.name : im.stored.name, src: src(im) })));

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onsubmit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      oncancel();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="fields"
  class:drag={dragOver}
  {onkeydown}
  ondragover={(e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();
    dragOver = true;
  }}
  ondragleave={(e) => {
    if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dragOver = false;
  }}
  ondrop={(e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();
    dragOver = false;
    addFiles(imageFiles(e.dataTransfer.files));
  }}
>
  <input class="input title" bind:this={titleEl} bind:value={title} placeholder="Title" aria-label="Title" {onpaste} />
  <textarea class="textarea body" bind:this={bodyEl} bind:value={body} rows="8" placeholder="Details for the agent (optional)" aria-label="Details" {onpaste}></textarea>
  <ImageThumbs
    images={thumbs}
    max={MAX_IMAGES}
    accept={TYPES.join(',')}
    onpick={addFiles}
    onremove={(key) => (images = images.filter((im) => keyOf(im) !== key))}
  />
  <button class="more" onclick={() => (more = !more)} aria-expanded={more}>
    <span class="chev" class:on={more}><ChevronRight size={12} /></span>More options<span class="faint">{startUrl || doneUrl ? '(webhooks set)' : '(webhooks)'}</span>
  </button>
  {#if more}
    <label class="hook"><span class="faint">Webhook on start</span><input class="input mono" bind:value={startUrl} placeholder="https://… (POSTed when it moves to Doing)" spellcheck="false" /></label>
    <label class="hook"><span class="faint">Webhook on done</span><input class="input mono" bind:value={doneUrl} placeholder="https://… (POSTed when it moves to Done)" spellcheck="false" /></label>
  {/if}
  {#if dragOver}<div class="dropzone"><Image size={16} />Drop images to attach</div>{/if}
</div>

<style>
  .fields {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .title {
    font-weight: 500;
  }
  .body {
    min-height: 150px;
    overflow: hidden;
    resize: none;
    line-height: 1.5;
  }
  .more {
    display: flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .more:hover {
    color: var(--text);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--t-fast) var(--ease);
  }
  .chev.on {
    transform: rotate(90deg);
  }
  .hook {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--fs-xs);
  }
  .dropzone {
    position: absolute;
    inset: -4px;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 2px dashed var(--accent);
    border-radius: var(--r-lg);
    background: color-mix(in srgb, var(--bg-1) 80%, var(--accent));
    color: var(--text);
    font-weight: 500;
    pointer-events: none;
  }
</style>
