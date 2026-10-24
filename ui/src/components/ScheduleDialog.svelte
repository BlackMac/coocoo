<script lang="ts" module>
  import type { Schedule } from '../lib/protocol';

  /** Timing presets of the editor; `once` is an `at` schedule, the others map to cron. */
  export type Preset = 'once' | 'hourly' | 'daily' | 'weekdays' | 'weekly' | 'monthly' | 'custom';

  /** A cron expression that is exactly one of the recurring presets. */
  export interface CronTiming {
    preset: 'hourly' | 'daily' | 'weekdays' | 'weekly' | 'monthly';
    minute: number;
    hour: number;
    /** 0 = Sunday … 6 = Saturday. */
    weekday: number;
    /** Day of month, 1–31. */
    day: number;
  }

  export const browserZone = Intl.DateTimeFormat().resolvedOptions().timeZone;

  export const pad = (n: number) => String(n).padStart(2, '0');

  const MACROS: Record<string, string> = {
    '@hourly': '0 * * * *',
    '@daily': '0 0 * * *',
    '@weekly': '0 0 * * 0',
    '@monthly': '0 0 1 * *',
  };
  const DAY_NAMES = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
  const WEEKDAYS_PLURAL = ['Sundays', 'Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays'];

  function num(s: string, min: number, max: number): number | undefined {
    if (!/^\d{1,2}$/.test(s)) return undefined;
    const n = Number(s);
    return n >= min && n <= max ? n : undefined;
  }

  /** The preset a cron expression is exactly, if any (`@hourly`/`@daily`/`@weekly`/`@monthly` included). */
  export function parseCron(cron: string): CronTiming | undefined {
    const expr = MACROS[cron.trim().toLowerCase()] ?? cron.trim();
    const f = expr.split(/\s+/);
    if (f.length !== 5) return undefined;
    const [mi, ho, dom, mon, dowRaw] = f;
    const dow = dowRaw.toLowerCase();
    const minute = num(mi, 0, 59);
    if (minute === undefined || mon !== '*') return undefined;
    if (ho === '*') return dom === '*' && dow === '*' ? { preset: 'hourly', minute, hour: 0, weekday: 1, day: 1 } : undefined;
    const hour = num(ho, 0, 23);
    if (hour === undefined) return undefined;
    const base = { minute, hour, weekday: 1, day: 1 };
    if (dom === '*') {
      if (dow === '*') return { ...base, preset: 'daily' };
      if (dow === '1-5' || dow === 'mon-fri') return { ...base, preset: 'weekdays' };
      const named = DAY_NAMES.indexOf(dow);
      const d = named >= 0 ? named : num(dow, 0, 7);
      return d === undefined ? undefined : { ...base, preset: 'weekly', weekday: d % 7 };
    }
    const day = num(dom, 1, 31);
    return day !== undefined && dow === '*' ? { ...base, preset: 'monthly', day } : undefined;
  }

  export function ordinal(n: number): string {
    const s = n % 100 >= 11 && n % 100 <= 13 ? 'th' : (['th', 'st', 'nd', 'rd'][n % 10] ?? 'th');
    return `${n}${s}`;
  }

  /** "Daily at 08:00", "Once on 1 Nov 2026, 09:00", …; undefined for a cron that fits no preset. */
  export function describeTiming(s: Pick<Schedule, 'cron' | 'at'>): string | undefined {
    if (s.at != null) {
      const when = new Date(s.at).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
      return `Once on ${when}`;
    }
    const t = s.cron ? parseCron(s.cron) : undefined;
    if (!t) return undefined;
    const time = `${pad(t.hour)}:${pad(t.minute)}`;
    switch (t.preset) {
      case 'hourly':
        return `Hourly at :${pad(t.minute)}`;
      case 'daily':
        return `Daily at ${time}`;
      case 'weekdays':
        return `Weekdays at ${time}`;
      case 'weekly':
        return `${WEEKDAYS_PLURAL[t.weekday]} at ${time}`;
      case 'monthly':
        return `Monthly on the ${ordinal(t.day)} at ${time}`;
    }
  }

  /** `datetime-local` value of a time, in the browser's zone. */
  function localInput(ts: number): string {
    const d = new Date(ts);
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  const ZONES = typeof Intl.supportedValuesOf === 'function' ? Intl.supportedValuesOf('timeZone') : [];
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { ModelInfo } from '../lib/protocol';
  import Dialog from './Dialog.svelte';
  import ModelPicker from './ModelPicker.svelte';
  import { GitBranch } from '../lib/icons';

  // Creates or edits a project's scheduled run. Props are fixed for the dialog's lifetime.
  let { hostId, projectId, scheduleId }: { hostId: string; projectId: string; scheduleId?: string } = $props();

  const PRESETS: { id: Preset; label: string }[] = [
    { id: 'once', label: 'Once' },
    { id: 'hourly', label: 'Hourly' },
    { id: 'daily', label: 'Daily' },
    { id: 'weekdays', label: 'Weekdays (Mon–Fri)' },
    { id: 'weekly', label: 'Weekly' },
    { id: 'monthly', label: 'Monthly' },
    { id: 'custom', label: 'Custom (cron)' },
  ];
  // Monday first, as a week reads.
  const WEEKDAY_OPTIONS = [1, 2, 3, 4, 5, 6, 0].map((d) => ({ value: d, label: ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'][d] }));

  const existing = untrack(() => (scheduleId ? fleet.schedules[key(hostId, projectId)]?.find((s) => s.id === scheduleId) : undefined));
  const seed = untrack(() => {
    const t = existing?.cron ? parseCron(existing.cron) : undefined;
    const preset: Preset = existing ? (existing.at != null ? 'once' : (t?.preset ?? 'custom')) : 'daily';
    const nextHour = Math.ceil((Date.now() + 60_000) / 3_600_000) * 3_600_000;
    return {
      preset,
      minute: t?.minute ?? 0,
      time: t && t.preset !== 'hourly' ? `${pad(t.hour)}:${pad(t.minute)}` : '09:00',
      weekday: t?.weekday ?? 1,
      day: t?.day ?? 1,
      onceAt: localInput(existing?.at ?? nextHour),
      cron: existing?.cron ?? '',
    };
  });

  let name = $state(existing?.name ?? '');
  let prompt = $state(existing?.prompt ?? '');
  let preset = $state<Preset>(seed.preset);
  let time = $state(seed.time);
  let minute = $state(seed.minute);
  let weekday = $state(seed.weekday);
  let day = $state(seed.day);
  let onceAt = $state(seed.onceAt);
  let cron = $state(seed.cron);
  let timezone = $state(existing?.timezone ?? browserZone);
  let target = $state(existing?.agentId ?? '');
  let worktree = $state(existing?.worktree ?? false);
  let model = $state(existing?.model ?? '');
  let enabled = $state(existing?.enabled ?? true);
  let busy = $state(false);
  let models = $state<ModelInfo[]>([]);
  let modelsError = $state('');

  const isGit = $derived(!!fleet.project(hostId, projectId)?.git);
  const agents = $derived(fleet.agentsOf(hostId, projectId).filter((a) => a.status !== 'stopped'));
  /** The saved target agent when it is archived or gone: kept selectable so editing does not drop it. */
  const missingTarget = $derived(existing?.agentId && !agents.some((a) => a.id === existing.agentId) ? existing : undefined);

  /** The cron expression the form describes; undefined for `once` or an incomplete form. */
  const cronExpr = $derived.by(() => {
    if (preset === 'once') return undefined;
    if (preset === 'custom') return cron.trim() || undefined;
    if (preset === 'hourly') return Number.isInteger(minute) && minute >= 0 && minute <= 59 ? `${minute} * * * *` : undefined;
    const m = /^(\d{1,2}):(\d{2})/.exec(time);
    if (!m) return undefined;
    const hm = `${Number(m[2])} ${Number(m[1])}`;
    if (preset === 'daily') return `${hm} * * *`;
    if (preset === 'weekdays') return `${hm} * * 1-5`;
    if (preset === 'weekly') return `${hm} * * ${weekday}`;
    return Number.isInteger(day) && day >= 1 && day <= 31 ? `${hm} ${day} * *` : undefined;
  });
  const atMs = $derived(preset === 'once' && onceAt ? new Date(onceAt).getTime() : NaN);
  const zone = $derived(timezone.trim() || browserZone);
  /** Whether saving changes when it runs (a finished one-off can be renamed without a new time). */
  const timingChanged = $derived(!existing || (preset === 'once' ? existing.at !== atMs : existing.cron !== cronExpr || existing.timezone !== zone));
  const onceError = $derived(preset !== 'once' || !timingChanged ? '' : !Number.isFinite(atMs) ? 'Pick a date and time' : atMs <= Date.now() ? 'Pick a time in the future' : '');

  function setPreset(next: Preset) {
    // Switching to Custom starts from what the form described.
    if (next === 'custom' && !cron.trim() && cronExpr) cron = cronExpr;
    preset = next;
  }

  $effect(() => {
    fleet
      .loadModels(hostId)
      .then((m) => (models = m))
      .catch((e) => (modelsError = e instanceof Error ? e.message : String(e)));
  });

  let preview = $state<{ next: number[]; zone: string } | null>(null);
  let previewError = $state('');
  $effect(() => {
    const expr = cronExpr;
    const tz = zone;
    if (!expr) {
      preview = null;
      previewError = '';
      return;
    }
    let stale = false;
    const timer = setTimeout(() => {
      fleet.call('schedules.preview', { cron: expr, timezone: tz, count: 3 }, hostId).then(
        (r) => {
          if (stale) return;
          preview = { next: r.next, zone: tz };
          previewError = '';
        },
        (err) => {
          if (stale) return;
          preview = null;
          previewError = err instanceof Error ? err.message : String(err);
        },
      );
    }, 300);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  });

  function fmtRun(ts: number, timeZone?: string): string {
    const opts: Intl.DateTimeFormatOptions = { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' };
    try {
      return new Date(ts).toLocaleString(undefined, { ...opts, timeZone });
    } catch {
      return new Date(ts).toLocaleString(undefined, opts);
    }
  }

  async function save(e?: SubmitEvent) {
    e?.preventDefault();
    if (busy) return;
    if (!name.trim()) return ui.toast('error', 'Give the schedule a name');
    if (!prompt.trim()) return ui.toast('error', 'Write the prompt the agent gets');
    if (preset === 'once') {
      if (onceError) return ui.toast('error', onceError);
    } else {
      if (!cronExpr) return ui.toast('error', preset === 'custom' ? 'Enter a cron expression' : 'Complete when it runs');
      if (previewError) return ui.toast('error', 'Fix when it runs', previewError);
    }
    const timing = !timingChanged ? {} : preset === 'once' ? { at: atMs } : { cron: cronExpr, timezone: zone };
    const agent = target
      ? { agentId: target }
      : { agentId: existing ? '' : undefined, worktree: isGit && worktree, model: model || (existing ? '' : undefined) };
    busy = true;
    try {
      const base = { projectId, name: name.trim(), prompt: prompt.trim(), ...timing, ...agent };
      const s = existing
        ? await fleet.call('schedules.update', { ...base, scheduleId: existing.id, enabled }, hostId, 60_000)
        : await fleet.call('schedules.add', base, hostId, 60_000);
      fleet.putSchedule(hostId, s);
      ui.submitDialog();
    } catch (err) {
      ui.error(existing ? 'Could not save the schedule' : 'Could not create the schedule', err);
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    }
  }
</script>

<Dialog title={scheduleId ? 'Edit schedule' : 'New schedule'} width={560} onclose={() => (ui.dialog = null)}>
  {#if scheduleId && !existing}
    <div class="empty"><strong>Schedule not found</strong><span>It may have been deleted.</span></div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form class="form" id="schedule" onsubmit={save} {onkeydown}>
      <label class="field">
        <span>Name</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input" bind:value={name} placeholder="Daily review" autofocus />
      </label>
      <label class="field">
        <span>Prompt</span>
        <textarea class="textarea" rows="5" bind:value={prompt} placeholder="What should the agent do on every run?"></textarea>
      </label>

      <div class="when">
        <label class="field preset">
          <span>When</span>
          <select class="select" value={preset} onchange={(e) => setPreset(e.currentTarget.value as Preset)}>
            {#each PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
          </select>
        </label>
        {#if preset === 'once'}
          <label class="field">
            <span>Date and time</span>
            <input class="input" type="datetime-local" bind:value={onceAt} min={localInput(Date.now())} />
          </label>
        {:else if preset === 'hourly'}
          <label class="field small">
            <span>Minute</span>
            <input class="input num" type="number" min="0" max="59" step="1" bind:value={minute} />
          </label>
        {:else if preset === 'custom'}
          <label class="field cron">
            <span>Cron expression</span>
            <input class="input mono" bind:value={cron} placeholder="0 9 * * 1-5" spellcheck="false" autocomplete="off" />
          </label>
        {:else}
          {#if preset === 'weekly'}
            <label class="field">
              <span>Day</span>
              <select class="select" bind:value={weekday}>
                {#each WEEKDAY_OPTIONS as d (d.value)}<option value={d.value}>{d.label}</option>{/each}
              </select>
            </label>
          {:else if preset === 'monthly'}
            <label class="field small">
              <span>Day of month</span>
              <select class="select" bind:value={day}>
                {#each Array.from({ length: 31 }, (_, i) => i + 1) as d (d)}<option value={d}>{ordinal(d)}</option>{/each}
              </select>
            </label>
          {/if}
          <label class="field small">
            <span>Time</span>
            <input class="input" type="time" bind:value={time} required />
          </label>
        {/if}
      </div>
      {#if preset === 'custom'}
        <span class="hint">minute hour day-of-month month day-of-week, e.g. <span class="mono">30 7 * * mon,thu</span> or <span class="mono">0 */4 * * *</span></span>
      {/if}
      {#if preset === 'once'}
        {#if onceError}<span class="hint err">{onceError}</span>{:else}<span class="hint">In your time zone ({browserZone}).</span>{/if}
      {:else}
        <label class="field">
          <span>Time zone</span>
          <input class="input" bind:value={timezone} list="schedule-zones" placeholder={browserZone} spellcheck="false" autocomplete="off" />
          {#if ZONES.length}
            <datalist id="schedule-zones">{#each ZONES as z (z)}<option value={z}></option>{/each}</datalist>
          {/if}
        </label>
        <div class="preview">
          {#if previewError}
            <span class="err">{previewError}</span>
          {:else if preview}
            <span class="faint">Next runs ({preview.zone}):</span>
            <ul>
              {#each preview.next as ts (ts)}
                <li class="num">
                  {fmtRun(ts, preview.zone)}
                  {#if preview.zone !== browserZone}<span class="faint"> · {fmtRun(ts)} your time</span>{/if}
                </li>
              {/each}
            </ul>
          {:else}
            <span class="faint">Next runs…</span>
          {/if}
        </div>
      {/if}

      <label class="field">
        <span>Agent</span>
        <select class="select" bind:value={target}>
          <option value="">New agent for each run</option>
          {#if agents.length || missingTarget}
            <optgroup label="Prompt an existing agent">
              {#each agents as a (a.id)}<option value={a.id}>{a.title}</option>{/each}
              {#if missingTarget}<option value={missingTarget.agentId}>{fleet.agent(hostId, missingTarget.agentId!)?.title ?? missingTarget.agentTitle ?? 'Deleted agent'} (archived or deleted)</option>{/if}
            </optgroup>
          {/if}
        </select>
        <span class="hint">
          {target
            ? 'It gets the prompt like a normal message; a run is skipped while the agent is busy.'
            : 'Each run starts a fresh agent named after the schedule.'}
          Runs only while the host's daemon is running; a missed run happens once when it starts again.
        </span>
      </label>
      {#if !target}
        <div class="wt">
          {#if isGit}
            <label class="switch">
              <input type="checkbox" bind:checked={worktree} />
              <span><GitBranch size={13} /> Run each agent in a new git worktree</span>
            </label>
          {/if}
          <div class="field">
            <span>Model</span>
            <ModelPicker {models} value={model} onselect={(v) => (model = v)} defaultLabel="Default" variant="field" />
            {#if modelsError}<span class="hint">{modelsError}</span>{/if}
          </div>
        </div>
      {/if}
      {#if existing}
        <label class="switch">
          <input type="checkbox" bind:checked={enabled} />
          <span>Enabled</span>
        </label>
      {/if}
    </form>
  {/if}
  {#snippet footer()}
    <span class="hint grow">⌘↵ to save</span>
    <button class="btn" type="button" onclick={() => (ui.dialog = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="schedule" disabled={busy || (!!scheduleId && !existing)}>
      {#if busy}<span class="spinner"></span>{/if}
      {existing ? 'Save' : 'Create schedule'}
    </button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .when {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .when > .field {
    flex: 1 1 160px;
    min-width: 0;
  }
  .when > .field.small {
    flex: 0 1 130px;
  }
  .when > .field.cron {
    flex: 2 1 220px;
  }
  .preview {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  .preview ul {
    margin: 0;
    padding-left: 18px;
  }
  .err {
    color: var(--red);
  }
  .wt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
  }
  .switch span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .grow {
    margin-right: auto;
    align-self: center;
  }
  textarea {
    min-height: 96px;
  }
</style>
