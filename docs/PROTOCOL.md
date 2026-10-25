# Coocoo wire protocol (v1)

One WebSocket per client at `ws://<host>:<port>/ws?token=<token>`. Text frames, one JSON object each.
A bad token is refused with HTTP 401 before the upgrade. `&scope=local` starts the connection in
`system.subscribe {scope:"local"}` mode (used by peer links, so no relayed event can loop back).
Envelope types live in `crates/coocoo-proto/src/wire.rs`; payload types in `types.rs`; the UI
mirrors both in `ui/src/lib/protocol.ts`. All JSON is camelCase.

```text
client → daemon   {"id": 7, "method": "agents.create", "params": {...}, "host": "<hostId>"?}
daemon → client   {"id": 7, "result": ...}
                  {"id": 7, "error": {"code": "not_found", "message": "agent abc not found"}}
daemon → client   {"event": "agent.updated", "host": "<origin hostId>", "data": {...}}
```

- `host` omitted (or equal to the home daemon id) → handled by the daemon the client is connected to.
  Any other known host id → forwarded over that peer link (one hop; peers never forward further).
  Unknown host → `not_found`; known but offline → `unavailable`.
- Every event carries its origin `host`; the home daemon relays peer events unchanged, so a client
  connected to one daemon sees the whole fleet. `host.updated`/`host.removed` describe the home
  daemon's view of its fleet and are never relayed further. A `system.resync` with a peer's `host`
  means the link to that peer lagged: refetch that host's state.
- `browser.watch`/`browser.unwatch`/`stats.watch` addressed to a peer are reference-counted by the
  home daemon: several clients share one remote stream, which stops when the last one leaves or
  disconnects, and is re-established after a reconnect. `terminal.attach`/`terminal.detach` are
  counted the same way per terminal; after a reconnect clients re-attach themselves (output sent
  while the link was down is only in the scrollback).
- Error codes: `invalid_params`, `not_found`, `unknown_method`, `conflict`, `unauthorized`,
  `unavailable`, `forbidden`, `internal`.
- HTTP (same port): `GET /api/health` (no auth, CORS `*`) → `{ok, hostId, name, version}`; static
  UI at `/` (web/phone access, SPA fallback to `index.html`).
- CDP discovery: each browser profile gets its own loopback listener (`browser.cdpEndpoint`), whose
  root serves `GET /json/version`, `/json/list`, `/json`, `/json/protocol` (loopback `Host` header
  only, else 403; no token) → Chrome's discovery JSON unchanged, launching that profile's Chromium
  on demand; errors are `{code, message}` with 404/503/500. A dedicated root is required because
  puppeteer resolves discovery as `new URL('/json/version', cdpUrl)`, dropping any path.

## Methods

Params marked `?` are optional. Return types refer to `types.rs`.

### system / hosts / stats — `coocoo-core`
| Method | Params | Result |
|---|---|---|
| `system.hello` | `{ client?: { id, device: "desktop" \| "phone" \| "browser" } }` | `{ protocol: u32, host: HostInfo, hosts: HostInfo[] }` (`hosts` = local + peers). `client` names the client installation (random id) and device kind for the usage log's actor of this connection's requests |
| `system.subscribe` | `{ scope: "all" \| "local" }` | `{}` — peer links use `local` (only events originating on this daemon) |
| `hosts.list` | – | `HostInfo[]` |
| `hosts.add` | `{ kind: "direct", url, token?, name? }` or `{ kind: "ssh", target: "user@host", sshPort?, remotePort? (7421), token?, install?: bool, name? }` | `HostInfo` — connects first and fails (`unavailable`, with the reason) if the host cannot be reached; `conflict` if already in the fleet. `url`: `ws://host:port`, `host[:port]`, or a pairing link `http://host:port/#token=…` (then `token` may be omitted). SSH without `token` runs `coocoo token` remotely; `install` copies this `coocoo` binary to `~/.local/bin` when missing (same OS/arch) and starts it (lingering systemd user unit, else `nohup`) |
| `hosts.remove` | `{ id }` | `{}` |
| `hosts.rename` | `{ id, name }` | `HostInfo` (`id` = local renames this daemon) |
| `stats.watch` | `{ enabled: bool }` | `{}` — while ≥1 connection watches, `host.stats` every 2 s |
| `stats.get` | – | `HostStats` |

### projects / git / fs — `coocoo-workspace`
| Method | Params | Result |
|---|---|---|
| `projects.list` | – | `Project[]` |
| `projects.get` | `{ id }` | `Project` |
| `projects.add` | `{ path, name?, create?: bool, cloneUrl? }` | `Project` (directory must exist; git optional). `create`: make `path` as a new directory first (`conflict` if it exists). `cloneUrl`: `git clone` it into `path` first (`path` must not exist or be empty; up to 600 s). Both create missing parents |
| `projects.update` | `{ id, name?, devUrls? }` | `Project` |
| `projects.remove` | `{ id }` | `{}` (never touches files) |
| `projects.refresh` | `{ id }` | `Project` (recomputes `git`, `icon` and `actions`) |
| `projects.icon` | `{ id }` | `{ mime, data: base64 }` — the file named by `Project.icon` (`not_found` without one). `icon` is picked from the project's favicon/app icon/logo at the usual places (root, `public/`, `static/`, `app/`, `src/app/`, `assets/`, `src/assets/`, `src/`, `web/`, `src-tauri/icons/`, `resources/`, `build/`, `.github/`, `docs/`): icons before logos, SVG before PNG before ICO, ≤ 512 KiB; re-detected on every git refresh |

