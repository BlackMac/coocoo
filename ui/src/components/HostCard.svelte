<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { bytes } from '../lib/format';
  import { agentState } from '../lib/status';
  import type { HostInfo } from '../lib/protocol';
  import { Cloud, Laptop, Server } from '../lib/icons';
  import HostDot from './HostDot.svelte';

  let { host: h }: { host: HostInfo } = $props();

  const s = $derived(fleet.stats[h.id]);
  const agents = $derived(Object.values(fleet.agents).filter((a) => a.hostId === h.id && a.status !== 'stopped'));
  const active = $derived(agents.filter((a) => ['working', 'needs'].includes(agentState(a))).length);
  const procs = $derived.by(() => {
    let daemon = 0;
    let omp = 0;
    let ompN = 0;
    let chrome = 0;
    for (const p of s?.procs ?? []) {
      if (p.key === 'daemon') daemon += p.rss;
      else if (p.key.startsWith('browser:')) chrome += p.rss;
      else {
        omp += p.rss;
        ompN++;
      }
    }
    return { daemon, omp, ompN, chrome };
  });
  const memPct = $derived(s ? (s.memUsed / s.memTotal) * 100 : 0);
  const level = (pct: number) => (pct >= 90 ? 'hot' : pct >= 70 ? 'warm' : '');
</script>

<div class="host card" class:offline={!h.online}>
  <div class="top">
    <span class="kind" title={h.kind}>
      {#if h.kind === 'local'}<Laptop size={14} />{:else if h.kind === 'ssh'}<Server size={14} />{:else}<Cloud size={14} />{/if}
    </span>
    <span class="name ellipsis">{h.name}</span>
    <HostDot online={h.online} />
    <span class="agents num" title="Active / total agents">
      <strong>{active}</strong><span class="faint">/{agents.length}</span>
    </span>
  </div>
  {#if !h.online}
    <div class="err ellipsis" title={h.error}>{h.error ?? 'Offline'}</div>
    <div class="sub faint ellipsis">{h.address ?? ''}</div>
  {:else if s}
    <div class="meters">
      <div class="meter">
        <span class="lbl">CPU</span>
        <span class="progress {level(s.cpu)}"><i style:width="{Math.min(100, s.cpu)}%"></i></span>
        <span class="val num">{Math.round(s.cpu)}%</span>
      </div>
      <div class="meter">
        <span class="lbl">RAM</span>
        <span class="progress {level(memPct)}"><i style:width="{memPct}%"></i></span>
        <span class="val num">{bytes(s.memUsed).replace(/ GB$/, '')}<span class="faint">/{bytes(s.memTotal)}</span></span>
      </div>
    </div>
    <div class="procs num">
      <span title="Coocoo process (daemon; the desktop app including its webview)">coocoo <b>{bytes(procs.daemon)}</b></span>
      <span title="{procs.ompN} omp processes">omp <b>{procs.ompN ? bytes(procs.omp) : '—'}</b>{#if procs.ompN}<span class="faint"> ×{procs.ompN}</span>{/if}</span>
      <span title="Chromium">chromium <b>{procs.chrome ? bytes(procs.chrome) : '—'}</b></span>
    </div>
  {:else}
    <div class="sub faint">{[h.os, h.arch].filter(Boolean).join(' · ') || 'Waiting for stats…'}</div>
  {/if}
</div>

<style>
  .host {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    padding: 12px 14px;
  }
  .host.offline {
    background: transparent;
    border-style: dashed;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .kind {
    display: flex;
    color: var(--text-3);
  }
  .name {
    font-weight: 600;
  }
  .agents {
    margin-left: auto;
    font-size: var(--fs-sm);
  }
  .agents strong {
    font-weight: 600;
  }
  .meters {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .meter {
    display: grid;
    grid-template-columns: 30px 1fr auto;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
  }
  .lbl {
    color: var(--text-3);
    font-weight: 500;
  }
  .val {
    min-width: 40px;
    text-align: right;
    color: var(--text-2);
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
  .procs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .procs b {
    color: var(--text-2);
    font-weight: 500;
  }
  .err {
    color: var(--red);
    font-size: var(--fs-sm);
  }
  .sub {
    font-size: var(--fs-xs);
  }
</style>
