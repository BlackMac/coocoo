<script lang="ts">
  // Projects: agents, cost, commits, lines and finished cards per project; a row sets the project filter.
  import type { InsightProjectRow, UsageReport } from '../../lib/protocol';
  import { count, usd } from '../format';
  import { insights } from '../store.svelte';
  import Section from './Section.svelte';
  import Table from './Table.svelte';
  import { hostName } from './util';

  let { report }: { report: UsageReport } = $props();

  const maxCost = $derived(Math.max(0, ...report.projects.map((p) => p.cost)));
  const manyHosts = $derived(new Set(report.projects.map((p) => p.hostId)).size > 1);

  const cols = [
    { label: 'Project' },
    { label: 'Agents', title: 'Agents with at least one request' },
    { label: 'Cost', title: 'API equivalent at list prices' },
    { label: 'Commits', title: 'Commits attributed to the agents' },
    { label: 'Lines', title: '+ / − from git --shortstat' },
    { label: 'Cards done', title: 'Cards that entered Done' },
  ];

  const isActive = (p: InsightProjectRow) => insights.project?.hostId === p.hostId && insights.project.id === p.projectId;

  function pick(p: InsightProjectRow) {
    insights.project = isActive(p) ? null : { hostId: p.hostId, id: p.projectId, name: p.name };
  }
</script>

<Section title="Projects" note="click a row to filter">
  {#snippet help()}
    Per project: <b>Agents</b> with at least one request, <b>Cost</b> (API equivalent) of their requests incl. subagents and advisor,
    <b>Commits</b> attributed to them (exact via Kanban, or during one of their turns), <b>Lines</b> + / − from
    <code>git --shortstat</code>, and Kanban <b>cards</b> that entered Done.
  {/snippet}
  <Table {cols} rows={report.projects} key={(p) => `${p.hostId}:${p.projectId}`} onrow={pick} active={isActive} empty="No project activity in this period.">
    {#snippet row(p: InsightProjectRow)}
      <td class="t">{p.name}{#if manyHosts}<span class="faint"> · {hostName(report, p.hostId)}</span>{/if}</td>
      <td data-l="agents">{p.agents}</td>
      <td class="bar"><i style:width="{maxCost ? (100 * p.cost) / maxCost : 0}%"></i><span>{usd(p.cost)}</span></td>
      <td data-l="commits">{p.commits}</td>
      <td data-l="lines"><span class="add">+{count(p.linesAdded)}</span> <span class="del">−{count(p.linesDeleted)}</span></td>
      <td data-l="cards done">{p.cardsDone || '—'}</td>
    {/snippet}
  </Table>
</Section>
