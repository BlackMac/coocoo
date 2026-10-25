<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import { href } from '../lib/router.svelte';
  import type { SecretInfo } from '../lib/protocol';
  import { KeyRound, Plus } from '../lib/icons';

  // A project's secrets: names, where they go (env var, files Coocoo wrote), who added them, with
  // Replace and Delete. Values are never sent to clients. Kept current by `project.secrets`.
  let { host, projectId }: { host: string; projectId: string } = $props();

  const secrets = $derived(fleet.secrets[key(host, projectId)]);

  let error = $state('');
  $effect(() => {
    if (fleet.conn !== 'open') return;
    fleet.loadSecrets(host, projectId).then(
      () => (error = ''),
      (err) => (error = err instanceof Error ? err.message : String(err)),
    );
  });

  const create = () => (ui.dialog = { kind: 'secret', hostId: host, projectId });
  const replace = (s: SecretInfo) => (ui.dialog = { kind: 'secret', hostId: host, projectId, name: s.name });

  function remove(s: SecretInfo) {
    const files = s.files.length ? ` and takes it out of ${s.files.map((f) => f.relPath).join(', ')}` : '';
    ui.dialog = {
      kind: 'confirm',
      title: 'Delete secret',
      message: `Delete ${s.name}? Agents of this project lose $${s.name} at their next start; this also removes it from Coocoo's store${files} (lines and files Coocoo wrote).`,
      action: 'Delete',
      danger: true,
      run: async () => {
        await fleet.call('secrets.remove', { projectId, name: s.name }, host, 60_000);
        const list = fleet.secrets[key(host, projectId)];
        if (list) fleet.secrets[key(host, projectId)] = list.filter((x) => x.name !== s.name);
      },
    };
  }

  const agoLabel = (ts: number) => {
    const a = ago(ts, now.value);
    return a === 'now' ? 'just now' : `${a} ago`;
  };
</script>

{#if error && !secrets}
  <div class="empty"><strong>Could not load the secrets</strong><span>{error}</span></div>
{:else if !secrets}
  <div class="empty"><span class="spinner"></span></div>
{:else if !secrets.length}
  <div class="empty">
    <KeyRound size={18} />
    <span>API keys and tokens for this project's agents. Agents ask for them in the chat (coocoo_secret_request); every agent gets them as env vars without seeing the value.</span>
    <button class="btn primary" onclick={create}><Plus size={14} /> New secret</button>
  </div>
{:else}
  {#each secrets as s (s.name)}
    <div class="row">
      <span class="icon"><KeyRound size={14} /></span>
      <span class="name mono ellipsis" title={s.name}>{s.name}</span>
      <div class="targets">
        <span class="chip mono">${s.name}</span>
        {#each s.files as f (f.path)}
          <span class="chip mono" title={f.path}>{f.branch ? `${f.relPath} · ${f.branch}` : f.relPath}</span>
        {/each}
      </div>
      <span class="who faint" title={new Date(s.updatedAt).toLocaleString()}>
        {#if s.addedBy}
          by agent {#if fleet.agent(host, s.addedBy.id)}<a href={href({ name: 'agent', host, id: s.addedBy.id })}>“{s.addedBy.title}”</a>{:else}“{s.addedBy.title}”{/if}
        {:else}
          by you
        {/if}
        · {agoLabel(s.updatedAt)}
      </span>
      <div class="acts">
        <button class="btn ghost sm" onclick={() => replace(s)}>Replace</button>
        <button class="btn ghost sm danger" onclick={() => remove(s)}>Delete</button>
      </div>
    </div>
  {/each}
  <p class="note">
    Values are never readable, not even here. Replace rewrites every listed file. Running agents get changes at their next process start.
  </p>
{/if}

<style>
  .empty {
    padding: 24px 16px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
    padding: 8px 12px;
    font-size: var(--fs-sm);
  }
  .row + .row {
    border-top: 1px solid var(--border);
  }
  .icon {
    display: flex;
    flex: none;
    color: var(--text-3);
  }
  .name {
    flex: 0 1 220px;
    font-weight: 500;
  }
  .targets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    flex: 1 1 200px;
    min-width: 0;
  }
  .who {
    flex: 0 1 auto;
    min-width: 0;
  }
  .who a {
    color: var(--accent-text);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
    margin-left: auto;
  }
  .note {
    margin: 0;
    padding: 8px 12px 10px;
    border-top: 1px solid var(--border);
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
</style>
