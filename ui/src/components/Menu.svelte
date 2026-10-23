<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: import('svelte').Component<{ size?: number }>;
    danger?: boolean;
    disabled?: boolean;
    run: () => void;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let { items, label, trigger, align = 'right' }: { items: MenuItem[]; label: string; trigger: Snippet; align?: 'left' | 'right' } = $props();

  let open = $state(false);
  let root: HTMLDivElement;
  /** Opens above the button when there is no room below (e.g. the card bar above the composer). */
  let up = $state(false);

  function toggle() {
    if (!open) {
      const r = root.getBoundingClientRect();
      // Rough height of the list: rows of ~32 px plus padding.
      const need = items.length * 32 + 12;
      up = r.bottom + need > innerHeight && r.top > innerHeight - r.bottom;
    }
    open = !open;
  }

  $effect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        open = false;
      }
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', esc, true);
    };
  });
</script>

<div class="menu" bind:this={root}>
  <button class="btn ghost icon sm" class:active={open} aria-label={label} aria-haspopup="menu" aria-expanded={open} onclick={toggle}>
    {@render trigger()}
  </button>
  {#if open}
    <div class="pop" class:left={align === 'left'} class:up role="menu">
      {#each items as item (item.label)}
        <button
          role="menuitem"
          class:danger={item.danger}
          disabled={item.disabled}
          onclick={() => {
            open = false;
            item.run();
          }}
        >
          {#if item.icon}<item.icon size={14} />{/if}
          {item.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
  }
  .pop {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 40;
    min-width: 180px;
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    animation: pop var(--t-fast) var(--ease);
  }
  .pop.left {
    left: 0;
    right: auto;
  }
  .pop.up {
    top: auto;
    bottom: calc(100% + 4px);
  }
  button[role='menuitem'] {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    white-space: nowrap;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  button[role='menuitem']:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  button[role='menuitem']:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .danger {
    color: var(--red) !important;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
</style>
