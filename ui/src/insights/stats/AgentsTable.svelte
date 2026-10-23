<script lang="ts">
  // Agents (deleted ones included): what each cost and produced. A row expands into its cost
  // breakdown, session files, commits and cost per day.
  import type { InsightAgentRow, UsageReport } from '../../lib/protocol';
  import { modelName } from '../../lib/format';
  import { count, span, usd } from '../format';
  import AgentDetail from './AgentDetail.svelte';
  import Section from './Section.svelte';
  import Table from './Table.svelte';
  import Tag from './Tag.svelte';
  import { hostName, projectName } from './util';

  let { report }: { report: UsageReport } = $props();

  let openKey = $state<string | null>(null);

  const maxCost = $derived(Math.max(0, ...report.agents.map((a) => a.cost.total)));
  const key = (a: InsightAgentRow) => `${a.hostId}:${a.id}`;

  const cols = [
    { label: 'Agent' },
    { label: 'Project', left: true },
    { label: 'Host', left: true, hm: true },
    { label: 'Models', hm: true, title: 'Most requests first' },
    { label: 'Prompts', title: 'Messages you sent' },
    { label: 'Tool calls', hm: true },
    { label: 'Active', hm: true, title: 'Prompt to last response of each turn; gaps over 10 min not counted' },
    { label: 'Cost', title: 'API equivalent incl. subagents and advisor' },
    { label: 'Commits', title: 'Exact (Kanban), during a turn, or shared' },
    { label: 'Cards', hm: true, title: 'Cards of this agent that entered Done' },
  ];
</script>

<Section title="Agents" note="incl. subagents · click to expand">
  {#snippet help()}
    Every Coocoo agent with requests in the period; its subagents and advisor calls are added to it. Deleted agents stay listed.
    <b>Cost</b>: API equivalent. <b>Active</b>: from each prompt to the last response of its turn, gaps over 10 min not counted.
    <b>Commits</b>: recorded by Kanban, or made in the agent's repository during one of its turns (±60 s). Expand a row for the
    breakdown.
  {/snippet}
  <Table
    {cols}
    rows={report.agents}
    {key}
    onrow={(a) => (openKey = openKey === key(a) ? null : key(a))}
    open={(a) => openKey === key(a)}
    limit={25}
    empty="No agents with requests in this period."
  >
    {#snippet row(a: InsightAgentRow)}
      <td class="t">
        <span class="caret faint">{openKey === key(a) ? '▾' : '▸'}</span>
        {a.title || 'Untitled agent'}
        {#if a.deleted}<Tag kind="deleted" />{:else if a.archived}<Tag kind="archived" />{/if}
      </td>
      <td class="left proj">{projectName(report, a.hostId, a.projectId)}</td>
      <td class="left hm">{hostName(report, a.hostId)}</td>
      <td class="hm faint" title={a.models.join(', ')}>{a.models.length ? modelName(a.models[0]) : '—'}{a.models.length > 1 ? ` +${a.models.length - 1}` : ''}</td>
      <td data-l="prompts">{a.prompts}</td>
      <td class="hm">{count(a.toolCalls)}</td>
      <td class="hm">{a.activeMs ? span(a.activeMs) : '—'}</td>
      <td class="bar"><i style:width="{maxCost ? (100 * a.cost.total) / maxCost : 0}%"></i><span>{usd(a.cost.total)}</span></td>
      <td data-l="commits">{a.commits.length}</td>
      <td class="hm">{a.cardsDone || '—'}</td>
    {/snippet}
    {#snippet detail(a: InsightAgentRow)}
      <AgentDetail agent={a} days={report.days} />
    {/snippet}
  </Table>
</Section>

<style>
  .caret {
    display: inline-block;
    width: 12px;
    font-size: 10px;
  }
  .proj {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @media (max-width: 600px) {
    .proj {
      max-width: none;
      color: var(--text-2);
    }
  }
</style>