`Project.git` (`GitInfo`) describes the main checkout (`branch`, `head`, `upstream`, `ahead`/`behind`
against it, `changed`, `lastCommit`) plus `fetchedAt` (newest `FETCH_HEAD` mtime over all
worktrees, Unix ms; absent when never fetched: the upstream counts are as old as that) and
`worktrees: Worktree[]`. Each `Worktree` has `path`, `branch` (absent on a detached HEAD), short
`head`, `isMain`, `changed`, `upstream`/`ahead`/`behind` of its branch, `upstreamGone` (the upstream
is configured but its remote branch was deleted and pruned by a fetch; `ahead`/`behind` are 0 then),
`conflicts` (unmerged paths), `operation` (`merge` \| `rebase` \| `cherryPick` \| `revert` \|
`bisect` in progress) and, for linked worktrees on a branch, `baseAhead`/`baseBehind` against the
main worktree's branch (the target of `git.merge`) plus `changesInBase`: the branch has commits
ahead, but all their changes are in the base already (same tree, or `git merge-tree --write-tree`
of base and HEAD yields the base's tree, git ≥ 2.38), as after a squash or rebase merge. It is
recomputed when a watched git file changes (HEAD, index, refs, `FETCH_HEAD` of every worktree) and
after each agent turn; Coocoo itself never fetches.

`GitInfo.remote` is the default remote for pushing a branch without upstream (`origin`, else the
only remote; absent when there is none or it is ambiguous). When its URL points to github.com,
`GitInfo.github` is `{ repo: "owner/name", remote, gh: "unknown" \| "ready" \| "missing" \| "loggedOut", mergeMethods: ("squash" \| "merge" \| "rebase")[], deleteBranchOnMerge }`.
Pull requests go through the `gh` CLI (`GH_PROMPT_DISABLED=1`, always with `--repo`); `gh`'s state
is checked on the first pull request call of the daemon and again after a failed `gh` call, the
repository settings (`gh repo view`) once per repository. A linked worktree's branch carries its
pull request as `Worktree.pullRequest` (`PullRequestInfo`: `number`, `url`, `title`, `state`
open/closed/merged — an open one before older ones, PRs from forks ignored —, `draft`, `base`, `headSha` (the PR's head commit),
`checks: { total, passed, failed, pending, failing: [{ name, url? }] }`, `review?` approved /
changesRequested / reviewRequired, `mergeState` = GitHub's `mergeStateStatus`, `fetchedAt`) and
`Worktree.prCheckedAt` (last lookup, also when there is none). The daemon caches it and looks it up
only after `git.push`, `git.prCreate`, `git.prMerge`, `git.fetch` (all linked worktrees with an
upstream, ≤ 8 in parallel) and `git.prStatus` — never on watcher or turn refreshes. The UI calls
`git.prStatus` when the git popover opens and the status is older than 60 s.

| Method | Params | Result |
|---|---|---|
| `git.diff` | `{ cwd, base? }` | `DiffResult` — working tree (incl. untracked) vs `HEAD`, or vs `merge-base(base, HEAD)` when `base` is given |
| `git.diffStat` | `{ cwd, base? }` | `DiffStat` — same comparison as `git.diff`, counts only |
| `git.log` | `{ cwd, limit? (20) }` | `CommitInfo[]` |
| `git.commit` | `{ cwd, message }` | `CommitInfo` — stages everything (`git add -A`) then commits |
| `git.merge` | `{ projectId, branch }` | `{ ok: bool, output: String }` — merges `branch` into the project's checked-out branch (no fast-forward commit message prompt; aborts and reports on conflict) |
| `git.fetch` | `{ projectId }` | `{ ok: bool, output: String }` — `git fetch --all --prune --no-auto-gc` in the project's checkout, at most 60 s; never prompts (ssh runs with `BatchMode=yes` unless `GIT_SSH`/`GIT_SSH_COMMAND`/`core.sshCommand` is set), so missing credentials fail with git's message |
| `git.fastForward` | `{ projectId, path }` | `{ ok: bool, output: String }` — `git merge --ff-only @{upstream}` in the project's worktree at `path`, using the last fetch; `conflict` when the branch has no upstream (or it is gone), has commits of its own, or the tree has uncommitted files |
| `git.resetToBase` | `{ projectId, path }` | `{ ok: bool, output: String }` — `git reset --keep <base>` in the linked worktree at `path` (base = the main worktree's branch), then `git branch --unset-upstream` when the upstream is gone. `conflict` on a detached HEAD, with uncommitted files, conflicts or an operation in progress, or when the branch has changes not in the base (`baseAhead` > 0 without `changesInBase`) |
| `git.push` | `{ projectId, path }` | `{ ok: bool, output: String, rejected: bool }` — pushes the branch checked out in the worktree containing `path`: to its upstream, or without one with `-u` to `<remote>/<branch>` (`GitInfo.remote`). Never forces; `rejected` = refused as non-fast-forward. Same environment and 60 s limit as `git.fetch`. `conflict` on a detached HEAD or without a remote |
| `git.outgoing` | `{ projectId, path }` | `CommitInfo[]` (newest first, ≤ 200) — what `git.push` would send: `@{upstream}..HEAD`, else the commits no branch of the remote has |
| `git.prCreate` | `{ projectId, path, title, body?, draft?: bool, base? }` | `PullRequestInfo` — pushes first when the branch has no upstream (or it is gone) or unpushed commits, then `gh pr create --base <base> --head <branch>` (body via stdin); `base` defaults to the main worktree's branch. `unavailable` without a usable `gh`, `conflict` when the remote is not on GitHub, the push fails or `gh` refuses (e.g. a PR exists already) |
| `git.prStatus` | `{ projectId, path }` | `PullRequestInfo \| null` — looks the branch's pull request up now (`gh pr list --head <branch> --state all`) and emits `project.updated` with it |
| `git.prMerge` | `{ projectId, path, method: "squash" \| "merge" \| "rebase", deleteBranch?: bool }` | `{ ok: bool, output: String }` — `gh pr merge <n> --<method>` for the branch's open pull request (never `--admin`, `--auto` or `-d`, which would switch branches in the worktree); then optionally `git push <remote> --delete <branch>`, then `git.fetch`. A merged PR makes the worktree removable in the UI even when a squash merge left it "ahead" |
| `git.worktreeCreate` | `{ projectId, branch?, base? }` | `WorktreeRef` — new branch (default `coocoo/<id>`) from `base` (default current branch) under `<data_dir>/worktrees/<project-name>-<project-id>/<branch-slug>` |
| `git.worktreeRemove` | `{ projectId, path, deleteBranch?: bool }` | `{}` |
| `fs.list` | `{ path? }` (default home) | `{ path, parent?, entries: FsEntry[] }` — directories first, hidden entries omitted |

### terminal — `coocoo-workspace` (Linux/macOS hosts)
A terminal is a login shell (`$SHELL -l`, else `/bin/sh`, with `TERM=xterm-256color`) on a pty, or
the run of a project action (`actions.run`: `$SHELL -lc <command>`). A shell keeps running while
nobody is attached and ends when it exits or on `terminal.kill`. An action run carries
`TerminalInfo.action` (`{ projectId, actionId, name, base }`) and stays listed after its process
ended, with `exitedAt` and `exitCode` (absent after a signal) and its scrollback, until it is
restarted or closed. Opening and ending are broadcast (`terminal.updated` / `terminal.removed`).

| Method | Params | Result |
|---|---|---|
| `terminal.list` | – | `TerminalInfo[]` (oldest first) |
| `terminal.open` | `{ cwd, cols? (80), rows? (24) }` | `TerminalInfo` — `cwd` must be a directory; does not attach |
| `terminal.attach` | `{ id, cols?, rows? }` | `{ data: String }` — the recent output (≤ 256 KiB); this connection then receives `terminal.output`/`terminal.exit` until `terminal.detach` or disconnect. Output may arrive before this reply: apply it after `data` |
| `terminal.detach` | `{ id }` | `{}` — the process keeps running |
| `terminal.input` | `{ id, data }` | `{}` — keystrokes/paste as UTF-8 text (≤ 256 KiB); `conflict` once the process ended |
| `terminal.resize` | `{ id, cols, rows }` | `{}` — last resize wins when several clients are attached |
| `terminal.kill` | `{ id }` | `{}` — SIGHUP to the process group (no-op once it ended) |
| `terminal.close` | `{ id }` | `{}` — removes an ended action run; a running one is hung up and removed when it ends |

### actions — `coocoo-workspace`
A project declares actions in `.coocoo/actions.toml` in its directory (the main checkout, also for
agents in worktrees; format in the README). The daemon parses the file when the project is added or
refreshed and when the file changes (watched non-recursively: the project directory filtered to
`.coocoo`, and `.coocoo/` filtered to `actions.toml`, ~0.4 s debounce), and publishes the result as
`Project.actions: ProjectAction[]` (file order) and `Project.actionErrors: String[]`
(`actions.toml:LINE:COL – …`; invalid entries are skipped, a syntax error leaves no actions). Both
are runtime state, not stored in `projects.json`. A `ProjectAction` has `id`, `name`, `icon?`,
`scope` (`project` \| `agent` \| `both`), `default?`, `confirm?` and `kind`: `run` (`command`,
`cwd?`), `prompt` (`prompt`, `worktree?`) or `url` (`url`, `target?`: `browser` \| `external`).
Clients carry out `prompt` and `url` actions themselves (`agents.prompt`/`agents.create`,
`browser.open`); only `run` goes to the daemon.

| Method | Params | Result |
|---|---|---|
| `actions.run` | `{ projectId, actionId, cwd, cols?, rows? }` | `TerminalInfo` — starts the `run` action's command in a new terminal in `cwd` (the project directory, or an agent's directory inside it or one of its worktrees; else `invalid_params`) joined with the action's `cwd` (must exist there and stay inside). Env: `COOCOO_PROJECT_DIR` (main checkout), `COOCOO_WORKDIR` (`cwd`), `COOCOO_ACTION` (id). An earlier run of the same action for the same `cwd` is stopped first (SIGHUP, SIGKILL after 5 s) and removed; other directories run alongside. `TerminalInfo.action.base` is `cwd` as given |

### agents — `coocoo-agents` (omp only)
| Method | Params | Result |
|---|---|---|
| `agents.list` | `{ projectId? }` | `AgentSummary[]` |
| `agents.get` | `{ id }` | `AgentDetail` |
| `agents.create` | `{ projectId?, cwd?, title?, prompt?, images?: ImageInput[], worktree?: bool \| { branch?, base? }, model?, thinking?, parentId? }` | `AgentSummary` — one of `projectId`/`cwd` required; `images` go with the first prompt (alone when there is no `prompt`); `unavailable` while the daemon drains for a restart. The worktree's branch defaults to `coocoo/<title-slug>-<first 4 chars of the agent id>` from the title (given, else the prompt's first line; card agents: the card title), e.g. `coocoo/git-status-push-pr-1a2b`: lowercase ASCII words (ä/ö/ü/ß transliterated), at most 40 chars; `coocoo/<agent id>` without a usable title |
| `agents.prompt` | `{ id, message, mode?: "auto" \| "steer" \| "followUp", images?: ImageInput[] }` | `{}` — `auto`: prompt when idle, follow-up when running; wakes hibernated agents; `unavailable` while the daemon drains for a restart (running agents are asked to pause, see ARCHITECTURE.md) |
| `agents.abort` | `{ id }` | `{}` |
| `agents.respond` | `{ id, requestId, response }` | `{}` — see `UiRequest` docs for `response` shapes; for a `secret` request only `{ cancelled: true }` (decline) is accepted, the value goes through `secrets.provide` |
| `agents.timeline` | `{ id, before?: seq, limit? (200) }` | `TimelinePage` — a `user` item's `images` (`TimelineImage[]`: `name`, `mime`, `size`, `width?`, `height?`) are the message's images as the agent got them (omp scales them to at most 1568 px and usually re-encodes them as WebP); fetch each with `agents.image`. Items from before images were stored end in `[N image(s)]` instead |
| `agents.output` | `{ id }` | `{ status: AgentStatus, text: String? }` — full last assistant text |
| `agents.markRead` | `{ id }` | `{}` |
| `agents.rename` | `{ id, title }` | `AgentSummary` |
| `agents.setModel` | `{ id, model: "provider/id" }` | `AgentSummary` |
| `agents.setThinking` | `{ id, level }` | `AgentSummary` |
| `agents.models` | – | `ModelInfo[]` (cached per host) |
| `agents.commands` | `{ id }` | `SlashCommand[]` — the agent's slash-command catalog from omp (`name`, `source` builtin/skill/extension/custom/mcp_prompt/file, `aliases`, `description`, `hint`, `subcommands`), as reported by its process; an agent that never ran one gets it from a throwaway omp in its directory. Builtins that make no sense through Coocoo are left out (`/move`, `/wt`, `/pin`, `/trace`, `/stats`, `/browser`, `/export`, `/dump`, `/share`) along with the subcommands `/session delete`, `/todo edit|copy|expand|collapse` and `/advisor configure|dump`; a list-style hint is rebuilt from the remaining subcommands. Sending `/name args` with `agents.prompt` runs the command (one of the left-out builtins is not sent to omp: the agent stays as it is and the timeline gets the `user` item plus an `error` item saying why; `/rename` also sets the agent's title): omp-local ones answer with a `command` timeline item (plain text) after an echoed `user` item; skills and custom commands start a turn |
| `agents.usage` | `{ provider, model? }` | `ProviderUsage` — the provider's subscription quota on this host from `omp usage --json --provider <provider>`: `windows` (one per window and quota pool, summed over all logged-in accounts: `used` in accounts' worth, earliest `resetsAt`; `name` labels a pool only some models draw from, such as an Anthropic tier, Codex Spark or an Antigravity counter like `Gemini`; `gating` marks the windows `model`, the model id within the provider, draws from) and per-account `limits` (`unit` percent/usd/tokens/…, `usedFraction`, `status`, `resetsAt`; a quota omp reports once per routing family is listed once). Empty for pay-per-token providers. Cached 60 s per provider; `unavailable` if omp fails or takes over 20 s |
| `agents.subagentTimeline` | `{ id, sessionFile, live? }` | `TimelinePage` — read-only transcript of one of the agent's subagents, read from its omp session file (`Subagent.sessionFile`; only files below the agent's own session directory, else `forbidden`); newest 500 items. `live`: the subagent still runs, so unanswered tool calls show as running instead of interrupted |
| `agents.image` | `{ id, name }` | `{ mime, data }` (base64) — one image of a user message (`TimelineImage.name`, `<seq>-<index>.<png\|jpg\|gif\|webp>`; any other name is `invalid_params`), stored in the agent's data directory and deleted with the agent |
| `agents.hibernate` | `{ id }` | `AgentSummary` |
| `agents.archive` | `{ id }` | `AgentSummary` (status `stopped`, process gone, read-only) |
| `agents.delete` | `{ id, removeWorktree?: bool, deleteBranch?: bool }` | `{}` — `deleteBranch` (with `removeWorktree`) also deletes the worktree's branch |

