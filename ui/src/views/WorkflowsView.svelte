<script lang="ts">
  // Workflows tab: how Coocoo is used — KPIs, the agent journey and Kanban flow funnels, session
  // shapes, frequent sequences, feature usage and friction, each linked to concrete examples.
  // Read from `insights.workflows` (the usage log, reconstructed where it has no entries yet).
  import { untrack } from 'svelte';
  import FilterBar from '../insights/FilterBar.svelte';
  import Help from '../insights/Help.svelte';
  import Kpi from '../insights/Kpi.svelte';
  import { delta, pct, span } from '../insights/format';
  import { insights } from '../insights/store.svelte';
  import AnalyzeBar from '../insights/workflows/AnalyzeBar.svelte';
  import FeaturesTable from '../insights/workflows/FeaturesTable.svelte';
  import FrictionTable from '../insights/workflows/FrictionTable.svelte';
  import Funnel from '../insights/workflows/Funnel.svelte';
  import Recorded from '../insights/workflows/Recorded.svelte';
  import SequencesTable from '../insights/workflows/SequencesTable.svelte';
  import ShapesTable from '../insights/workflows/ShapesTable.svelte';
  import { href } from '../lib/router.svelte';

  $effect(() => {
    void insights.query;
    untrack(() => insights.loadWorkflows());
  });

  const wf = $derived(insights.workflows);
  const report = $derived(wf.data);

  /** The log does not cover the whole period: part of what is shown is reconstructed. */
  const partial = $derived(!!report && (!report.logSince || report.logSince > (report.query?.from ?? insights.query.from)));
  const loggingOff = $derived(report?.hosts.filter((h) => h.ok && !h.logEnabled) ?? []);
  const reconNote = $derived(
    report && report.reconstructed > 0 ? `Includes reconstructed data (${report.reconstructed.toLocaleString()} of ${report.events.toLocaleString()} events).` : 'Recorded data only.',
  );

  /** Small ratios and medians keep a decimal (`2.5`), larger ones don't. */
  function num(v: number): string {
    return v < 10 && !Number.isInteger(v) ? v.toFixed(1) : String(Math.round(v));
  }

  function longDate(ts: number): string {
    return new Date(ts).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  }

  function share(pair: [number, number]): string {
    return pair[1] ? pct(pair[0] / pair[1]) : '—';
  }
</script>

