<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { copyText } from '../lib/clipboard';
  import { bytes } from '../lib/format';
  import { notificationsEnabled, notificationsSupported, setNotifications } from '../lib/notify';
  import { ui, type ThemePref } from '../lib/ui.svelte';
  import type { HostInfo, InsightsSettings } from '../lib/protocol';
  import { Copy, Laptop, Monitor, Moon, Plus, Server, Sun, Trash2, Cloud, Bell, Check, CircleAlert, Download, RefreshCw, RotateCcw } from '../lib/icons';
  import { desktopUpdate } from '../lib/desktop-update.svelte';
  import HostDot from '../components/HostDot.svelte';
  import InlineEdit from '../components/InlineEdit.svelte';

  $effect(() => fleet.watchStats());

  // fleet.client is set once at startup and never replaced, so a plain const is enough.
  const pairUrl = fleet.client ? `${fleet.client.httpOrigin}/#token=${encodeURIComponent(fleet.client.token)}` : '';
  const loopback = $derived(/\/\/(127\.|localhost|\[::1\])/.test(pairUrl));
  let reveal = $state(false);
  let notify = $state(notificationsEnabled());

  const THEMES: { id: ThemePref; label: string; icon: typeof Sun }[] = [
    { id: 'system', label: 'System', icon: Monitor },
    { id: 'light', label: 'Light', icon: Sun },
    { id: 'dark', label: 'Dark', icon: Moon },
  ];

  function rename(h: HostInfo, name: string) {
    fleet
      .call('hosts.rename', { id: h.id, name })
      .then((nh) => (fleet.hosts[nh.id] = nh))
      .catch((err) => ui.error('Rename failed', err));
  }

  function remove(h: HostInfo) {
    ui.dialog = {
      kind: 'confirm',
      title: 'Remove host',
      message: `Disconnect ${h.name} and remove it from the fleet? Agents on it keep running on that machine.`,
      action: 'Remove host',
      danger: true,
      run: async () => {
        await fleet.call('hosts.remove', { id: h.id });
        delete fleet.hosts[h.id];
      },
    };
  }

  function copy(t: string, what: string) {
    copyText(t).then(() => ui.toast('success', `${what} copied`), (err) => ui.error('Copy failed', err));
  }

  async function toggleNotify(on: boolean) {
    notify = await setNotifications(on);
    if (on && !notify) ui.toast('error', 'Notifications are blocked', 'Allow them for this site in the browser settings.');
  }

  // ─── Usage log: kept per host, so each online host is configured on its own ──────────────
  const RETENTION = [
    { days: 30, label: '30 days' },
    { days: 90, label: '90 days' },
    { days: 180, label: '180 days' },
    { days: 365, label: '1 year' },
    { days: 0, label: 'Forever' },
  ];
  const onlineHosts = $derived(fleet.hostList.filter((h) => h.online));
  let usageLog = $state<Record<string, InsightsSettings>>({});
  /** Hosts already asked (plain set: the effect must not re-run when an answer arrives). */
  const requested = new Set<string>();

  $effect(() => {
    for (const h of onlineHosts) {
      if (requested.has(h.id)) continue;
      requested.add(h.id);
      fleet
        .call('insights.settings', undefined, h.id)
        .then((s) => (usageLog[h.id] = s))
        .catch((err) => {
          requested.delete(h.id);
          ui.error(`Could not load the usage log settings of ${h.name}`, err);
        });
    }
  });

  function updateLog(h: HostInfo, change: { enabled?: boolean; retentionDays?: number }) {
    fleet
      .call('insights.settings', change, h.id)
      .then((s) => (usageLog[h.id] = s))
      .catch((err) => ui.error('Could not change the usage log', err));
  }

  function clearLog(h: HostInfo) {
    ui.dialog = {
      kind: 'confirm',
      title: 'Delete usage log',
      message: `Delete the usage log of ${h.name}? The Workflows tab then only shows what can be reconstructed from agent histories.`,
      action: 'Delete log',
      danger: true,
      run: async () => {
        usageLog[h.id] = await fleet.call('insights.clear', undefined, h.id);
      },
    };
  }

  const masked = $derived(pairUrl.replace(/token=[^&]+/, 'token=••••••••••••'));
</script>

