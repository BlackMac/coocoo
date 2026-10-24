<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago, modelName, until } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import { href } from '../lib/router.svelte';
  import type { Schedule } from '../lib/protocol';
  import { CalendarClock, Clock, GitBranch, Pencil, Play, Plus, Trash2 } from '../lib/icons';
  import { browserZone, describeTiming } from '../components/ScheduleDialog.svelte';

  // A project's scheduled runs: when, which agent, the next and the last run, with actions to
  // pause, run now, edit and delete. Kept current by `project.schedules`.
  let { host, projectId }: { host: string; projectId: string } = $props();

  const schedules = $derived(fleet.schedules[key(host, projectId)]);

  let error = $state('');
  $effect(() => {
    if (fleet.conn !== 'open') return;
    fleet.loadSchedules(host, projectId).then(
      () => (error = ''),
      (err) => (error = err instanceof Error ? err.message : String(err)),
    );
  });

  async function run<T>(label: string, f: () => Promise<T>): Promise<T | undefined> {
    try {
      return await f();
    } catch (err) {
      ui.error(label, err);
      return undefined;
    }
  }

  const create = () => (ui.dialog = { kind: 'schedule', hostId: host, projectId });
  const edit = (s: Schedule) => (ui.dialog = { kind: 'schedule', hostId: host, projectId, scheduleId: s.id });

  async function setEnabled(s: Schedule, box: HTMLInputElement) {
    const enabled = box.checked;
    const r = await run(enabled ? 'Could not enable the schedule' : 'Could not pause the schedule', () =>
      fleet.call('schedules.update', { projectId, scheduleId: s.id, enabled }, host),
    );
    if (r) fleet.putSchedule(host, r);
    else box.checked = s.enabled;
  }

  let running = $state<Record<string, boolean>>({});
  async function runNow(s: Schedule) {
    running[s.id] = true;
    const r = await run('Could not run the schedule', () => fleet.call('schedules.run', { projectId, scheduleId: s.id }, host, 60_000));
    delete running[s.id];
    if (r) fleet.putSchedule(host, r);
  }

  function remove(s: Schedule) {
    ui.dialog = {
      kind: 'confirm',
      title: 'Delete schedule',
      message: `Delete the schedule “${s.name}”? Agents it started stay.`,
      action: 'Delete',
      danger: true,
      run: async () => {
        await fleet.call('schedules.remove', { projectId, scheduleId: s.id }, host);
        const list = fleet.schedules[key(host, projectId)];
        if (list) fleet.schedules[key(host, projectId)] = list.filter((x) => x.id !== s.id);
      },
    };
  }

  const agoLabel = (ts: number) => {
    const a = ago(ts, now.value);
    return a === 'now' ? 'just now' : `${a} ago`;
  };
  const nextLabel = (ts: number) => {
    const u = until(ts, now.value);
    if (u !== 'now') return `in ${u}`;
    return ts > now.value ? 'in < 1m' : 'due now';
  };
  /** Without a next run: a one-off whose time has come is done, anything else disabled is paused. */
  const idleLabel = (s: Schedule) => (!s.enabled && !(s.at != null && s.at <= now.value) ? 'Paused' : 'Done');
  const OUTCOME = { started: 'Started', skipped: 'Skipped', failed: 'Failed' } as const;
</script>

