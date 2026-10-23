<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AddHostParams } from '../lib/protocol';
  import Dialog from './Dialog.svelte';
  import { Globe, Link, Terminal } from '../lib/icons';

  let tab = $state<'link' | 'ssh' | 'direct'>('link');
  let link = $state('');
  let name = $state('');
  let target = $state('');
  let sshPort = $state('');
  let remotePort = $state('7421');
  let install = $state(true);
  let sshToken = $state('');
  let url = $state('');
  let token = $state('');
  let busy = $state(false);

  /**
   * A connect link as `coocoo pair` and Settings show it (`http://host:7421/#token=…`): the
   * daemon's WebSocket URL and token. A path before the hash (a reverse proxy) is kept.
   */
  function parseLink(text: string): { url: string; token: string; loopback: boolean } | undefined {
    try {
      const u = new URL(text.trim());
      if (u.protocol !== 'http:' && u.protocol !== 'https:') return undefined;
      const t = /(?:^#|[#&?])token=([^&]+)/.exec(u.hash)?.[1] ?? u.searchParams.get('token');
      if (!t) return undefined;
      const path = u.pathname.replace(/\/+$/, '');
      return {
        url: `${u.protocol === 'https:' ? 'wss' : 'ws'}://${u.host}${path}/ws`,
        token: decodeURIComponent(t),
        loopback: /^(127\.|localhost$|\[::1\]$)/.test(u.hostname),
      };
    } catch {
      return undefined;
    }
  }
  const parsed = $derived(parseLink(link));

  /** A connect link pasted into the Direct URL field fills URL and token. */
  function splitDirect(e: ClipboardEvent) {
    const p = parseLink(e.clipboardData?.getData('text') ?? '');
    if (!p) return;
    e.preventDefault();
    [url, token] = [p.url, p.token];
  }

  const valid = $derived(
    tab === 'link'
      ? !!parsed && !parsed.loopback
      : tab === 'ssh'
        ? /^[^\s@]+@[^\s@]+$|^[^\s@]+$/.test(target.trim())
        : /^wss?:\/\/./.test(url.trim()) && !!token.trim(),
  );

  async function add(e: SubmitEvent) {
    e.preventDefault();
    if (!valid || busy) return;
    const params: AddHostParams =
      tab === 'link'
        ? { kind: 'direct', url: parsed!.url, token: parsed!.token, name: name.trim() || undefined }
        : tab === 'ssh'
        ? {
            kind: 'ssh',
            target: target.trim(),
            sshPort: sshPort ? Number(sshPort) : undefined,
            remotePort: remotePort ? Number(remotePort) : undefined,
            token: sshToken.trim() || undefined,
            install,
            name: name.trim() || undefined,
          }
        : { kind: 'direct', url: url.trim(), token: token.trim(), name: name.trim() || undefined };
    busy = true;
    try {
      // The daemon waits up to 240 s when it has to install Coocoo over SSH.
      const h = await fleet.call('hosts.add', params, undefined, 260_000);
      fleet.hosts[h.id] = h;
      ui.toast('success', `Added ${h.name}`, h.online ? 'Connected' : h.error);
      ui.submitDialog();
    } catch (err) {
      ui.error('Could not add host', err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Add host" width={500} onclose={() => (ui.dialog = null)}>
  <div class="tabs" role="tablist">
    <button role="tab" class:on={tab === 'link'} aria-selected={tab === 'link'} onclick={() => (tab = 'link')}><Link size={14} /> Link</button>
    <button role="tab" class:on={tab === 'ssh'} aria-selected={tab === 'ssh'} onclick={() => (tab = 'ssh')}><Terminal size={14} /> SSH</button>
    <button role="tab" class:on={tab === 'direct'} aria-selected={tab === 'direct'} onclick={() => (tab = 'direct')}><Globe size={14} /> Direct URL</button>
  </div>
  <form id="add-host" class="form" onsubmit={add}>
    {#if tab === 'link'}
      <p class="hint">Paste the connect link of the other machine: run <code>coocoo pair</code> there, or copy it under <em>Hosts &amp; Settings</em> in its Coocoo.</p>
      <label class="field">
        <span>Connect link</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value={link} placeholder="http://192.168.1.20:7421/#token=…" spellcheck="false" autocomplete="off" autofocus />
      </label>
      {#if link.trim() && !parsed}
        <span class="msg err">Not a connect link: it looks like <code>http://host:port/#token=…</code>.</span>
      {:else if parsed?.loopback}
        <span class="msg err">This link only works on that machine itself. Run <code>coocoo pair</code> there for its LAN address.</span>
      {:else if parsed}
        <span class="msg faint">Connects to <code>{parsed.url}</code></span>
      {/if}
    {:else if tab === 'ssh'}
      <p class="hint">The home daemon opens an SSH tunnel to the remote daemon using your OpenSSH config and agent. Coocoo can install and start <code>coocoo serve</code> on the remote machine.</p>
      <label class="field">
        <span>SSH target</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value={target} placeholder="user@build-server" spellcheck="false" autocomplete="off" autofocus />
      </label>
      <div class="row3">
        <label class="field"><span>SSH port</span><input class="input mono" bind:value={sshPort} placeholder="22" inputmode="numeric" /></label>
        <label class="field"><span>Remote daemon port</span><input class="input mono" bind:value={remotePort} placeholder="7421" inputmode="numeric" /></label>
      </div>
      <label class="field">
        <span>Remote token <span class="faint">(optional — read over SSH when empty)</span></span>
        <input class="input mono" bind:value={sshToken} spellcheck="false" autocomplete="off" />
      </label>
      <label class="switch"><input type="checkbox" bind:checked={install} /> Install or update Coocoo on the remote host if needed</label>
    {:else}
      <p class="hint">Connect to a daemon reachable over the network (LAN, Tailscale, WireGuard). Run <code>coocoo serve --listen 0.0.0.0:7421</code> on it and copy its token.</p>
      <label class="field">
        <span>Daemon URL</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value={url} onpaste={splitDirect} placeholder="ws://10.0.0.42:7421/ws" spellcheck="false" autocomplete="off" autofocus />
      </label>
      <label class="field">
        <span>Token</span>
        <input class="input mono" bind:value={token} spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    <label class="field">
      <span>Display name <span class="faint">(optional)</span></span>
      <input class="input" bind:value={name} placeholder="Defaults to the machine's hostname" />
    </label>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="add-host" disabled={!valid || busy}>
      {#if busy}<span class="spinner"></span> Connecting…{:else}Add host{/if}
    </button>
  {/snippet}
</Dialog>

<style>
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r);
    background: var(--bg-2);
  }
  .tabs button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 26px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
    cursor: pointer;
  }
  .tabs button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--border-strong);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .hint {
    margin: 0;
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .msg {
    margin-top: -6px;
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .msg.err {
    color: var(--red);
  }
  .row3 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
</style>