<div class="scroll">
  <div class="page">
    <h1>Hosts &amp; Settings</h1>

    <section>
      <h2 class="section-title">
        Hosts <span class="count">{fleet.hostList.length}</span>
        <button class="btn sm add" onclick={() => (ui.dialog = { kind: 'addHost' })}><Plus size={13} /> Add host</button>
      </h2>
      <div class="card table">
        <div class="tr th">
          <span>Host</span><span>Kind</span><span>Address</span><span>Load</span><span>Versions</span><span></span>
        </div>
        {#each fleet.hostList as h (h.id)}
          {@const s = fleet.stats[h.id]}
          <div class="tr" class:off={!h.online}>
            <span class="name">
              <HostDot online={h.online} />
              <InlineEdit value={h.name} label="Host name" onsave={(n) => rename(h, n)} />
              {#if h.id === fleet.homeId}<span class="chip">this daemon</span>{/if}
            </span>
            <span class="kind">
              {#if h.kind === 'local'}<Laptop size={13} />{:else if h.kind === 'ssh'}<Server size={13} />{:else}<Cloud size={13} />{/if}
              {h.kind === 'ssh' ? 'SSH' : h.kind === 'direct' ? 'Direct' : 'Local'}
            </span>
            <span class="mono addr ellipsis" title={h.address}>{h.address ?? '—'}</span>
            <span class="num load">
              {#if s}{Math.round(s.cpu)}% · {bytes(s.memUsed)}{:else}<span class="faint">{h.online ? '—' : 'Offline'}</span>{/if}
            </span>
            <span class="ver mono">
              <span title="Coocoo">{h.version ?? '—'}</span>
              <span class="faint" title="omp">omp {h.ompVersion ?? '—'}</span>
            </span>
            <span class="acts">
              {#if h.kind !== 'local'}
                <button class="btn ghost icon sm" title="Remove host" aria-label="Remove {h.name}" onclick={() => remove(h)}><Trash2 size={13} /></button>
              {/if}
            </span>
          </div>
          {#if h.error && h.online === false}
            <div class="tr errrow"><span class="err">{h.error}</span></div>
          {/if}
        {/each}
      </div>
    </section>

    <section>
      <h2 class="section-title">Web &amp; phone access</h2>
      <div class="card pad">
        <p class="muted">Open this link on any device that can reach the daemon to use Coocoo in a browser. It contains the access token — treat it like a password.</p>
        <div class="pair">
          <code class="ellipsis">{reveal ? pairUrl : masked}</code>
          <button class="btn sm" onclick={() => (reveal = !reveal)}>{reveal ? 'Hide' : 'Reveal'}</button>
          <button class="btn sm primary" onclick={() => copy(pairUrl, 'Link')}><Copy size={13} /> Copy link</button>
        </div>
        {#if loopback}
          <p class="hint">This daemon only listens on loopback. For phone access, run it with <code>--listen 0.0.0.0:7421</code> or expose it via Tailscale, then use that address.</p>
        {/if}
      </div>
    </section>

    <section>
      <h2 class="section-title">Appearance</h2>
      <div class="card pad row">
        <span>Theme</span>
        <div class="seg" role="radiogroup" aria-label="Theme">
          {#each THEMES as t (t.id)}
            <button role="radio" aria-checked={ui.themePref === t.id} class:on={ui.themePref === t.id} onclick={() => ui.setTheme(t.id)}><t.icon size={13} />{t.label}</button>
          {/each}
        </div>
      </div>
    </section>

    {#if notificationsSupported(fleet.native)}
      <section>
        <h2 class="section-title">Notifications</h2>
        <div class="card pad row">
          <span class="lbl"><Bell size={14} /> Notify when an agent needs input or finishes while this tab is in the background</span>
          <label class="switch"><input type="checkbox" checked={notify} onchange={(e) => toggleNotify(e.currentTarget.checked)} /></label>
        </div>
      </section>
     {/if}

    {#if fleet.native}
      <section>
        <h2 class="section-title">Desktop app</h2>
        <div class="card pad update-card">
          <div class="update-row">
            <div class="update-info">
              <span class="update-title">Coocoo Desktop</span>
              <span class="faint mono">v{desktopUpdate.currentVersion ?? __APP_VERSION__}</span>
            </div>
            <div class="update-actions">
              {#if desktopUpdate.isAvailable}
                <button
                  class="btn sm primary"
                  onclick={() => {
                    ui.dialog = {
                      kind: 'confirm',
                      title: `Update to v${desktopUpdate.availableVersion}`,
                      message: 'The app will download the update, pause any active agents, restart, and resume them. Proceed?',
                      action: 'Install and restart',
                      run: () => desktopUpdate.install(),
                    };
                  }}
                >
                  <Download size={13} /> Install v{desktopUpdate.availableVersion} and restart
                </button>
              {:else if desktopUpdate.isDownloading}
                <button class="btn sm primary" disabled>
                  <span class="spin"><RefreshCw size={13} /></span>
                  {desktopUpdate.status && desktopUpdate.status.status === 'downloading' && desktopUpdate.status.percent != null
                    ? `Downloading ${desktopUpdate.status.percent}%…`
                    : 'Downloading…'}
                </button>
              {:else if desktopUpdate.isReady}
                <button class="btn sm primary" onclick={() => desktopUpdate.install()}>
                  <RotateCcw size={13} /> Restart now to update
                </button>
              {:else}
                <button
                  class="btn sm"
                  disabled={desktopUpdate.isChecking}
                  onclick={() => desktopUpdate.check()}
                >
                  <span class={desktopUpdate.isChecking ? 'spin' : ''}><RefreshCw size={13} /></span>
                  {desktopUpdate.isChecking ? 'Checking…' : 'Check for updates'}
                </button>
              {/if}
            </div>
          </div>

          {#if desktopUpdate.status?.status === 'available'}
            <div class="update-banner">
              <div class="update-badge-label">
                <Check size={14} /> Version {desktopUpdate.status.version} is available!
              </div>
              {#if desktopUpdate.status.body}
                <div class="update-notes faint">{desktopUpdate.status.body}</div>
              {/if}
            </div>
          {:else if desktopUpdate.status?.status === 'upToDate'}
            <div class="update-meta faint">
              Coocoo is up to date (checked {new Date(desktopUpdate.status.checkedAt).toLocaleTimeString()}).
            </div>
          {:else if desktopUpdate.status?.status === 'error'}
            <div class="update-error">
              <CircleAlert size={14} /> {desktopUpdate.status.message}
            </div>
          {:else if desktopUpdate.status?.status === 'unsupported'}
            <div class="update-meta faint">
              {desktopUpdate.status.reason}
            </div>
          {/if}
        </div>
      </section>
    {/if}

    <section>
      <h2 class="section-title">Usage log</h2>
      <div class="card table">
        <p class="muted explain">Records which features and commands are used — locally on each host, never prompt texts or arguments. It feeds the Workflows tab.</p>
        {#each onlineHosts as h (h.id)}
          {@const s = usageLog[h.id]}
          <div class="lr">
            <span class="name"><HostDot online={h.online} />{h.name}</span>
            {#if s}
              <label class="lbl"><span class="switch"><input type="checkbox" checked={s.enabled} onchange={(e) => updateLog(h, { enabled: e.currentTarget.checked })} /></span>Record usage log</label>
              <label class="lbl">
                Keep
                <select class="select" value={s.retentionDays} onchange={(e) => updateLog(h, { retentionDays: Number(e.currentTarget.value) })}>
                  {#each RETENTION as r (r.days)}<option value={r.days}>{r.label}</option>{/each}
                  {#if !RETENTION.some((r) => r.days === s.retentionDays)}<option value={s.retentionDays}>{s.retentionDays} days</option>{/if}
                </select>
              </label>
              <span class="num faint">{bytes(s.bytes)}{#if s.logSince} · since {new Date(s.logSince).toLocaleDateString()}{/if}</span>
              <button class="btn sm danger" disabled={!s.bytes} onclick={() => clearLog(h)}><Trash2 size={13} /> Delete log</button>
            {:else}
              <span class="faint">Loading…</span>
            {/if}
          </div>
        {:else}
          <div class="lr"><span class="faint">No host is online.</span></div>
        {/each}
      </div>
    </section>

    <section>
      <h2 class="section-title">Keyboard</h2>
      <div class="card pad keys">
        <div><span class="kbd">⌘</span><span class="kbd">K</span><span>Command palette</span></div>
        <div><span class="kbd">N</span><span>New agent</span></div>
        <div><span class="kbd">G</span><span class="kbd">O</span><span>Overview</span></div>
        <div><span class="kbd">G</span><span class="kbd">S</span><span>Hosts &amp; settings</span></div>
        <div><span class="kbd">J</span><span class="kbd">K</span><span>Move through the needs-you queue</span></div>
        <div><span class="kbd">Esc</span><span>Close dialog · abort a running agent</span></div>
        <div><span class="kbd">↵</span><span>Send · <span class="kbd">⇧</span><span class="kbd">↵</span> newline</span></div>
      </div>
    </section>

    <p class="about faint">Coocoo UI {__APP_VERSION__} · protocol v{fleet.protocol}</p>
  </div>
</div>

<style>
  .scroll {
    flex: 1;
    overflow-y: auto;
  }
  .page {
    max-width: 980px;
    margin: 0 auto;
    padding: 20px 24px 48px;
    display: flex;
    flex-direction: column;
    gap: 26px;
  }
  h1 {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .add {
    margin-left: auto;
    font-weight: 500;
  }
  .table {
    overflow: hidden;
  }
  .tr {
    display: grid;
    grid-template-columns: minmax(180px, 1.4fr) 90px minmax(140px, 1.4fr) 130px 150px 36px;
    align-items: center;
    gap: 12px;
    min-height: 42px;
    padding: 4px 12px;
    font-size: var(--fs-sm);
  }
  .tr + .tr {
    border-top: 1px solid var(--border);
  }
  .th {
    min-height: 32px;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
    background: var(--bg-2);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-weight: 500;
  }
  .name .chip {
    flex: none;
    height: 18px;
  }
  .kind {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .addr {
    color: var(--text-2);
    font-size: 11.5px;
  }
  .load {
    color: var(--text-2);
    min-width: 0;
  }
  .ver {
    display: flex;
    flex-direction: column;
    font-size: 11px;
    line-height: 1.35;
  }
  .acts {
    display: flex;
    justify-content: flex-end;
  }
  .off .name {
    color: var(--text-2);
  }
  .err {
    color: var(--red);
  }
  .errrow {
    display: block;
    min-height: 0;
    padding: 0 12px 10px 32px;
    border-top: 0 !important;
    font-size: var(--fs-xs);
  }
  .pad {
    padding: 14px 16px;
  }
  .pad p {
    margin: 0 0 10px;
  }
  .pair {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .pair code {
    flex: 1;
    height: 28px;
    padding: 0 10px;
    line-height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg);
    font-size: 11.5px;
  }
  .hint {
    margin: 10px 0 0 !important;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .lbl {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: var(--r);
    background: var(--bg-3);
  }
  .seg button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 12px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--border-strong);
  }
  .keys {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px 20px;
    font-size: var(--fs-sm);
  }
  .keys > div {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .keys > div > span:last-child {
    margin-left: 6px;
    color: var(--text-2);
  }
  .about {
    margin: 0;
    font-size: var(--fs-xs);
    text-align: center;
  }
  .explain {
    margin: 0;
    padding: 12px 16px;
    font-size: var(--fs-sm);
    border-bottom: 1px solid var(--border);
  }
  .lr {
    display: grid;
    grid-template-columns: minmax(140px, 1fr) auto auto minmax(130px, auto) auto;
    align-items: center;
    gap: 16px;
    min-height: 46px;
    padding: 6px 16px;
    font-size: var(--fs-sm);
  }
  .lr + .lr {
    border-top: 1px solid var(--border);
  }
  .lr .select {
    width: auto;
    height: 28px;
  }
  .lr .num {
    text-align: right;
  }
  @media (max-width: 760px) {
    .page {
      padding: 16px 12px 40px;
    }
    .lr {
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 8px 12px;
    }
    .tr {
      grid-template-columns: minmax(0, 1fr) auto 36px;
    }
    .th,
    .addr,
    .load,
    .kind {
      display: none;
    }
    .pair {
      flex-wrap: wrap;
    }
    .pair code {
      flex-basis: 100%;
    }
  }
  .update-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .update-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .update-info {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .update-title {
    font-weight: 500;
  }
  .update-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .update-banner {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--accent-subtle);
    border: 1px solid var(--accent);
  }
  .update-badge-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 500;
    color: var(--accent);
  }
  .update-notes {
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    max-height: 120px;
    overflow-y: auto;
  }
  .update-meta {
    font-size: var(--fs-xs);
  }
  .update-error {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--red);
  }
  .spin {
    display: inline-flex;
    animation: spin 0.8s linear infinite;
  }
</style>
