<script lang="ts">
  // "What is recorded": what the usage log behind this tab contains, where it lives and who sees it.
  import { href } from '../../lib/router.svelte';
</script>

<details class="card how">
  <summary>What is recorded</summary>
  <ul>
    <li>
      <b>Actions and UI navigation only</b> — which action happened, when, by whom (you, an agent or the daemon), on which kind of device, for which agent, project
      or card, whether it worked and how long it took. <b>Never</b> prompt texts, arguments, file contents or paths; only ids, counts, lengths and names of commands,
      panels, dialogs and models.
    </li>
    <li>
      Event families: <b>agents</b> (create, prompt, steer, abort, answer, model and thinking changes, rename, hibernate, archive, delete), <b>agent runs</b> (turns
      with their stop reason and tool count, needs-you questions asked / answered / dismissed, retries, compactions, crashes, refused commands), <b>Kanban</b>
      (cards added, moved, refined, started, done, deleted, board sources), <b>git</b> (commit, merge, push, fetch, pull requests, worktrees), <b>browser and
      terminal</b>, <b>project actions</b>, <b>projects and hosts</b> added or removed, and <b>UI events</b> (views, tabs, panels, command palette, shortcuts, slash
      command names, dialogs opened / submitted / cancelled, model picker, leaving and returning to the app, followed workflow examples).
    </li>
    <li>
      Stored per host in <code>&lt;data dir&gt;/events/</code> as monthly JSONL files, kept 365 days by default. Recording can be switched off and the log
      deleted in <a href={href({ name: 'settings' })}>Settings</a>.
    </li>
    <li>
      Before a host's log starts, sequences and counts are <b>reconstructed</b> from agent timelines, Kanban cards and omp's statistics. The JSONL download
      contains recorded events only.
    </li>
    <li>
      <b>Analyze patterns</b> hands the aggregated data of this page to the analysis agent, so that agent's model provider sees it. Otherwise nothing leaves
      your hosts.
    </li>
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
    gap: 4px;
  }
  b {
    color: var(--text);
    font-weight: 500;
  }
  code {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text);
  }
  a {
    color: var(--accent-text);
  }
</style>
