<script lang="ts">
  // The six headline numbers of the Stats tab, each with its breakdown, formula and source.
  import type { CommitMatch, UsageReport } from '../../lib/protocol';
  import { count, delta, pct, span, usd, type Delta } from '../format';
  import { insights } from '../store.svelte';
  import Kpi from '../Kpi.svelte';

  let { report }: { report: UsageReport } = $props();

  const t = $derived(report.totals);
  const prev = $derived(report.prev);
  const tokens = $derived(t.tokens.input + t.tokens.output + t.tokens.cacheRead + t.tokens.cacheWrite);
  const prevTokens = $derived(prev ? prev.tokens.input + prev.tokens.output + prev.tokens.cacheRead + prev.tokens.cacheWrite : undefined);

  /** Distinct commits of the listed agents by how they were attributed. */
  const byMatch = $derived.by(() => {
    const seen = new Map<string, CommitMatch>();
    for (const a of report.agents) for (const c of a.commits) if (!seen.has(c.sha)) seen.set(c.sha, c.match);
    const n: Record<CommitMatch, number> = { exact: 0, turn: 0, shared: 0 };
    for (const m of seen.values()) n[m]++;
    return n;
  });

  /** A change that is neither good nor bad (more tokens or requests is just more use). */
  function neutral(d: Delta | null): Delta | null {
    return d && { ...d, tone: 'flat' };
  }

  const vs = $derived(`vs ${insights.periodLabel}`);
  const lines = (n: number) => n.toLocaleString('en');
</script>

<div class="kpis">
  <Kpi label="Cost" value={usd(t.cost.total)} sub={prev ? `${vs} · API equivalent` : 'API equivalent'} delta={delta(t.cost.total, prev?.cost.total, false)}>
    {#snippet help()}
      <b>API equivalent</b>: the sum of omp's per-response <code>cost_total</code> at API list prices. On subscriptions (Claude Max,
      Codex, …) this is not what you were billed.
      <table>
        <tbody>
          <tr><td>Input</td><td>{usd(t.cost.input)}</td></tr>
          <tr><td>Output</td><td>{usd(t.cost.output)}</td></tr>
          <tr><td>Cache read</td><td>{usd(t.cost.cacheRead)}</td></tr>
          <tr><td>Cache write</td><td>{usd(t.cost.cacheWrite)}</td></tr>
          {#if prev}<tr><td>Previous period</td><td>{usd(prev.cost.total)}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">Cost = input + output + cache read + cache write. Source: <code>~/.omp/stats.db</code> of each host.</div>
    {/snippet}
  </Kpi>
  <Kpi
    label="Tokens"
    value={count(tokens)}
    sub="{pct(tokens ? t.tokens.cacheRead / tokens : undefined)} from cache · {count(t.tokens.output)} out"
    delta={neutral(delta(tokens, prevTokens))}
  >
    {#snippet help()}
      Tokens = input + output + cache read + cache write, over every response of the agents, their subagents and advisor.
      <table>
        <tbody>
          <tr><td>Input</td><td>{count(t.tokens.input)}</td></tr>
          <tr><td>Output</td><td>{count(t.tokens.output)}</td></tr>
          <tr><td>Cache read</td><td>{count(t.tokens.cacheRead)}</td></tr>
          <tr><td>Cache write</td><td>{count(t.tokens.cacheWrite)}</td></tr>
          {#if prevTokens != null}<tr><td>Previous period</td><td>{count(prevTokens)}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">“From cache” = cache read / tokens. Source: <code>~/.omp/stats.db</code>.</div>
    {/snippet}
  </Kpi>
  <Kpi
    label="Requests"
    value={count(t.requests)}
    sub="{count(t.prompts)} prompts · {pct(t.requests ? t.failed / t.requests : undefined)} failed"
    delta={neutral(delta(t.requests, prev?.requests))}
  >
    {#snippet help()}
      <b>Requests</b>: assistant responses, one per model call (main agents, subagents and advisor). <b>Prompts</b>: messages you sent to
      main agents.
      <table>
        <tbody>
          <tr><td>Requests</td><td>{t.requests.toLocaleString('en')}</td></tr>
          <tr><td>Prompts</td><td>{t.prompts.toLocaleString('en')}</td></tr>
          <tr><td>Failed (stop reason error)</td><td>{t.failed.toLocaleString('en')}</td></tr>
          <tr><td>Aborted</td><td>{t.aborted.toLocaleString('en')}</td></tr>
          {#if prev}<tr><td>Previous period</td><td>{prev.requests.toLocaleString('en')}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">Failed % = failed / requests. Source: <code>~/.omp/stats.db</code>.</div>
    {/snippet}
  </Kpi>
  <Kpi label="Commits" value={count(t.commits)} sub="+{lines(t.linesAdded)} −{lines(t.linesDeleted)} lines" delta={delta(t.commits, prev?.commits)}>
    {#snippet help()}
      Commits attributed to an agent, each counted once.
      <table>
        <tbody>
          <tr><td>Exact (Kanban done)</td><td>{byMatch.exact}</td></tr>
          <tr><td>During one of its turns (±60 s)</td><td>{byMatch.turn}</td></tr>
          <tr><td>Shared by several agents</td><td>{byMatch.shared}</td></tr>
          {#if prev}<tr><td>Previous period</td><td>{prev.commits}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">Lines: + / − from <code>git --shortstat</code> of each commit. Split counted over the agents listed below.</div>
    {/snippet}
  </Kpi>
  <Kpi label="Agents" value={count(t.agents)} sub="+ {count(t.subagents)} subagents · {span(t.activeMs)} active" delta={delta(t.agents, prev?.agents)}>
    {#snippet help()}
      Coocoo agents with at least one request in the period (deleted ones included).
      <table>
        <tbody>
          <tr><td>Agents</td><td>{t.agents}</td></tr>
          <tr><td>Subagent sessions</td><td>{t.subagents}</td></tr>
          <tr><td>Active time</td><td>{span(t.activeMs)}</td></tr>
          {#if prev}<tr><td>Previous period</td><td>{prev.agents}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">Active time: from each prompt to the last response of its turn, summed over agents; gaps over 10 min are not counted.</div>
    {/snippet}
  </Kpi>
  <Kpi label="Active days" value="{t.activeDays} / {report.days.length}" sub="streak {t.streak} {t.streak === 1 ? 'day' : 'days'}" delta={delta(t.activeDays, prev?.activeDays)}>
    {#snippet help()}
      Days with at least one prompt or request, out of the days in the period, in your time zone.
      <table>
        <tbody>
          <tr><td>Active days</td><td>{t.activeDays}</td></tr>
          <tr><td>Days in the period</td><td>{report.days.length}</td></tr>
          <tr><td>Longest streak</td><td>{t.streak}</td></tr>
          {#if prev}<tr><td>Previous period</td><td>{prev.activeDays}</td></tr>{/if}
        </tbody>
      </table>
      <div class="src">Streak = longest run of consecutive active days in the period.</div>
    {/snippet}
  </Kpi>
</div>

<style>
  .kpis {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 10px;
  }
  .src {
    margin-top: 6px;
    color: var(--text-2);
  }
  @media (max-width: 1100px) {
    .kpis {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @media (max-width: 600px) {
    .kpis {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 8px;
    }
  }
</style>