<div class="scroll">
  <div class="page">
    <FilterBar current="workflows" sources={report?.hosts ?? []} models={[]} loading={wf.loading} onrefresh={() => insights.loadWorkflows(true)} />

    {#if !report}
      {#if wf.error}
        <div class="card failed">
          <span>Could not load the workflows: {wf.error}</span>
          <button class="btn sm" onclick={() => insights.loadWorkflows(true)}>Retry</button>
        </div>
      {:else}
        <div class="loading faint"><span class="spinner"></span> Reading the usage log…</div>
      {/if}
    {:else}
      {@const k = report.kpis}
      {#if wf.error}<div class="banner err">Refresh failed ({wf.error}); showing the previous numbers.</div>{/if}
      {#if partial || loggingOff.length}
        <div class="banner note">
          {#if partial}
            {report.logSince ? `Recorded since ${longDate(report.logSince)}` : 'Nothing recorded in this period yet'}; earlier data is reconstructed from timelines, cards
            and omp statistics ({report.reconstructed.toLocaleString()} of {report.events.toLocaleString()} events).
          {/if}
          {#if loggingOff.length}
            Recording is off on {loggingOff.map((h) => h.name).join(', ')} — switch it on in <a href={href({ name: 'settings' })}>Settings</a>.
          {/if}
        </div>
      {/if}

      <div class="body" class:dim={wf.loading}>
        <AnalyzeBar {report} />

        <div class="kpis">
          <Kpi label="Agents started" value={String(k.agentsStarted.value)} sub="{k.agentsStarted.extra ?? 0} from Kanban cards · {k.agentsByAgents} by agents">
            {#snippet help()}
              <div>Agents created in the period (<code>agent.created</code>), including deleted ones.</div>
              <table>
                <tbody>
                  <tr><td>Started by you</td><td>{k.agentsStarted.value - (k.agentsStarted.extra ?? 0) - k.agentsByAgents}</td></tr>
                  <tr><td>From Kanban cards</td><td>{k.agentsStarted.extra ?? 0}</td></tr>
                  <tr><td>Spawned by agents</td><td>{k.agentsByAgents}</td></tr>
                  {#if k.agentsStarted.prev != null}<tr><td>{insights.periodLabel}</td><td>{k.agentsStarted.prev}</td></tr>{/if}
                </tbody>
              </table>
              <div class="src">Source: usage log; before it, the agents' creation times. {reconNote}</div>
            {/snippet}
          </Kpi>
          <Kpi label="Prompts / agent" value={num(k.promptsPerAgent.value)} sub="median · p90 {num(k.promptsPerAgent.extra ?? 0)}">
            {#snippet help()}
              <div>Median number of prompts you sent per agent, over the {k.promptsPerAgent.n} agents with at least one prompt in the period; p90 = 9 in 10 agents needed at most that many.</div>
              {#if k.promptsPerAgent.prev != null}<div>{insights.periodLabel}: {num(k.promptsPerAgent.prev)}</div>{/if}
              <div class="src">Source: <code>agents.prompt</code> in the usage log; before it, omp's prompt times. {reconNote}</div>
            {/snippet}
          </Kpi>
          <Kpi label="Autonomy" value={num(k.autonomy.value)} sub="tool calls per prompt" delta={delta(k.autonomy.value, k.autonomy.prev, true)}>
            {#snippet help()}
              <div>Tool calls the agents made (including their subagents) ÷ the {k.autonomy.n} prompts you sent, summed over all agents. Higher = agents work longer on their own per message.</div>
              {#if k.autonomy.prev != null}<div>{insights.periodLabel}: {num(k.autonomy.prev)}</div>{/if}
              <div class="src">Source: tool calls from omp statistics, prompts from the usage log. {reconNote}</div>
            {/snippet}
          </Kpi>
          <Kpi label="Needs-you wait" value={span(k.needsYouWait.value)} sub="median · p90 {span(k.needsYouWait.extra)}" delta={delta(k.needsYouWait.value, k.needsYouWait.prev, false)}>
            {#snippet help()}
              <div>How long an agent waited for your answer when it asked something: median over {k.needsYouWait.n} answered questions (<code>request.answer</code>); p90 = 9 in 10 were answered within that time. Lower is better.</div>
              {#if k.needsYouWait.prev != null}<div>{insights.periodLabel}: {span(k.needsYouWait.prev)}</div>{/if}
              <div class="src">Source: usage log; before it, the <code>ask</code> tool calls in the agents' timelines. {reconNote}</div>
            {/snippet}
          </Kpi>
          <Kpi label="Abort rate" value={pct(k.abortRate.value)} sub="of turns · {k.abortRate.extra ?? 0} followed by a steer">
            {#snippet help()}
              <div>Aborted turns ÷ all turns ({k.abortRate.n} turns). “Followed by a steer”: aborts after which you steered or prompted the same agent within 2 minutes — usually a correction.</div>
              {#if k.abortRate.prev != null}<div>{insights.periodLabel}: {pct(k.abortRate.prev)}</div>{/if}
              <div class="src">Source: <code>turn.end</code> in the usage log; before it, omp's turn counts and abort times. {reconNote}</div>
            {/snippet}
          </Kpi>
          <Kpi label="Agent → commit" value={pct(k.agentCommit.value)} sub="agents with ≥1 commit">
            {#snippet help()}
              <div>Agents with at least one commit ÷ the {k.agentCommit.n} agents with a prompt in the period.</div>
              {#if k.agentCommit.prev != null}<div>{insights.periodLabel}: {pct(k.agentCommit.prev)}</div>{/if}
              <div class="src">Commits: made from the UI, recorded by Kanban “done”, or found in the agent's repository during one of its turns. {reconNote}</div>
            {/snippet}
          </Kpi>
        </div>

        <div class="grid2">
          <section class="card sec">
            <h3>
              Agent journey
              <Help label="How is the agent journey computed?">
                Agents created in the period, by the furthest stage they reached: prompted, changed files (first edit), reviewed (Changes panel opened or marked read),
                committed, archived or deleted. The columns show the drop-off from the previous stage and the median time it took. Click a stage for the agents that
                stopped there.
              </Help>
              <span class="r faint">drop-off · median time to next step</span>
            </h3>
            <Funnel stages={report.journey} color="var(--accent)" section="journey" unit="agents" />
          </section>
          <section class="card sec">
            <h3>
              Kanban flow
              <Help label="How is the Kanban flow computed?">
                Cards created in the period: accepted (entered Todo; cards created outside the inbox count as accepted at once), refined, started (Doing), done.
                “Finished in one run”: done after a single start without being moved back. Click a stage for the cards that stopped there.
              </Help>
              <span class="r faint">cards in range</span>
            </h3>
            <Funnel stages={report.kanban.stages} color="var(--green)" section="kanban" unit="cards" />
            <div class="foot faint">
              Refined cards finished in one run: <b class="good">{share(report.kanban.refinedOneRun)}</b>
              <span title="{report.kanban.refinedOneRun[0]} of {report.kanban.refinedOneRun[1]} done">({report.kanban.refinedOneRun[1]})</span>
              vs. unrefined <b>{share(report.kanban.unrefinedOneRun)}</b>
              <span title="{report.kanban.unrefinedOneRun[0]} of {report.kanban.unrefinedOneRun[1]} done">({report.kanban.unrefinedOneRun[1]})</span>
              · {report.kanban.backMoves}
              {report.kanban.backMoves === 1 ? 'card' : 'cards'} moved back from Doing
            </div>
          </section>
        </div>

        <section class="card sec">
          <h3>
            Session shapes
            <Help label="How are session shapes assigned?">
              Every agent with at least one prompt in the period gets the first shape whose rule matches (in the order shown). Commit rate = agents with a commit ÷
              agents that changed files; cost is the API equivalent.
            </Help>
            <span class="r faint">rule-based · hover a name for the rule · click for examples</span>
          </h3>
          <ShapesTable shapes={report.shapes} />
        </section>

        <section class="card sec">
          <h3>
            Frequent sequences
            <Help label="How are sequences found?">
              Steps of one agent (including its cards), or of one client within 30 minutes of activity, in order; repeated steps collapse. Runs of 3–5 steps that
              occur at least twice are counted, longer ones win over their parts. 📱 = mostly started on a phone. Ends with: most occurrences led to a commit within 30
              minutes, or contained friction (abort, failed turn, crash, API limit, cancelled dialog).
            </Help>
            <span class="r faint">within one agent or 30 min · click for examples</span>
          </h3>
          <SequencesTable sequences={report.sequences} />
        </section>

        <div class="grid2">
          <section class="card sec">
            <h3>
              Feature usage
              <Help label="How is feature usage counted?">
                Uses = times the feature was opened or used in the period; days = days with at least one use. The split shows the device: the desktop app, a phone, or
                a browser. Features never or rarely used are highlighted.
              </Help>
              <span class="r faint">desktop / phone / browser</span>
            </h3>
            <FeaturesTable features={report.features} partialSince={partial ? (report.logSince ?? 0) : undefined} />
          </section>
          <section class="card sec">
            <h3>
              Friction
              <Help label="How is friction counted?">
                Occurrences of each signal in the period and the change against the previous one (▲ = more, red). “New” = none in the previous period. Click a signal
                for its latest examples.
              </Help>
              <span class="r faint">vs. {insights.periodLabel} · click for examples</span>
            </h3>
            <FrictionTable friction={report.friction} />
          </section>
        </div>

        <Recorded />
      </div>
    {/if}
  </div>
</div>

<style>
  .scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .page {
    max-width: 1280px;
    margin: 0 auto;
    padding: 20px 24px 48px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .body.dim {
    opacity: 0.55;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 40px 0;
    justify-content: center;
    font-size: var(--fs-sm);
  }
  .failed {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px;
    color: var(--del-text);
    font-size: var(--fs-sm);
  }
  .banner {
    padding: 8px 12px;
    border-radius: var(--r);
    font-size: var(--fs-sm);
    margin-top: -6px;
  }
  .banner.err {
    background: var(--red-soft);
    color: var(--del-text);
  }
  .banner.note {
    background: var(--blue-soft);
    color: var(--text-2);
  }
  .banner a {
    color: var(--accent-text);
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 10px;
  }
  .kpis :global(.src) {
    margin-top: 6px;
    color: var(--text-3);
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
  }
  .sec {
    padding: 14px 16px;
    min-width: 0;
  }
  h3 {
    margin: 0 0 10px;
    font-size: var(--fs);
    font-weight: 600;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
  }
  h3 .r {
    margin-left: auto;
    font-weight: 400;
    font-size: var(--fs-sm);
  }
  .foot {
    margin-top: 8px;
    font-size: var(--fs-xs);
  }
  .foot b {
    color: var(--text);
  }
  .foot b.good {
    color: var(--add-text);
  }
  @media (max-width: 1100px) {
    .kpis {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @media (max-width: 900px) {
    .grid2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (max-width: 600px) {
    .page {
      padding: 14px 12px 40px;
      gap: 14px;
    }
    .body {
      gap: 14px;
    }
    .kpis {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 8px;
    }
    .sec {
      padding: 12px;
    }
    h3 .r {
      margin-left: 0;
      flex-basis: 100%;
      font-size: var(--fs-xs);
    }
  }
</style>
