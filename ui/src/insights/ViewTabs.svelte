<script lang="ts">
  import { href } from '../lib/router.svelte';
  import { track } from '../lib/track';

  /** The page these tabs head. */
  let { current }: { current: 'overview' | 'stats' | 'workflows' } = $props();

  const tabs = [
    { id: 'overview', label: 'Overview', href: href({ name: 'overview' }) },
    { id: 'stats', label: 'Stats', href: href({ name: 'stats' }) },
    { id: 'workflows', label: 'Workflows', href: href({ name: 'workflows' }) },
  ] as const;
</script>

<nav class="tabs" aria-label="Overview pages">
  {#each tabs as t (t.id)}
    <a class="tab" class:on={t.id === current} href={t.href} aria-current={t.id === current ? 'page' : undefined} onclick={() => t.id !== current && track('tab', { meta: { tab: t.id } })}
      >{t.label}</a
    >
  {/each}
</nav>

<style>
  .tabs {
    display: flex;
    gap: 18px;
    min-width: 0;
  }
  .tab {
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--text-3);
    text-decoration: none;
    padding-bottom: 3px;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
  }
  .tab:hover {
    color: var(--text-2);
  }
  .tab.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  @media (max-width: 600px) {
    .tabs {
      gap: 14px;
    }
    .tab {
      font-size: var(--fs-lg);
    }
  }
</style>