{#if error && !schedules}
  <div class="empty"><strong>Could not load the schedules</strong><span>{error}</span></div>
{:else if !schedules}
  <div class="empty"><span class="spinner"></span></div>
{:else if !schedules.length}
  <div class="empty">
    <CalendarClock size={18} />
    <span>Run an agent on a schedule: a daily review, a weekly dependency check, or a one-off run later.</span>
    <button class="btn primary" onclick={create}><Plus size={14} /> New schedule</button>
  </div>
{:else}
  {#each schedules as s (s.id)}
    {@const timing = describeTiming(s)}
    {@const live = s.agentId ? fleet.agent(host, s.agentId) : undefined}
    {@const last = s.lastRun}
    <div class="row" class:off={!s.enabled}>
      <span class="icon"><Clock size={14} /></span>
      <div class="main">
        <button class="name ellipsis" title={s.prompt} onclick={() => edit(s)}>{s.name}</button>
        <div class="meta">
          {#if timing}<span>{timing}</span>{:else}<span class="mono">{s.cron}</span>{/if}
          {#if s.cron && s.timezone !== browserZone}<span class="faint">{s.timezone}</span>{/if}
          <span class="sep faint">·</span>
          {#if s.agentId}
            {#if live}
              <a class="agent ellipsis" href={href({ name: 'agent', host, id: s.agentId })}>{live.title}</a>
            {:else}
              <span class="faint ellipsis" title="The agent is deleted">{s.agentTitle ?? 'Deleted agent'}</span>
            {/if}
          {:else}
            <span class="target">
              New agent
              {#if s.worktree}<span class="faint" title="In its own git worktree"><GitBranch size={12} /></span>{/if}
              {#if s.model}<span class="faint mono">{modelName(s.model)}</span>{/if}
            </span>
          {/if}
        </div>
      </div>
      <div class="runs">
        {#if s.nextRunAt}
          <span title={new Date(s.nextRunAt).toLocaleString()}>Next {nextLabel(s.nextRunAt)}</span>
        {:else}
          <span class="faint">{idleLabel(s)}</span>
        {/if}
        {#if last}
          {@const lastAgent = last.agentId ? fleet.agent(host, last.agentId) : undefined}
          <span class="last" class:warn={last.outcome === 'skipped'} class:bad={last.outcome === 'failed'} title={new Date(last.at).toLocaleString()}>
            {OUTCOME[last.outcome]} {agoLabel(last.at)}{last.manual ? ' (run now)' : ''}
            {#if last.outcome === 'started' && last.agentId}
              {#if lastAgent}
                → <a class="agent" href={href({ name: 'agent', host, id: last.agentId })}>{lastAgent.title}</a>
              {:else if last.agentTitle}
                → {last.agentTitle}
              {/if}
            {:else if last.message}
              — {last.message}
            {/if}
          </span>
        {/if}
      </div>
      <div class="acts">
        <label class="switch" title={s.enabled ? 'Pause' : 'Enable'}>
          <input type="checkbox" checked={s.enabled} onchange={(e) => setEnabled(s, e.currentTarget)} aria-label="Enabled" />
        </label>
        <button class="btn ghost icon sm" title="Run now" aria-label="Run now" disabled={running[s.id]} onclick={() => runNow(s)}>
          {#if running[s.id]}<span class="spinner"></span>{:else}<Play size={13} />{/if}
        </button>
        <button class="btn ghost icon sm" title="Edit schedule" aria-label="Edit schedule" onclick={() => edit(s)}><Pencil size={13} /></button>
        <button class="btn ghost icon sm" title="Delete schedule" aria-label="Delete schedule" onclick={() => remove(s)}><Trash2 size={13} /></button>
      </div>
    </div>
  {/each}
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
  .row.off .icon,
  .row.off .name {
    color: var(--text-3);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1 1 220px;
    min-width: 0;
  }
  .name {
    align-self: flex-start;
    max-width: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }
  .name:hover {
    text-decoration: underline;
  }
  .meta,
  .target {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 6px;
    min-width: 0;
    color: var(--text-2);
  }
  .meta > * {
    max-width: 100%;
  }
  .agent {
    color: var(--accent-text);
  }
  .runs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 0 1 260px;
    min-width: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .warn {
    color: var(--amber);
  }
  .bad {
    color: var(--red);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
    margin-left: auto;
  }
  .acts .switch {
    margin-right: 4px;
  }
  .acts .spinner {
    width: 12px;
    height: 12px;
  }
  @media (max-width: 640px) {
    .runs {
      flex-basis: 100%;
      padding-left: 26px;
    }
  }
</style>