### boards — `coocoo-agents` (one Kanban board per project)
Stored in `<data_dir>/boards/<project id>.json`; boards that earlier versions kept per agent move to
the agent's project on startup (cards in doing/done keep that agent). Every method takes
`projectId` (`not_found` for an unknown project).

| Method | Params | Result |
|---|---|---|
| `boards.get` | `{ projectId }` | `Board` — `cards` (`column` inbox/todo/doing/done; a column's order is the order of its cards, first = top; `agentId`/`agentTitle` = the agent it was handed to or that finished it; `refining` while its agent works out a refinement; `refinement`: `{ text (Markdown), attachments?: [{ name, mime, size }], agentId?, agentTitle?, at }`; `pullRequest?: { number, url }` and `gitError?` from finishing it; `doneAt?`: when it last entered `done`, cleared when it moves back — cards finished before Coocoo recorded it have none), `sources` and `doneAction` (`commit` \| `push` \| `pullRequest`, default `commit`: what `boards.cardDone` does after the commit) |
| `boards.update` | `{ projectId, doneAction? }` | `Board` |
| `boards.cardAdd` | `{ projectId, title, body?, column? (todo), top?: bool, startUrl?, doneUrl?, images?: [{ name?, mimeType, data (base64) }], agentId? }` | `Card` — `agentId`: the agent adding it, recorded as `addedBy: { id, title }` (`kanban_add`; its inbox suggestions go to the top); `startUrl`/`doneUrl`: http(s) webhooks, see below. `images`: context images (PNG, JPEG, GIF or WebP, ≤ 5 MiB each, ≤ 10 per card), stored in `<data_dir>/boards/cards/<card id>/` under safe, unique names (`Card.images`: `[{ name, mime, size }]`); the agent that starts or refines the card gets their paths in its prompt, `kanban_list` shows them |
| `boards.cardUpdate` | `{ projectId, cardId, title?, body?, startUrl?, doneUrl? ("" clears each), column?, index?, addImages?: [{ name?, mimeType, data }], removeImages?: [name] }` | `Card` — images as for `cardAdd` (the UI uploads one per call to stay below the socket's message limit); `index` = position in the (target) column, 0 = top, past the end = bottom |
| `boards.cardDelete` | `{ projectId, cardId }` | `{}` (its images and refinement attachments are deleted) |
| `boards.cardStart` | `{ projectId, cardId, agentId?, worktree?: bool, model? }` | `Card` — prompts `agentId` (one of the project's agents, not archived) with the card (title, body, link, how to finish it) like `agents.prompt` (`auto`), or without `agentId` creates a new agent for it (title = card title, optionally in its own worktree); then records the agent on the card and moves it to the top of `doing`. `conflict` if it is already in doing/done, or if that agent is busy (`starting`, `running` or `waiting`): a card never queues behind a running answer. A refined card's prompt carries its refinement and the attachments' paths |
| `boards.cardRefine` | `{ projectId, cardId, agentId?, worktree?: bool, model? }` | `Card` — like `boards.cardStart`, but the prompt asks for a refinement instead of the work: research what the agent can find out itself, ask the user (`ask`) only the rest, optionally make UI mocks, then call `kanban_refine`. The card stays in `todo` (an inbox card moves to the bottom of `todo`) with the agent recorded and `refining: true`; an earlier refinement is included for revision. `refining` also clears when the agent's run ends without `kanban_refine` (idle, error, crash; an open `ask` keeps it): after an error the card's `note` becomes `Refinement aborted: <first line of the error>`. Starting or refining a card clears its `note` |
| `boards.refinementSave` | `{ projectId, cardId, agentId?, text (Markdown, ≤ 32 KiB), attachments?: [path] }` | `Card` — what `kanban_refine` calls: replaces the card's refinement and clears `refining`. Attachments (≤ 10, ≤ 5 MiB each; png, jpg, gif, webp, svg, html) are copied to `<data_dir>/boards/attachments/<card id>/`; relative paths resolve in the agent's directory |
| `boards.attachment` | `{ projectId, cardId, name }` | `{ mime, data }` (base64) — a file of the card's refinement. The UI shows HTML mocks only in `sandbox="allow-scripts"` frames (no app origin) |
| `boards.cardImage` | `{ projectId, cardId, name }` | `{ mime, data }` (base64) — one of the card's images |
| `boards.cardDone` | `{ projectId, cardId, agentId? (the card's agent), commitMessage? (card title), note?, prTitle?, prBody? }` | `{ card, commit?: CommitInfo, commitSkipped?: "not a git repository" \| "nothing to commit", pushed?, pullRequest?: { number, url }, gitError? }` — `git.commit` in that agent's directory (the project's without one), records the agent, then moves the card to the top of `done`; a failing commit leaves the card in place and returns the error. With `doneAction` `push` it then runs `git.push` there; with `pullRequest` also `git.prCreate` for agents in their own worktree when the branch has no open pull request (title `prTitle`, else the card title; body `prBody`, else the card reference, the refinement's Goal/Ziel section or the card text, and the pushed commits), agents in the project checkout only push. A failing push or PR leaves the card done with `card.gitError`; on success `card.pullRequest` is set. An agent in the project checkout that pushed more than one commit gets "Pushed N commits." added to the note |
| `boards.cardPublish` | `{ projectId, cardId }` | `{ card, pushed, pullRequest?, error? }` — repeats the push (and pull request, per `doneAction`) for a done card, e.g. after `gitError`; also pushes when the board is set to `commit` |
| `boards.sourceAdd` | `{ projectId, kind: "feed" \| "command" \| "script", target? (feed URL / command), script? , name? (required for scripts), intervalMins? (15 \| 60 \| 360 \| 1440, default 60), agentId? }` | `{ source: BoardSource, imported }` — an importer for the inbox, run now and then on its schedule while the daemon runs (see below); `source.error` says why a run failed |
| `boards.sourceUpdate` | `{ projectId, sourceId, name? ("" clears, not for scripts), target? (feed/command), script? (scripts), intervalMins?, agentId? }` | `{ source, imported }` — a given `target`/`script` runs right away; `agentId` records who changed it |
| `boards.sourceRemove` | `{ projectId, sourceId }` | `Board` (its cards stay; a script's directory is deleted) |
| `boards.sourceRefresh` | `{ projectId, sourceId }` | `{ source, imported }` — run now |

Importers (`BoardSource`): `feed` fetches an RSS/Atom URL with `curl`; `command` runs `sh -c target`
in the project directory; `script` is a program (usually written by an agent through
`kanban_importer_save`) stored in the board and run as `<interpreter from its #! line, else sh>
<file>` in its own directory `<data_dir>/boards/importers/<source id>/` (cwd, `$HOME`,
`$COOCOO_STATE_DIR`; kept between runs) with `$COOCOO_PROJECT_DIR`, `PATH` and `LANG` only, 60 s
timeout. On Linux scripts are sandboxed with Landlock (unprivileged): they can read the system
(`/usr`, `/etc`, `/proc`, …) and the project, write only their directory (and `/dev`); the rest of
the filesystem, including the home directory, is denied; network stays open. `BoardSource.sandboxed`
is the state of the last run (false where Landlock is missing, e.g. macOS). Commands and scripts
print one item per line: plain text (the title, deduped by the line) or a JSON object
`{ title, body?, url? (http/https), id? }` (deduped by `id`, else `url`, else `title`). New entries
go to the top of `inbox` (at most 20 on the first run); entries are remembered, so deleted cards do
not come back. A failing run (non-zero exit: the last stderr lines) lands in `BoardSource.error`.

Card webhooks: when a card enters `doing` (by `boards.cardStart` or a move) its `startUrl`, when it
enters `done` (`boards.cardDone`, `kanban_done` or a move) its `doneUrl` gets a `POST` with JSON
`{ event: "start" | "done", text, card: { id, title, body?, url?, column, note?, commit? }, agent: { id, title }?, project: { id, name, path }?, host, ts }`
(`text` is a one-line summary for chat webhooks that read a `text` field). Sent with `curl` from the
project's host in the background (15 s timeout); a non-2xx answer or network error is stored in
`Card.hookError` (cleared by the next successful call), nothing is retried.

### schedules — `coocoo-agents` (scheduled agent runs per project)
Stored per host in `<data_dir>/schedules.json`. A `Schedule` (`id`, `projectId`, `name`, `prompt`,
`cron?` or `at?`, `timezone`, `agentId?`, `agentTitle?`, `worktree?`, `model?`, `enabled`,
`nextRunAt?`, `lastRun?`, `createdAt`, `updatedAt`, `addedBy?: { id, title }`) runs `prompt` either
in a new agent of the project (title = `name`, optionally in its own worktree and with `model`) or,
with `agentId`, as a message to that agent of the project. `cron` has the five classic fields
(minute hour day-of-month month day-of-week; `*`, lists, ranges, steps, month and weekday names, 0
and 7 = Sunday; day of month and day of week both restricted = either matches) or `@hourly`,
`@daily`, `@weekly`, `@monthly`, `@yearly`, read in the IANA zone `timezone` (a time skipped by a DST
change runs shifted, 02:30 → 03:30); runs are at least 5 minutes apart. `at` (Unix ms) is a one-off
run, after which the schedule disables itself.

| Method | Params | Result |
|---|---|---|
| `schedules.list` | `{ projectId? }` | `Schedule[]` (oldest first; all of the host without `projectId`) |
| `schedules.add` | `{ projectId, name, prompt (≤ 32 KiB), cron? \| at?, timezone? (default: the host's zone), agentId?, worktree?, model?, enabled? (true), byAgentId? }` | `Schedule` — exactly one of `cron`/`at`; `at` must lie in the future; `agentId` must be a non-archived agent of the project (then `worktree`/`model` are dropped); `worktree` needs a git project; `byAgentId`: the agent creating it (`coocoo_schedule_save`), recorded as `addedBy` |
| `schedules.update` | `{ projectId, scheduleId, name?, prompt?, cron?, at?, timezone?, agentId? ("" = new agent per run), worktree?, model? ("" = default), enabled? }` | `Schedule` — `cron` drops `at` and vice versa; `nextRunAt` is recomputed when the timing or `enabled` changes (re-enabling a one-off run whose time has passed is `invalid_params`) |
| `schedules.remove` | `{ projectId, scheduleId }` | `{}` (agents it started stay) |
| `schedules.run` | `{ projectId, scheduleId }` | `Schedule` — runs it now (also when disabled; `nextRunAt` stays) and returns it with the new `lastRun` |
| `schedules.preview` | `{ cron, timezone?, count? (5, ≤ 20) }` | `{ next: [Unix ms] }` — the next run times; `invalid_params` with the parse error otherwise |

Runs happen on the project's host while its daemon runs: the scheduler sleeps until the earliest
`nextRunAt` (at most a minute, as the wall clock can jump) and wakes on every change. A schedule
moves on to its next time before its run starts, so a run missed while the daemon was down (or
draining for a restart) happens once when it is back, never several times. A new agent's chat
starts with the notice `Started by the schedule “<name>”`, then the prompt (its usage-log
`agent.created` has `source: "schedule"`). An existing agent gets the same notice and the prompt
like `agents.prompt`; if it is `starting`, `running` or `waiting` the run is skipped (never queued),
an archived or deleted agent fails it. `lastRun` = `{ at, outcome: "started" | "skipped" | "failed",
agentId?, agentTitle?, message? (why it was skipped or failed), manual? (by schedules.run) }`; every
run is also a `schedule.run` usage-log event.

### secrets — `coocoo-agents` (secrets for agents, per project)
Stored on the project's host in `<data_dir>/secrets/<projectId>.json` (directory 0700, file 0600,
values in plain text, like a hand-written `.env`). No method or event ever returns a value: a
`SecretInfo` is `{ name, files: SecretFile[], addedBy?: { id, title } (the agent that asked; absent =
the user), createdAt, updatedAt }`, a `SecretFile` `{ path (absolute), relPath, format: "dotenv" |
"raw", branch? }` — a file Coocoo wrote the value into. `name` is an env var name
(`^[A-Z_][A-Z0-9_]*$`, ≤ 100 chars; `PATH`, `HOME`, `USER`, `SHELL`, `PWD`, `TERM`, `LANG`,
`TMPDIR`, `IFS` and `COOCOO_*`, `LD_*`, `DYLD_*` are refused), a value ≤ 64 KiB without NUL.

| Method | Params | Result |
|---|---|---|
| `secrets.list` | `{ projectId }` | `SecretInfo[]` sorted by name |
| `secrets.set` | `{ projectId, name, value, file?: { path, format? ("dotenv") } }` | `SecretInfo` — creates or replaces; replacing rewrites every listed file (records of files whose directory is gone, e.g. a removed worktree, are dropped); `file` (relative to the project directory) is written in addition |
| `secrets.remove` | `{ projectId, name }` | `{}` — also removes its `NAME=` line from dotenv files and deletes raw files that still hold exactly the value |
| `secrets.provide` | `{ id, requestId, value }` | `{}` — answers agent `id`'s `secret` request: saves like `secrets.set` (into the agent's directory, `addedBy` = the agent) and resolves the request; on an error (path, git) the request stays open |

Files: the path must stay below the directory (no absolute path, `..`, `.git`; existing symlinks
that lead outside and a target that is not a regular file are refused; missing parent directories
are created). `dotenv` replaces the first `NAME=`/`export NAME=` definition in place (a quoted value
spanning lines as a whole), drops later ones and appends one otherwise; every other line stays
byte for byte. The value is written bare when it has only `A–Z a–z 0–9 _ - . : / + = @ , % ~`, else
single-quoted (literal, may span lines), and only when it contains `'` double-quoted with `\"`/`\\`
escapes. `raw` makes the value the whole file. A new file gets mode 0600, an existing one keeps its
mode (written atomically). In a git repository a file tracked by git is refused (`invalid_params`);
one that is not ignored (`git check-ignore`) is added as `/<path>` to the repository's
`info/exclude` (`git rev-parse --git-path info/exclude`, shared by its worktrees).

**Agents:** every omp process of the project starts with each secret as env var `NAME`, plus
`COOCOO_SECRET_<NAME>` when the name does not match omp's pattern
`/(KEY|SECRET|TOKEN|PASSWORD|PASS|AUTH|CREDENTIAL|PRIVATE|OAUTH)(_|$)/i`, and with
`secrets.enabled: true` in its overlay, so omp replaces the values with placeholders in everything
it sends to the model (and puts the value back into tool arguments). omp collects them only at
session start and ignores values under 8 characters. The host tool `coocoo_secret_request`
(`{ name, description, file?: { path, format? }, multiline?, replace? }`) opens a `UiRequest` with
method `secret` and a `SecretUiPayload` (`{ method: "secret", name, description, projectId,
projectName, multiline?, replace? (it exists), file?: { path, format, gitIgnored, exclude (will be
added to info/exclude) }, branch?, worktree? }`); the agent is `waiting` until the user saves
(`secrets.provide`), declines (`agents.respond` with `{ cancelled: true }`) or omp cancels the call.
The tool result names the variable, the value's length and the files, never the value. A secret
the project has already (without `replace`) returns at once, after writing the value into a
requested file it is not in yet. When the running process does not have the secret (new or
changed since it started), the agent is told to end its turn; once it is idle (no prompt, dialog
or subagent left) Coocoo hibernates it with the notice `Restarting the session so it gets the
secret NAME` and prompts it to continue, the new process having `$NAME`. Values (and their
JSON-escaped forms; at least 4 characters, also when cut off by a truncation) are replaced with
`••••NAME` in every timeline item, streaming delta (a chunk ending in the start of a value is held
back until the next one), `UiRequest` payload, `agents.output` text and `agents.subagentTimeline`
page before it is stored or sent — omp's RPC events carry tool output unmasked. Re-encoded values
(base64, hex, substrings) are caught by neither. Running agents see `secrets.set`/`remove`
changes at their next process start; values they started with stay redacted until then. A value
for another host travels client → home daemon → peer link like every other request: inside an SSH
tunnel for SSH hosts, as plain as the rest of the traffic on a direct `ws://` link. The usage log
records `secrets.set`/`remove`/`provide` with ids only (never name or value).

### browser — `coocoo-browser`
| Method | Params | Result |
|---|---|---|
| `browser.profiles` | – | `BrowserProfileInfo[]` |
| `browser.cdpEndpoint` | `{ profile }` | `{ url }` — `http://127.0.0.1:<port>` discovery root for the profile (bound on first call, stable for the daemon's lifetime; does not start Chromium) |
| `browser.start` / `browser.stop` | `{ profile }` | `BrowserProfileInfo` |
| `browser.open` | `{ profile, url }` | `BrowserTab` (starts the profile if needed) |
| `browser.navigate` | `{ profile, targetId, url? , action?: "back" \| "forward" \| "reload" }` | `{}` |
| `browser.activate` | `{ profile, targetId }` | `{}` |
| `browser.closeTab` | `{ profile, targetId }` | `{}` |
| `browser.watch` | `{ profile, targetId, maxWidth?, maxHeight?, quality? }` | `{}` — this connection receives `browser.frame` + `browser.console` for the tab until `browser.unwatch` or disconnect |
| `browser.unwatch` | `{ profile, targetId }` | `{}` |
| `browser.resize` | `{ profile, targetId, width, height }` | `{}` — viewport in CSS px (fits page to the viewer pane) |
| `browser.input` | `{ profile, targetId, event }` | `{}` — `event`: `{type:"mouse", action:"down"\|"up"\|"move"\|"wheel", x, y, button?, clickCount?, deltaX?, deltaY?, modifiers?}`, `{type:"key", action:"down"\|"up", key, code, text?, keyCode?, modifiers?}`, `{type:"text", text}`; coordinates in CSS px of the page viewport; `modifiers` = CDP bitmask (Alt 1, Ctrl 2, Meta 4, Shift 8), a list of names, or `{alt,ctrl,meta,shift}` flags; `button` = DOM name or number |
| `browser.console` | `{ profile, targetId }` | `BrowserConsoleEntry[]` (last 500) |
| `browser.screenshot` | `{ profile, targetId }` | `{ data: base64 PNG }` |
| `browser.inspect` | `{ profile, targetId, x, y }` | `{ selector, tag, html (outer, ≤ 4 KiB), text (≤ 1 KiB), styles: {prop: value} (layout/typography/color subset), box: {x,y,width,height}, screenshot: base64 PNG of the element (≤ 800 px) }` — "design mode": the UI attaches this to a prompt |
| `browser.highlight` | `{ profile, targetId, x?, y? }` | `{}` — overlay-highlight the element under the point (omit x/y to clear) |

### insights — `coocoo-core` + `coocoo-insights` (Stats and Workflows tabs)
Usage statistics of Coocoo agents (incl. their subagents and advisor) and the workflows around them.
Every host answers for its own data; the home daemon asks each online peer with `localOnly: true`
and merges, so a client and the `coocoo_insights` agent tool see the same numbers. Hosts that are
offline, too old or slow (80 s) are listed with `ok: false` and `error`; their numbers are missing.

`InsightsQuery`: `{ from, to (Unix ms, [from, to)), tzOffset (client's UTC offset in minutes, east
positive: days and the heatmap use it), project? (id), model? (omp model name), hosts?: [id],
compare? (default true: also the equally long period before `from`) }`.

| Method | Params | Result |
|---|---|---|
| `insights.usage` | `InsightsQuery` | `UsageReport` — `hosts` (per host: `statsDb`, `syncedAt`, `schema` = omp's `rollup_version`, `statsError`, `logSince`, `logEnabled`), `totals` / `prev` (cost split input/output/cache read/cache write, tokens, requests, failed, aborted, prompts, commits, lines, agents, subagents, activeMs, activeDays, streak, cardsDone), `days` (every local day: cost/tokens/requests per model, commits, prompts, flagged, cardsDone), `models` (cacheHit, ttft, tps, failed), `projects`, `agents` (cost by main/subagents/advisor, session files, commits with `match`, `dayCost`, `deleted`), `tools`, `heatmap` (weekday × hour prompts), `frustration` (flagged prompts with omp's signals; `examples` carry the prompt text, ≤ 400 chars), `allModels` |
| `insights.workflows` | `InsightsQuery` | `WorkflowReport` — `kpis` (agents started, prompts per agent, autonomy, needs-you wait, abort rate, agent → commit; with `prev`), `journey` and `kanban` funnels (count, median time to the next stage, `stuck` examples), `shapes`, `sequences` (3–5 step n-grams per agent or 30-min client session), `features`, `friction`, `lastAnalyze`, `logSince`, `events`/`reconstructed` counts. Examples are `ExampleRef { hostId, agentId?, projectId?, cardId?, label?, ts }` |
| `insights.events` | `InsightsQuery & { limit? (10 000) }` | `{ events: InsightEvent[], truncated, hosts }` — the recorded log, oldest first, newest `limit` kept |
| `insights.track` | `{ clientId, device, events: [{ ts, kind ("ui.…"), agentId?, projectId?, cardId?, meta? }] }` | `{}` — client events, recorded by the home daemon (≤ 500 per call; meta values are cut to short strings, numbers and booleans) |
| `insights.settings` | `{ enabled?, retentionDays? }` (per host) | `{ enabled, retentionDays, logSince?, bytes }` — `[insights]` in `config.toml`; switching off drops buffered events |
| `insights.clear` | – (per host) | same as `insights.settings` — deletes the log files |
| `insights.analyze` | `{ projectId, from, to, tzOffset, focus? }` (on the project's host) | `AgentSummary` — starts the pattern-analysis agent with a fixed prompt: read the insights with `coocoo_insights`, compare with the code read-only, add 3–7 evidence-backed inbox cards with `kanban_add`, implement nothing. `lastAnalyze` counts its cards and how many left the inbox |

**Sources.** Tokens, cost, requests, tool calls and prompts come from omp's `~/.omp/stats.db`,
opened read-only after `omp stats --summary` brought it up to date (at most once per minute, only
when queried, 60 s limit). Only the columns Coocoo reads are checked; a missing one is reported as
`statsError` instead of guessed. Rows belong to an agent by `session_file`: its main session files
(every one it had, from the agent ledger) and everything under `<file without .jsonl>/`
(subagents, advisor). Cost is omp's `cost_total` at API prices — the API equivalent, not what a
subscription bills. Commits come from `git log --branches --no-merges --shortstat` of the projects the
agents worked in; a commit is `exact` when a card recorded it (`kanban_done`), `turn` when its
committer time lies within one of the agent's turns (± 60 s), `shared` when within turns of several
agents of the project; others are not counted. A turn runs from a prompt to the last response before
the next prompt; active time sums the gaps between them, each capped at 10 min.

**Usage log.** `<data_dir>/events/YYYY-MM.jsonl` (UTC months), append-only, buffered ≤ 2 s,
months older than `retentionDays` (default 365, 0 = forever) are deleted. One `InsightEvent` per
line: `{ ts, kind, actor: "user" | "agent" | "daemon", device?, clientId?, agentId?, projectId?, cardId?, ok, ms?, meta }`.
It never holds prompt texts, arguments, file contents or paths; `meta` holds ids, counts and names
(prompt length, image count, a slash command's name, a panel or dialog name, a model). Recorded are:
- **Actions** with user intent, by the host that executes them, once (nested calls are not
  recorded again): `agents.create/prompt/abort/respond/setModel/setThinking/markRead/rename/hibernate/archive/delete`,
  `boards.card*`, `boards.update`, `boards.source*`, `git.commit/merge/push/fetch/fastForward/resetToBase/prCreate/prMerge/worktreeCreate/worktreeRemove`,
  `browser.start/open/navigate/watch/input` (input at most every 10 min per client and profile),
  `terminal.open`, `actions.run`, `projects.add/remove`, `hosts.add/remove`, `insights.analyze`.
  `actor` is `user` for a client's request (with `device`, `clientId` from `system.hello`), `agent`
  for a host tool's call (`meta.by` = the agent), and comes with the request when a peer link
  forwards it (`Request.origin`). The daemon's own calls (diff stats, refreshes) are not recorded.
- **Domain events** from the agents subsystem, carrying the actor of the request that caused them:
  `agent.created` (`meta.source` user/card/agent), `agent.deleted`, `turn.start`, `turn.end`
  (`ms`, `meta.stop` completed/aborted/error, `meta.tools`), `request.ask` / `request.answer` /
  `request.cancel` (`ms` = wait, `meta.reason` user/omp/process), `model.change` (`meta.cause`
  user/switch/fallback/respawn), `retry.start`/`retry.end`, `compaction`, `agent.crash`,
  `agent.hibernate`, `agent.resume`, `command.refused`, `card.add`, `card.move` (`meta.from`/`to`),
  `card.refined`, `card.delete`.
- **Client events** (`insights.track`): `ui.route`, `ui.tab`, `ui.panel`, `ui.palette.open|run`,
  `ui.shortcut`, `ui.dialog.open|submit|cancel`, `ui.slash`, `ui.model.open|pick`, `ui.leave`,
  `ui.return`, `ui.example`.

Workflow data from before a host's log began is reconstructed (marked `reconstructed`) from agent
timelines, cards and omp's statistics. The agent ledger `<data_dir>/insights/ledger.jsonl` keeps
each agent's title, project, directory, worktree, session files and deletion time, so deleted
agents stay in the statistics.

## Events

| Event | Payload | Scope |
|---|---|---|
| `host.updated` | `HostInfo` | all |
| `host.removed` | `{ id }` | all |
| `host.stats` | `HostStats` | all (only while watched) |
| `project.updated` / `project.removed` | `Project` / `{ id }` | all |
| `agent.updated` / `agent.removed` | `AgentSummary` / `{ id }` | all |
| `agent.timeline` | `TimelineEvent` (upsert by `item.id`) — a tool item's `tool.subagents` lists the subagents the call started (`status`, `activity`, `toolCount`, `costUsd`, `model`, `sessionFile`); their live progress arrives as further upserts at most once per second | all |
| `agent.delta` | `TimelineDelta` (append to partial item text) | all |
| `agent.request` / `agent.requestResolved` | `UiRequest` / `{ agentId, requestId }` | all |
| `project.board` | `BoardEvent` (`{ projectId, board }`, the whole board after every change) | all |
| `project.schedules` | `{ projectId, schedules: Schedule[] }` — the project's whole list after every change, including each run | all |
| `project.secrets` | `{ projectId, secrets: SecretInfo[] }` — the project's whole list after every change (never values) | all |
| `agent.commands` | `{ agentId, commands: SlashCommand[] }` — the catalog changed (startup, skills/plugins reloaded) | all |
| `browser.updated` | `BrowserProfileInfo` | all |
| `browser.frame` | `BrowserFrame` | watching connection only |
| `browser.console` | `BrowserConsoleEntry` | watching connection only |
| `terminal.output` | `{ id, data }` (UTF-8 text) | attached connections only |
| `terminal.exit` | `{ id, code? }` (`null` when killed by a signal) | attached connections only |
| `terminal.updated` / `terminal.removed` | `TerminalInfo` (opened, or its process ended) / `{ id }` | all |
| `system.resync` | `{}` | connection that lagged; refetch everything |

## omp integration

Each agent is one `omp --mode rpc-ui` process (same wire protocol as `rpc`, plus omp's tool dialogs
such as `ask`; via the vendored official Rust client `crates/omp-rpc`). The daemon:

- passes `--config <overlay>` setting `browser.cdpUrl` to the profile's discovery root from
  `browser.cdpEndpoint` (`http://127.0.0.1:<port>`; profile = project id, or `default`), so omp's
  own browser tool drives the daemon-managed, per-project Chromium that the UI live-views; the
  spawn fails with an agent error if the endpoint cannot be obtained;
- registers host tools (`coocoo_hosts`, `coocoo_projects`, `coocoo_agents`, `coocoo_spawn`,
  `coocoo_send`, `coocoo_wait`) so agents orchestrate other agents on any host of the fleet,
  `coocoo_insights` (read-only: the Stats/Workflows numbers by section — usage, workflows,
  friction, features, sequences, events — for a period, project and host),
  `coocoo_actions` (read-only: the project's parsed `.coocoo/actions.toml` and its problems; the tool description carries the file format),
  `coocoo_task_summary` (sets `AgentSummary.task` = `{ text, since? }`: one or two sentences on the agent's current task, single line, ≤ 400 bytes, and the time of the newest user message, so the UI pins it above the transcript and jumps to that message),
  `kanban_list`, `kanban_add` (suggestions go to the inbox, work the user asked for to todo), `kanban_move`, `kanban_done`, `kanban_refine` (attach a refinement with mocks, see `boards.refinementSave`) for the board of the agent's project, `kanban_importers`, `kanban_importer_save` (create/change a script or feed importer; runs it once and reports the result), `kanban_importer_delete` for its inbox importers (agents cannot create or change `command` importers),
  `coocoo_schedules`, `coocoo_schedule_save` (create/change a scheduled run of the agent's project: `cron` or `at` as an ISO 8601 date-time, read in `timezone` without an offset; `agentId: "self"` targets the agent itself) and `coocoo_schedule_delete`, and
  `coocoo_secret_request` (ask the user for a secret without seeing it, see `secrets`);
- passes `--append-system-prompt` telling the agent it runs in Coocoo, to call `coocoo_task_summary` first whenever it starts a new task, and to get secrets only through `coocoo_secret_request` and never print or re-encode them;
- enables `set_ask_dialog` and surfaces omp `extension_ui_request`s as `UiRequest`s;
- starts every process of a project with its secrets in the env and omp's secret obfuscation on;
- hibernates idle agents (process exit, session kept) and resumes them with `--resume <sessionFile>`.
