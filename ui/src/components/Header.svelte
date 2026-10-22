<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { href, router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import { ChevronRight, Menu, Plus, Search } from '../lib/icons';
  import HostDot from './HostDot.svelte';

  let { onnew }: { onnew: () => void } = $props();

  interface Crumb {
    label: string;
    href?: string;
    online?: boolean;
  }

  const crumbs = $derived.by<Crumb[]>(() => {
    const r = router.route;
    if (r.name === 'overview') return [{ label: 'Overview' }];
    if (r.name === 'settings') return [{ label: 'Hosts & Settings' }];
    const h = fleet.host(r.host);
    const hostCrumb: Crumb = { label: h?.name ?? r.host, href: '#/settings', online: h?.online ?? false };
    if (r.name === 'project') {
      const p = fleet.project(r.host, r.id);
      return [hostCrumb, { label: p?.name ?? r.id }];
    }
    const a = fleet.agent(r.host, r.id);
    const p = a?.projectId ? fleet.project(r.host, a.projectId) : undefined;
    const out = [hostCrumb];
    if (p) out.push({ label: p.name, href: href({ name: 'project', host: r.host, id: p.id }) });
    out.push({ label: a?.title ?? r.id });
    return out;
  });
</script>

<header class="hdr">
  <button class="btn ghost icon menu" aria-label="Open navigation" onclick={() => (ui.drawerOpen = true)}><Menu size={16} /></button>
  <nav class="crumbs" aria-label="Breadcrumb">
    {#each crumbs as c, i (i)}
      {#if i > 0}<ChevronRight size={13} class="sep" />{/if}
      {#if c.href}
        <a href={c.href} class="crumb link">
          {#if c.online !== undefined}<HostDot online={c.online} />{/if}
          <span class="ellipsis">{c.label}</span>
        </a>
      {:else}
        <span class="crumb current ellipsis" aria-current="page">{c.label}</span>
      {/if}
    {/each}
  </nav>
  <button class="search" onclick={() => (ui.paletteOpen = true)}>
    <Search size={14} />
    <span class="ph">Search agents, projects, actions…</span>
    <span class="kbd">⌘K</span>
  </button>
  <button class="btn primary new" onclick={onnew} title="New agent (N)">
    <Plus size={15} /><span class="lbl">New agent</span>
  </button>
</header>

<style>
  .hdr {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--header-h);
    padding: 0 12px 0 16px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .menu {
    display: none;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
  }
  .crumbs :global(.sep) {
    flex: none;
    color: var(--text-3);
  }
  .crumb {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 3px 6px;
    border-radius: var(--r-sm);
  }
  .crumb.link {
    color: var(--text-2);
    flex-shrink: 1;
  }
  .crumb.link:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .crumb.current {
    font-weight: 500;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 280px;
    height: 28px;
    padding: 0 6px 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg-1);
    color: var(--text-3);
    cursor: pointer;
    transition: border-color var(--t-fast) var(--ease);
  }
  .search:hover {
    border-color: var(--border-strong);
    color: var(--text-2);
  }
  .ph {
    flex: 1;
    text-align: left;
    font-size: var(--fs-sm);
  }
  @media (max-width: 1100px) {
    .search {
      width: auto;
    }
    .ph {
      display: none;
    }
  }
  @media (max-width: 860px) {
    .menu {
      display: inline-flex;
      margin-left: -6px;
    }
    .hdr {
      padding: 0 8px 0 12px;
      gap: 6px;
    }
    .search .kbd {
      display: none;
    }
    .search {
      border: 0;
      background: transparent;
      width: 28px;
      padding: 0;
      justify-content: center;
    }
    .new {
      width: 28px;
      padding: 0;
    }
    .lbl {
      display: none;
    }
    .crumb.link {
      display: none;
    }
    .crumbs :global(.sep) {
      display: none;
    }
  }
</style>
