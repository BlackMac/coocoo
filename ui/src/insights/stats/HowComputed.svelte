<script lang="ts">
  // "How these numbers are computed": scope, formulas and sources of every number on the Stats tab.
  import Tag from './Tag.svelte';
</script>

<details class="card how" open>
  <summary>How these numbers are computed</summary>
  <ul>
    <li>
      <b>Scope:</b> only omp sessions of Coocoo agents — including their subagents and advisor calls — on every online host. Deleted agents
      stay counted, marked <Tag kind="deleted" />.
    </li>
    <li>
      <b>Cost:</b> the sum of omp's per-response <code>cost_total</code> at API list prices, i.e. the <b>API equivalent</b>. On
      subscriptions (Claude Max, Codex, …) this is not what you were billed.
    </li>
    <li><b>Tokens:</b> input + output + cache read + cache write. The Cost “?” splits the cost by these four parts.</li>
    <li><b>Requests:</b> assistant responses, one per model call (agents, subagents, advisor). <b>Prompts:</b> messages you sent.</li>
    <li>
      <b>Commits:</b> <Tag kind="exact" /> = recorded by the Kanban board when the agent finished its card; <Tag kind="turn" /> = a commit
      in the agent's repository or worktree whose commit time falls inside one of its turns (±60 s); <Tag kind="shared" /> = inside turns of
      several agents of the same repository, counted for each. Lines are + / − from <code>git --shortstat</code>.
    </li>
    <li>
      <b>Active time:</b> from each prompt to the last response of its turn; gaps of more than 10 minutes are not counted.
    </li>
    <li><b>Days:</b> in your time zone. Active days have at least one prompt or request; the streak is the longest run of them.</li>
    <li>
      <b>Source:</b> <code>~/.omp/stats.db</code> on each host, synced via <code>omp stats</code> before reading (at most once a minute).
      Kanban numbers come from the project boards.
    </li>
    <li><b>Flagged prompts:</b> omp's frustration signals (CAPS, swearing, anguish, negation, repetition, blame) on your prompts.</li>
  </ul>
</details>

<style>
  .how {
    padding: 12px 16px;
  }
  summary {
    cursor: pointer;
    color: var(--text-2);
  }
  ul {
    margin: 8px 0 0;
    padding-left: 18px;
    color: var(--text-2);
    font-size: var(--fs-sm);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  b {
    color: var(--text);
    font-weight: 600;
  }
  code {
    font-size: var(--fs-xs);
    color: var(--text);
  }
</style>
