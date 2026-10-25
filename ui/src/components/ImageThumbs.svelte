<script lang="ts" module>
  /** One image shown: `src` is `undefined` while it loads, `''` when it cannot be loaded. */
  export interface Thumb {
    key: string;
    name: string;
    src: string | undefined;
  }
</script>

<script lang="ts">
  import { imageFiles } from '../lib/images';
  import { Image, X } from '../lib/icons';
  import Dialog from './Dialog.svelte';

  // Attached images as thumbnails (click opens one large), each removable, and an "Add image"
  // tile that picks files; reading, checking and storing them is the parent's.
  let {
    images,
    max,
    accept = 'image/*',
    onpick,
    onremove,
  }: {
    images: Thumb[];
    /** Hides the "Add image" tile once reached. */
    max?: number;
    accept?: string;
    onpick: (files: File[]) => void;
    onremove: (key: string) => void;
  } = $props();

  let fileInput = $state<HTMLInputElement>();
  let view = $state<{ name: string; src: string } | null>(null);
</script>

<div class="imgs">
  {#each images as im (im.key)}
    <div class="thumb">
      <button type="button" class="open" onclick={() => im.src && (view = { name: im.name, src: im.src })} title="Open {im.name}" aria-label="Open {im.name}">
        {#if im.src}<img src={im.src} alt={im.name} />{:else if im.src === undefined}<span class="spinner"></span>{:else}<span class="faint">?</span>{/if}
      </button>
      <button type="button" class="rm" onclick={() => onremove(im.key)} title="Remove image" aria-label="Remove {im.name}"><X size={11} /></button>
    </div>
  {/each}
  {#if max === undefined || images.length < max}
    <button type="button" class="addimg" onclick={() => fileInput?.click()}><Image size={14} />Add image</button>
  {/if}
  <input
    type="file"
    {accept}
    multiple
    hidden
    bind:this={fileInput}
    onchange={(e) => {
      onpick(imageFiles(e.currentTarget.files));
      e.currentTarget.value = '';
    }}
  />
</div>

{#if view}
  <Dialog title={view.name} width={1200} onclose={() => (view = null)}>
    <img class="full" src={view.src} alt={view.name} />
  </Dialog>
{/if}

<style>
  .imgs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .thumb {
    position: relative;
    width: 76px;
    height: 58px;
  }
  .open {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg-2);
    cursor: zoom-in;
  }
  .open img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .open .spinner {
    width: 12px;
    height: 12px;
  }
  .rm {
    position: absolute;
    top: 3px;
    right: 3px;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.7);
    color: #fff;
    cursor: pointer;
  }
  .addimg {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    width: 76px;
    height: 58px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-3);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .addimg:hover {
    border-color: var(--text-3);
    color: var(--text);
  }
  .full {
    display: block;
    width: 100%;
    max-height: 75vh;
    object-fit: contain;
  }
</style>
