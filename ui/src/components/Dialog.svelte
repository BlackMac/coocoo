<script lang="ts">
  import type { Snippet } from 'svelte';
  import { X } from '../lib/icons';

  let {
    title,
    width = 480,
    fill = false,
    onclose,
    children,
    actions,
    footer,
  }: {
    title: string;
    width?: number;
    /** Take the whole window (viewers for images and mocks); `width` is ignored. */
    fill?: boolean;
    onclose: () => void;
    children: Snippet;
    /** Buttons in the header, left of Close. */
    actions?: Snippet;
    footer?: Snippet;
  } = $props();

  let overlay: HTMLDivElement;
  let box: HTMLDivElement;

  $effect(() => {
    // Rendered at the end of `<body>`: an ancestor with `content-visibility`, `contain` or
    // `container-type` (the transcript's items, the board) would clip a fixed overlay and place
    // it inside itself. Moved before focusing, since moving a node takes the focus from it.
    // The overlay sits in a wrapper that stays in place, so Svelte can remove its block as usual.
    document.body.appendChild(overlay);
    const prev = document.activeElement as HTMLElement | null;
    const first = box.querySelector<HTMLElement>('[autofocus], input:not([type=hidden]), textarea, select, button.primary');
    // On touch screens a focused field pops up the keyboard before the form is even seen, so the
    // dialog itself takes focus there (also taking it from an `autofocus` field). Without a
    // field, the dialog takes focus so Escape reaches it.
    const touch = matchMedia('(pointer: coarse)').matches;
    (touch ? box : (first ?? box)).focus({ preventScroll: true });
    return () => {
      overlay.remove();
      prev?.focus?.({ preventScroll: true });
    };
  });

  // The keyboard sliding in shrinks the dialog: keep the field being typed in visible.
  $effect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const reveal = () => {
      const el = document.activeElement;
      if (el instanceof HTMLElement && el !== box && box.contains(el)) el.scrollIntoView({ block: 'nearest' });
    };
    vv.addEventListener('resize', reveal);
    return () => vv.removeEventListener('resize', reveal);
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    }
  }
</script>

<div class="anchor">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" class:fill bind:this={overlay} onmousedown={(e) => e.target === e.currentTarget && onclose()} {onkeydown}>
    <div class="dialog" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" style:max-width={fill ? null : `${width}px`} bind:this={box}>
      <header>
        <h2 class="ellipsis">{title}</h2>
        <div class="hbtns">
          {#if actions}{@render actions()}{/if}
          <button class="btn ghost icon sm" onclick={onclose} aria-label="Close" title="Close (Esc)"><X size={15} /></button>
        </div>
      </header>
      <div class="body">{@render children()}</div>
      {#if footer}<footer>{@render footer()}</footer>{/if}
    </div>
  </div>
</div>

<style>
  /* Holds the overlay's place in the component tree once the overlay moved to `<body>`. */
  .anchor {
    display: contents;
  }
  /* As tall as the visible viewport (`--app-height`, set in main.ts), not the layout viewport:
     with the on-screen keyboard up, a dialog must fit above it and scroll inside. */
  .overlay {
    position: fixed;
    inset: 0 0 auto;
    height: var(--app-height, 100%);
    z-index: 50;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 12vh 16px 16px;
    background: var(--overlay);
    animation: fade var(--t) var(--ease);
  }
  .dialog {
    width: 100%;
    outline: none;
    max-height: min(80vh, 100%);
    display: flex;
    flex-direction: column;
    border-radius: 10px;
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    animation: rise var(--t) var(--ease);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 14px 14px 6px 18px;
  }
  h2 {
    min-width: 0;
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .hbtns {
    display: flex;
    flex: none;
    align-items: center;
    gap: 2px;
  }
  .body {
    padding: 8px 18px 16px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .overlay.fill {
    padding: 16px;
  }
  .fill .dialog {
    height: 100%;
    max-height: none;
  }
  .fill .body {
    flex: 1;
    min-height: 0;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.99);
    }
  }
  @media (max-width: 640px) {
    .overlay {
      padding: env(safe-area-inset-top) 0 0;
      align-items: flex-end;
    }
    .dialog {
      max-width: none !important;
      /* Percent of the overlay, which shrinks with the keyboard (vh does not on iOS). */
      max-height: 92%;
      border-radius: 12px 12px 0 0;
    }
    /* Without the keyboard the sheet ends at the screen's bottom edge. */
    footer {
      padding-bottom: max(12px, env(safe-area-inset-bottom));
    }
    .overlay.fill {
      padding: env(safe-area-inset-top) 0 0;
    }
    .fill .body {
      padding: 4px 8px max(8px, env(safe-area-inset-bottom));
    }
  }
</style>
