<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary, Card } from '../lib/protocol';
  import { Bot, ChevronDown, GitBranch, NotebookPen, Play, Plus } from '../lib/icons';
  import Menu, { type MenuItem } from '../components/Menu.svelte';

  // Hands a card to an agent. The button starts it in the agent the board is shown for (or a
  // new agent on the project page); the menu offers the project's other agents and new ones.
  // `refine`: the agent refines the card (researches, asks, attaches a plan) instead of doing it.
  let {
    host,
    projectId,
    card,
    agent,
    primary = false,
    refine = false,
  }: { host: string; projectId: string; card: Card; agent?: AgentSummary; primary?: boolean; refine?: boolean } = $props();

  const project = $derived(fleet.project(host, projectId));
  const others = $derived(fleet.agentsOf(host, projectId).filter((x) => x.status !== 'stopped' && x.id !== agent?.id));
  let busy = $state(false);

  async function start(params: { agentId?: string; worktree?: boolean }) {
    if (busy) return;
    busy = true;
    try {
      await fleet.call(refine ? 'boards.cardRefine' : 'boards.cardStart', { projectId, cardId: card.id, ...params }, host, 120_000);
    } catch (err) {
      ui.error(refine ? 'Could not request a refinement' : 'Could not start the card', err);
    } finally {
      busy = false;
    }
  }

  /** A card is a new task: never queue it behind a running answer. */
  const isBusy = (x: AgentSummary) => x.status === 'starting' || x.status === 'running' || x.status === 'waiting';
  const inAgent = (x: AgentSummary): MenuItem => ({
    label: isBusy(x) ? `In ${x.title} (busy)` : `In ${x.title}`,
    icon: Bot,
    disabled: isBusy(x),
    run: () => start({ agentId: x.id }),
  });
  const items = $derived<MenuItem[]>([
    ...(agent && agent.status !== 'stopped' ? [inAgent(agent)] : []),
    ...others.map(inAgent),
    { label: 'New agent', icon: Plus, run: () => start({}) },
    ...(project?.git && !refine ? [{ label: 'New agent in a worktree', icon: GitBranch, run: () => start({ worktree: true }) }] : []),
  ]);
  const here = $derived(agent && agent.status !== 'stopped' ? agent : undefined);
  const hereBusy = $derived(!!here && isBusy(here));
</script>

<div class="split">
  <button
    class="btn sm go"
    class:primary
    disabled={busy || hereBusy}
    title={hereBusy
      ? `${here!.title} is still answering`
      : refine
        ? `Have ${here ? here.title : 'a new agent'} refine it first: research, questions, a plan and mocks`
        : here
          ? `Hand it to ${here.title}`
          : 'Start a new agent for it'}
    onclick={() => start(here ? { agentId: here.id } : {})}
  >
    {#if busy}<span class="spinner"></span>{:else if refine}<NotebookPen size={12} />{:else}<Play size={11} fill="currentColor" />{/if}{refine
      ? 'Refine'
      : here
        ? 'Start'
        : 'Start new'}
  </button>
  <Menu label={refine ? 'Refine in…' : 'Start in…'} {items}>{#snippet trigger()}<ChevronDown size={13} />{/snippet}</Menu>
</div>

<style>
  .split {
    display: flex;
    align-items: center;
    flex: none;
  }
  .go {
    gap: 4px;
  }
  .go .spinner {
    width: 10px;
    height: 10px;
  }
</style>
