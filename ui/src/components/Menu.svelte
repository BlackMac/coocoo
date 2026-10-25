<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { MenuItem } from '../lib/menus';
  import { keepInView } from '../lib/popover';

  let { items, label, trigger, align = 'right' }: { items: MenuItem[]; label: string; trigger: Snippet; align?: 'left' | 'right' } = $props();

  let open = $state(false);
  let root: HTMLDivElement;
  /**
   * Fixed position of the list, from the button's: inside a scrolling container (the sidebar's
   * tree) an absolutely positioned list would be clipped. Opens above the button when there is no
   * room below (e.g. the card bar above the composer).
   */
  let pos = $state('');

  function toggle() {
    if (!open) {
      const r = root.getBoundingClientRect();
      // Rough height of the list: rows of ~32 px plus padding.
      const need = items.length * 32 + 12;
      const up = r.bottom + need > innerHeight && r.top > innerHeight - r.bottom;
      const y = up ? `bottom:${innerHeight - r.top + 4}px` : `top:${r.bottom + 4}px`;
      const x = align === 'left' ? `left:${r.left}px` : `right:${document.documentElement.clientWidth - r.right}px`;
      pos = `${y};${x}`;
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
    // The list stays where it opened, so it closes when the button moves away under it: on
    // resizes and scrolls of a container of the button (not of others, like a streaming transcript).
    const dismiss = (e: Event) => {
      if (e.type === 'resize' || (e.target as Node).contains(root)) open = false;
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    document.addEventListener('scroll', dismiss, true);
    window.addEventListener('resize', dismiss);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', esc, true);
      document.removeEventListener('scroll', dismiss, true);
      window.removeEventListener('resize', dismiss);
    };
  });
</script>

<div class="menu" bind:this={root}>
  <button class="btn ghost icon sm" class:active={open} aria-label={label} aria-haspopup="menu" aria-expanded={open} onclick={toggle}>
    {@render trigger()}
  </button>
  {#if open}
    <div class="pop" style={pos} role="menu" use:keepInView>
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
    position: fixed;
    z-index: 40;
    min-width: 180px;
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    animation: pop var(--t-fast) var(--ease);
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
