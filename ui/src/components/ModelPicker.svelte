<script lang="ts" module>
  const RECENT_KEY = 'coocoo.recentModels';
  const MAX_RECENT = 5;

  function loadRecent(): string[] {
    try {
      const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
      return Array.isArray(v) ? v.filter((x) => typeof x === 'string') : [];
    } catch {
      return [];
    }
  }

  const PROVIDER_WORDS: Record<string, string> = { openai: 'OpenAI', xai: 'xAI', ai: 'AI', api: 'API', github: 'GitHub', deepseek: 'DeepSeek', openrouter: 'OpenRouter' };
  /** `openai-codex` → `OpenAI Codex`. */
  export function providerLabel(provider: string): string {
    return provider
      .split(/[-_]/)
      .map((w) => PROVIDER_WORDS[w] ?? w.charAt(0).toUpperCase() + w.slice(1))
      .join(' ');
  }

  /** `200000` → `200k`, `1000000` → `1M`. */
  function contextLabel(tokens?: number): string | undefined {
    if (!tokens) return undefined;
    if (tokens >= 1_000_000) return `${+(tokens / 1_000_000).toFixed(1)}M`;
    return `${Math.round(tokens / 1000)}k`;
  }
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import type { ModelInfo } from '../lib/protocol';
  import { track } from '../lib/track';
  import { Brain, Check, ChevronDown, Search } from '../lib/icons';

  // Searchable model picker: recent models first, then one group per provider (newest-looking
  // names first). Type to filter by name, id or provider; ↑/↓/Enter/Esc work from the search box.
  let {
    models,
    value,
    onselect,
    defaultLabel,
    variant = 'mini',
    loading = false,
  }: {
    models: ModelInfo[];
    /** `provider/id`, or `''` for the default. */
    value: string;
    onselect: (value: string) => void;
    /** Offer a "default" entry (value `''`) with this label. */
    defaultLabel?: string;
    /** `mini`: compact composer button; `field`: full-width like a select. */
    variant?: 'mini' | 'field';
    loading?: boolean;
  } = $props();

  type Row = { value: string; name: string; id: string; provider: string; ctx?: string; reasoning: boolean };
  type Group = { label: string; rows: Row[] };

  let open = $state(false);
  let query = $state('');
  let active = $state(0);
  let recent = $state<string[]>(loadRecent());
  let trigger = $state<HTMLButtonElement>();
  let pop = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement>();
  let place = $state('');

  const rows = $derived(
    models.map(
      (m): Row => ({ value: `${m.provider}/${m.id}`, name: m.name || m.id, id: m.id, provider: m.provider, ctx: contextLabel(m.contextWindow), reasoning: m.reasoning }),
    ),
  );
  const current = $derived(rows.find((r) => r.value === value));
  const buttonLabel = $derived(current?.name ?? (value || defaultLabel || 'Default model'));

  const groups = $derived.by((): Group[] => {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (r: Row) => {
      const hay = `${r.name} ${r.id} ${r.provider} ${providerLabel(r.provider)}`.toLowerCase();
      return terms.every((t) => hay.includes(t));
    };
    const byName = (a: Row, b: Row) => b.name.localeCompare(a.name, undefined, { numeric: true, sensitivity: 'base' });
    const out: Group[] = [];
    if (!terms.length) {
      const rec = recent.map((v) => rows.find((r) => r.value === v)).filter((r): r is Row => !!r);
      if (defaultLabel !== undefined) out.push({ label: '', rows: [{ value: '', name: defaultLabel, id: '', provider: '', reasoning: false }] });
      if (rec.length) out.push({ label: 'Recent', rows: rec });
    }
    const providers = new Map<string, Row[]>();
    for (const r of rows.filter(hit)) providers.set(r.provider, [...(providers.get(r.provider) ?? []), r]);
    for (const [provider, list] of providers) out.push({ label: providerLabel(provider), rows: list.sort(byName) });
    return out;
  });
  const flat = $derived(groups.flatMap((g) => g.rows));

  async function show() {
    query = '';
    open = true;
    track('model.open');
    await tick();
    position();
    active = Math.max(0, flat.findIndex((r) => r.value === value));
    // On touch devices focusing would pop the keyboard over the list; search starts on tap.
    if (!matchMedia('(pointer: coarse)').matches) input?.focus();
    scrollActive();
  }

  /** Below the button, or above it when there is more room there (the composer sits low). */
  function position() {
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const width = Math.min(360, window.innerWidth - 16);
    const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
    const below = window.innerHeight - r.bottom - 12;
    const above = r.top - 12;
    const up = below < 320 && above > below;
    const max = Math.min(440, up ? above : below);
    place = `left:${left}px;width:${width}px;max-height:${max}px;` + (up ? `bottom:${window.innerHeight - r.top + 6}px` : `top:${r.bottom + 6}px`);
  }

  function choose(v: string) {
    open = false;
    trigger?.focus();
    if (v) {
      recent = [v, ...recent.filter((x) => x !== v)].slice(0, MAX_RECENT);
      localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
    }
    if (v !== value) {
      track('model.pick', { meta: { model: v || 'default' } });
      onselect(v);
    }
  }

  async function scrollActive() {
    await tick();
    pop?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!flat.length) return;
      active = (active + (e.key === 'ArrowDown' ? 1 : -1) + flat.length) % flat.length;
      scrollActive();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (flat[active]) choose(flat[active].value);
    }
  }

  $effect(() => {
    void query;
    active = 0;
  });

  $effect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!pop?.contains(e.target as Node) && !trigger?.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      e.preventDefault();
      open = false;
      trigger?.focus();
    };
    const reflow = () => position();
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    window.addEventListener('resize', reflow);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', esc, true);
      window.removeEventListener('resize', reflow);
    };
  });

  /** Render the popover at the end of `<body>`, so dialogs and scroll containers cannot clip it. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<button
  type="button"
  class="trigger"
  class:compact={variant === 'mini'}
  class:wide={variant === 'field'}
  class:select={variant === 'field'}
  class:on={open}
  bind:this={trigger}
  onclick={() => (open ? (open = false) : show())}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-label="Model"
  title={value || 'Model'}
>
  <span class="ellipsis">{buttonLabel}</span>
  {#if variant === 'mini'}<ChevronDown size={12} />{/if}
</button>

{#if open}
  <div class="pop" style={place} bind:this={pop} use:portal>
    <label class="search">
      <Search size={13} />
      <input
        bind:this={input}
        bind:value={query}
        {onkeydown}
        placeholder="Search models…"
        spellcheck="false"
        autocomplete="off"
        role="combobox"
        aria-expanded="true"
        aria-controls="model-list"
        aria-activedescendant={flat[active] ? `model-opt-${active}` : undefined}
      />
    </label>
    <div class="list" id="model-list" role="listbox" aria-label="Models">
      {#each groups as g, gi (g.label + gi)}
        {#if g.label}<div class="glabel">{g.label}</div>{/if}
        {#each g.rows as r (r.value)}
          {@const i = flat.indexOf(r, groups.slice(0, gi).reduce((n, x) => n + x.rows.length, 0))}
          <button
            type="button"
            class="row"
            id="model-opt-{i}"
            role="option"
            aria-selected={r.value === value}
            data-active={i === active}
            onmousemove={() => (active = i)}
            onclick={() => choose(r.value)}
          >
            <span class="check">{#if r.value === value}<Check size={13} />{/if}</span>
            <span class="name ellipsis">{r.name}</span>
            {#if r.id && r.id !== r.name}<span class="id mono ellipsis">{r.id}</span>{/if}
            <span class="tags">
              {#if g.label === 'Recent'}<span class="tag">{providerLabel(r.provider)}</span>{/if}
              {#if r.id}<span class="tag icon" class:off={!r.reasoning} title={r.reasoning ? 'Reasoning model' : undefined}><Brain size={11} /></span>{/if}
              {#if r.ctx}<span class="tag num" title="Context window">{r.ctx}</span>{/if}
            </span>
          </button>
        {/each}
      {/each}
      {#if !flat.length}
        <div class="empty">{loading ? 'Loading models…' : query ? `No model matches “${query}”` : 'No models available'}</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    cursor: pointer;
  }
  .trigger.compact {
    max-width: 190px;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
  }
  .trigger.compact:hover,
  .trigger.compact.on {
    background: var(--bg-hover);
    color: var(--text);
  }
  .trigger.compact :global(svg) {
    flex: none;
    opacity: 0.7;
  }
  .trigger.wide {
    justify-content: flex-start;
    text-align: left;
  }
  .trigger.wide > span {
    flex: 1;
  }
  .pop {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    animation: pop var(--t-fast) var(--ease);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(2px);
    }
  }
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: none;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 36px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    outline: none;
  }
  .list {
    flex: 1;
    min-height: 0;
    padding: 4px;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .glabel {
    padding: 8px 8px 3px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    padding: 0 8px 0 4px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .row[data-active='true'] {
    background: var(--bg-hover);
  }
  .check {
    display: inline-flex;
    flex: none;
    justify-content: center;
    width: 16px;
    color: var(--accent);
  }
  .name {
    flex: none;
    max-width: 60%;
  }
  .id {
    flex: 1;
    min-width: 0;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .tags {
    display: flex;
    gap: 4px;
    flex: none;
    margin-left: auto;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 5px;
    border-radius: var(--r-sm);
    background: var(--bg-3);
    color: var(--text-2);
    font-size: 10px;
  }
  .tag.icon {
    padding: 0 4px;
    color: var(--purple, var(--text-2));
  }
  /* Keeps the context column aligned on rows without reasoning. */
  .tag.icon.off {
    visibility: hidden;
  }
  .tag.num {
    justify-content: center;
    min-width: 36px;
  }
  .empty {
    padding: 14px 10px;
    color: var(--text-3);
    font-size: var(--fs-sm);
    text-align: center;
  }
</style>
