<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { bytes } from '../lib/format';
  import { notificationsEnabled, notificationsSupported, setNotifications } from '../lib/notify';
  import { ui, type ThemePref } from '../lib/ui.svelte';
  import type { HostInfo } from '../lib/protocol';
  import { Copy, Laptop, Monitor, Moon, Plus, Server, Sun, Trash2, Cloud, Bell } from '../lib/icons';
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
    void navigator.clipboard.writeText(t).then(() => ui.toast('success', `${what} copied`));
  }

  async function toggleNotify(on: boolean) {
    notify = await setNotifications(on);
    if (on && !notify) ui.toast('error', 'Notifications are blocked', 'Allow them for this site in the browser settings.');
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
  @media (max-width: 760px) {
    .page {
      padding: 16px 12px 40px;
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
</style>
