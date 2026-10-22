<script lang="ts">
  import { untrack } from 'svelte';
  import { now } from '../lib/clock.svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago, cost, until } from '../lib/format';
  import type { UsageCapacity, UsageLimit } from '../lib/protocol';

  // Subscription quota of the agent's model provider on the agent's host, the way that provider
  // meters it (5-hour/weekly windows, monthly credits, extra usage…), as `omp usage` reports it.
  let { host, model, running }: { host: string; model: string | undefined; running: boolean } = $props();

  const provider = $derived(model && model.includes('/') ? model.slice(0, model.indexOf('/')) : undefined);
  const modelId = $derived(model ? model.slice(model.indexOf('/') + 1) : '');
  const usage = $derived(provider ? fleet.usage[key(host, `${provider}/${modelId}`)] : undefined);

  let open = $state(false);
  let root = $state<HTMLDivElement>();
  let pop = $state<HTMLDivElement>();
  /** Horizontal offset that keeps the popover on screen (the pill sits mid-bar). */
  let shift = $state(0);

  /** Untracked: effects below depend only on what they name, not on the usage store. */
  function load(maxAge?: number) {
    untrack(() => {
      if (provider) fleet.loadUsage(host, provider, modelId, maxAge).catch(() => {});
    });
  }

  $effect(() => {
    void [host, provider, modelId];
    load();
  });

  // Quota moves while the agent works: refetch when a turn ends.
  let wasRunning = untrack(() => running);
  $effect(() => {
    if (wasRunning && !running) load(0);
    wasRunning = running;
  });

  $effect(() => {
    if (!open) return;
    load();
    const close = (e: MouseEvent) => {
      if (!root?.contains(e.target as Node)) open = false;
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

  $effect(() => {
    if (!pop || !root) return;
    const left = root.getBoundingClientRect().left;
    const fit = Math.max(12, Math.min(left, innerWidth - 12 - pop.offsetWidth));
    shift = fit - left;
  });

  const fraction = (w: UsageCapacity) => (w.accounts ? w.used / w.accounts : 0);
  const pct = (f: number) => `${Math.round(f * 100)}%`;
  const level = (f: number | undefined, status?: string) =>
    status === 'exhausted' || (f ?? 0) >= 1 ? 'hot' : status === 'warning' || (f ?? 0) >= 0.8 ? 'warm' : '';
  const windowName = (w: UsageCapacity) => (w.name ? `${w.name} ${w.window}` : w.window);

  // Time windows that gate this model: provider-wide ones plus the separate pool the model draws
  // from (an Anthropic tier, Codex Spark, the Antigravity counter of its family); the daemon
  // decides which.
  const gating = $derived((usage?.windows ?? []).filter((w) => w.gating && w.durationMs != null && w.accounts > 0));
  /** One pool behind every compact window (Antigravity): named once in front instead of per window. */
  const poolName = $derived(gating.length && gating.every((w) => w.name === gating[0].name) ? gating[0].name : undefined);
  const tightest = $derived(gating.reduce<UsageCapacity | undefined>((t, w) => (!t || fraction(w) > fraction(t) ? w : t), undefined));
  /** Limit label with its window when the label alone does not say it (`Gemini` → `Gemini · Weekly`). */
  const limitLabel = (l: UsageLimit) => (l.window && !l.label.includes(l.window) ? `${l.label} · ${l.window}` : l.label);

  function amount(l: UsageLimit): { value: string; left?: string } {
    const f = l.usedFraction;
    if (l.unit === 'percent' || l.used == null) return { value: f == null ? '—' : pct(f), left: f == null ? undefined : `${pct(Math.max(0, 1 - f))} left` };
    const fmt = (n: number) => (l.unit === 'usd' ? cost(n) : n.toLocaleString(undefined, { maximumFractionDigits: 1 }));
    const unit = l.unit === 'usd' || l.unit === 'unknown' ? '' : ` ${l.unit}`;
    if (l.limit == null) return { value: `${fmt(l.used)}${unit}` };
    return { value: `${fmt(l.used)} / ${fmt(l.limit)}${unit}`, left: `${fmt(Math.max(0, l.limit - l.used))}${unit} left` };
  }
</script>

<!-- Hidden for pay-per-token providers and plans that report no limits. -->
{#if usage?.accounts.some((acct) => acct.limits.length)}
  <div class="usage" bind:this={root}>
    <button
      class="pill num"
      class:active={open}
      aria-haspopup="dialog"
      aria-expanded={open}
      title="{usage.provider} usage limits{poolName ? ` (${poolName} quota, used by this model)` : ''}"
      onclick={() => (open = !open)}
    >
      {#if poolName}<span class="pn minor">{poolName}</span>{/if}
      {#each gating as w (windowName(w))}
        {@const f = fraction(w)}
        <span class="w" class:minor={w !== tightest}>
          <span class="wl">{poolName ? w.window : windowName(w)}</span>
          <span class="progress {level(f)}"><i style:width="{Math.min(100, f * 100)}%"></i></span>
          <span class="wv {level(f)}">{pct(f)}</span>
        </span>
      {:else}
        <span class="wl">Quota</span>
      {/each}
    </button>
    {#if open}
      <div class="pop" role="dialog" aria-label="Usage limits" bind:this={pop} style:left="{shift}px">
        <div class="head">
          <b>{usage.provider}</b>
          <span class="faint">{ago(usage.fetchedAt, now.value) === 'now' ? 'just updated' : `updated ${ago(usage.fetchedAt, now.value)} ago`}</span>
        </div>
        {#if usage.accounts.length > 1}
          <section>
            <div class="acct">All {usage.accounts.length} accounts</div>
            {#each usage.windows.filter((w) => w.durationMs != null) as w (windowName(w))}
              {@const f = fraction(w)}
              <div class="row">
                <span class="rl">{windowName(w)}</span>
                <span class="progress {level(f)}"><i style:width="{Math.min(100, f * 100)}%"></i></span>
                <span class="rv num {level(f)}">{pct(f)}</span>
                <span class="rs faint num">{[`${w.used.toFixed(2)} of ${w.accounts} accounts used`, w.resetsAt ? `resets in ${until(w.resetsAt, now.value)}` : ''].filter(Boolean).join(' · ')}</span>
              </div>
            {/each}
          </section>
        {/if}
        {#each usage.accounts as acct, i (i)}
          <section>
            <div class="acct ellipsis" title={acct.label}>
              {acct.label}<span class="faint">{[acct.org, acct.plan].filter(Boolean).map((s) => ` · ${s}`).join('')}</span>
            </div>
            {#each acct.limits as l (l.id)}
              {@const a = amount(l)}
              {@const lv = level(l.usedFraction, l.status)}
              {@const sub = [a.left, l.resetsAt && l.resetsAt > now.value ? `resets in ${until(l.resetsAt, now.value)}` : ''].filter(Boolean).join(' · ')}
              <div class="row">
                <span class="rl">{limitLabel(l)}</span>
                <span class="progress {lv}"><i style:width="{Math.min(100, (l.usedFraction ?? 0) * 100)}%"></i></span>
                <span class="rv num ellipsis {lv}" title={a.value}>{a.value}</span>
                {#if sub}<span class="rs faint num">{sub}</span>{/if}
              </div>
            {:else}
              <div class="row faint">No limits reported</div>
            {/each}
          </section>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .usage {
    position: relative;
    min-width: 0;
  }
  .pill {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 100%;
    height: 24px;
    padding: 0 6px;
    overflow: hidden;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .pill:hover,
  .pill.active {
    background: var(--bg-hover);
    color: var(--text);
  }
  .w {
    display: flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .wl {
    color: var(--text-3);
  }
  .pn {
    color: var(--text-2);
    font-weight: 500;
    white-space: nowrap;
  }
  .w .progress {
    width: 28px;
  }
  .progress > i {
    background: var(--text-2);
  }
  .progress.warm > i {
    background: var(--amber);
  }
  .progress.hot > i {
    background: var(--red);
  }
  .warm:not(.progress) {
    color: var(--amber);
  }
  .hot:not(.progress) {
    color: var(--red);
  }
  .pop {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 0;
    z-index: 40;
    width: 330px;
    max-width: calc(100vw - 24px);
    max-height: 60vh;
    overflow-y: auto;
    padding: 10px 12px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-xs);
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 4px;
    font-size: var(--fs-sm);
  }
  .head .faint {
    font-size: var(--fs-xs);
  }
  section {
    padding-top: 8px;
  }
  section + section {
    margin-top: 8px;
    border-top: 1px solid var(--border);
  }
  .acct {
    margin-bottom: 6px;
    color: var(--text-2);
    font-weight: 500;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px 84px;
    align-items: center;
    gap: 2px 8px;
    padding: 3px 0;
  }
  .rl {
    color: var(--text-2);
  }
  .rv {
    text-align: right;
    color: var(--text);
  }
  .rs {
    grid-column: 1 / -1;
  }
  /* The pool name goes first when space runs short; it stays in the tooltip and popover. */
  @container (max-width: 760px) {
    .pn {
      display: none;
    }
  }
  @container (max-width: 520px) {
    .minor {
      display: none;
    }
  }
</style>
