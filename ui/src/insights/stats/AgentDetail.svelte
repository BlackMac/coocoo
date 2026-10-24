<script lang="ts">
  // Expanded agent row: where its cost came from (main session, subagents, advisor; token kinds),
  // its omp session files, the commits attributed to it and its cost per day.
  import type { InsightAgentRow, InsightDay } from '../../lib/protocol';
  import { copyText } from '../../lib/clipboard';
  import { shortPath } from '../../lib/format';
  import { href } from '../../lib/router.svelte';
  import { track } from '../../lib/track';
  import { ui } from '../../lib/ui.svelte';
  import { count, shortDate, usd } from '../format';
  import Tag from './Tag.svelte';

  let { agent: a, days }: { agent: InsightAgentRow; days: InsightDay[] } = $props();

  const COMMITS = 5;
  let allCommits = $state(false);

  const commits = $derived(allCommits ? a.commits : a.commits.slice(0, COMMITS));
  const maxDay = $derived(Math.max(0, ...a.dayCost));
  const reqs = (k: string) => `${count(a.requestsBy[k] ?? 0)} requests`;

  function copy(text: string) {
    copyText(text).then(() => ui.toast('success', 'Copied'), (err) => ui.error('Copy failed', err));
  }
</script>

<div class="dgrid">
  <div>
    <h5>Cost breakdown</h5>
    <div class="kv num">
      <span>Main agent</span><span>{usd(a.costBy.main)} · {reqs('main')}</span>
      <span>Subagents ({a.subagentSessions})</span><span>{usd(a.costBy.subagents)} · {reqs('subagent')}</span>
      <span>Advisor</span><span>{usd(a.costBy.advisor)} · {reqs('advisor')}</span>
      <span>Input / Output</span><span>{usd(a.cost.input)} / {usd(a.cost.output)}</span>
      <span>Cache read / write</span><span>{usd(a.cost.cacheRead)} / {usd(a.cost.cacheWrite)}</span>
      <span>Requests</span><span>{count(a.requests)}{a.failed ? ` · ${a.failed} failed` : ''}</span>
      <span>Session files</span>
      <span class="files">
        {#each a.sessionFiles as f, i (i)}
          <span class="file"><span class="mono ellipsis" title={f}>{shortPath(f, 1)}</span><button class="link" onclick={() => copy(f)}>copy</button></span>
        {:else}
          <span class="faint">—</span>
        {/each}
      </span>
    </div>
  </div>

  <div>
    <h5>Commits</h5>
    {#each commits as c, i (i)}
      <div class="commit">
        <span class="mono sha" title={new Date(c.ts).toLocaleString()}>{c.sha.slice(0, 7)}</span>
        <span class="sum ellipsis" title={c.summary}>{c.summary}</span>
        <Tag kind={c.match} />
      </div>
    {:else}
      <div class="faint small">No commits attributed in this period.</div>
    {/each}
    {#if a.commits.length}
      <div class="faint small foot">
        {#if a.commits.length > COMMITS}
          <button class="link" onclick={() => (allCommits = !allCommits)}>{allCommits ? 'Show fewer' : `+${a.commits.length - COMMITS} more`}</button> ·
        {/if}
        <span class="add">+{a.linesAdded.toLocaleString('en')}</span> <span class="del">−{a.linesDeleted.toLocaleString('en')}</span> lines
      </div>
    {/if}
  </div>

  <div>
    <h5>Per day</h5>
    {#if maxDay > 0}
      <svg class="mini" viewBox="0 0 {a.dayCost.length * 10} 60" preserveAspectRatio="none" aria-label="Cost per day">
        {#each a.dayCost as v, i (i)}
          <rect x={i * 10 + 1.5} y={60 - (56 * v) / maxDay} width="7" height={(56 * v) / maxDay} fill="var(--accent)">
            <title>{days[i] ? shortDate(days[i].start) : ''} · {usd(v)}</title>
          </rect>
        {/each}
      </svg>
    {:else}
      <div class="faint small">No cost in this period.</div>
    {/if}
    {#if !a.deleted}
      <a class="link open" href={href({ name: 'agent', host: a.hostId, id: a.id })} onclick={() => track('example', { agentId: a.id, meta: { section: 'stats' } })}
        >Open agent →</a
      >
    {/if}
  </div>
</div>

<style>
  .dgrid {
    display: grid;
    grid-template-columns: 1.1fr 1fr 1fr;
    gap: 18px;
    min-width: 0;
  }
  .dgrid > div {
    min-width: 0;
  }
  h5 {
    margin: 0 0 6px;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .kv {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 12px;
    font-size: var(--fs-sm);
  }
  .kv > span:nth-child(odd) {
    color: var(--text-3);
  }
  .files {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .file {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .link {
    all: unset;
    color: var(--accent-text);
    cursor: pointer;
    white-space: nowrap;
  }
  .link:hover {
    text-decoration: underline;
  }
  .commit {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .sha {
    flex: none;
    color: var(--text-2);
  }
  .sum {
    flex: 1;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .foot {
    margin-top: 4px;
  }
  .mini {
    display: block;
    width: 100%;
    height: 60px;
  }
  .open {
    display: inline-block;
    margin-top: 8px;
    font-size: var(--fs-sm);
  }
  @media (max-width: 900px) {
    .dgrid {
      grid-template-columns: minmax(0, 1fr);
      gap: 14px;
    }
  }
</style>
