<script lang="ts">
  import type { Snippet } from 'svelte';
  import { X } from '../lib/icons';

  let {
    title,
    width = 480,
    onclose,
    children,
    footer,
  }: { title: string; width?: number; onclose: () => void; children: Snippet; footer?: Snippet } = $props();

  let box: HTMLDivElement;

  $effect(() => {
    const prev = document.activeElement as HTMLElement | null;
    const first = box.querySelector<HTMLElement>('[autofocus], input:not([type=hidden]), textarea, select, button.primary');
    first?.focus();
    return () => prev?.focus?.();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onmousedown={(e) => e.target === e.currentTarget && onclose()} {onkeydown}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} style:max-width="{width}px" bind:this={box}>
    <header>
      <h2>{title}</h2>
      <button class="btn ghost icon sm" onclick={onclose} aria-label="Close"><X size={15} /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
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
    max-height: 80vh;
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
    padding: 14px 14px 6px 18px;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .body {
    padding: 8px 18px 16px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
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
      padding: 0;
      align-items: flex-end;
    }
    .dialog {
      max-width: none !important;
      max-height: 92vh;
      border-radius: 12px 12px 0 0;
    }
  }
</style>
