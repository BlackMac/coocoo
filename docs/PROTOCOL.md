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
| `system.hello` | – | `{ protocol: u32, host: HostInfo, hosts: HostInfo[] }` (`hosts` = local + peers) |
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
| `projects.add` | `{ path, name? }` | `Project` (directory must exist; git optional) |
| `projects.update` | `{ id, name?, devUrls? }` | `Project` |
| `projects.remove` | `{ id }` | `{}` (never touches files) |
| `projects.refresh` | `{ id }` | `Project` (recomputes `git` and `icon`) |
| `projects.icon` | `{ id }` | `{ mime, data: base64 }` — the file named by `Project.icon` (`not_found` without one). `icon` is picked from the project's favicon/app icon/logo at the usual places (root, `public/`, `static/`, `app/`, `src/app/`, `assets/`, `src/assets/`, `src/`, `web/`, `src-tauri/icons/`, `resources/`, `build/`, `.github/`, `docs/`): icons before logos, SVG before PNG before ICO, ≤ 512 KiB; re-detected on every git refresh |
| `git.diff` | `{ cwd, base? }` | `DiffResult` — working tree (incl. untracked) vs `HEAD`, or vs `merge-base(base, HEAD)` when `base` is given |
| `git.diffStat` | `{ cwd, base? }` | `DiffStat` — same comparison as `git.diff`, counts only |
| `git.log` | `{ cwd, limit? (20) }` | `CommitInfo[]` |
| `git.commit` | `{ cwd, message }` | `CommitInfo` — stages everything (`git add -A`) then commits |
| `git.merge` | `{ projectId, branch }` | `{ ok: bool, output: String }` — merges `branch` into the project's checked-out branch (no fast-forward commit message prompt; aborts and reports on conflict) |
| `git.worktreeCreate` | `{ projectId, branch?, base? }` | `WorktreeRef` — new branch (default `coocoo/<id>`) from `base` (default current branch) under `<data_dir>/worktrees/<project-name>-<project-id>/<branch-slug>` |
| `git.worktreeRemove` | `{ projectId, path, deleteBranch?: bool }` | `{}` |
| `fs.list` | `{ path? }` (default home) | `{ path, parent?, entries: FsEntry[] }` — directories first, hidden entries omitted |

### terminal — `coocoo-workspace` (Linux/macOS hosts)
A terminal is a login shell (`$SHELL -l`, else `/bin/sh`, with `TERM=xterm-256color`) on a pty. It
keeps running while nobody is attached and ends when the shell exits or on `terminal.kill`.

| Method | Params | Result |
|---|---|---|
| `terminal.list` | – | `TerminalInfo[]` (oldest first) |
| `terminal.open` | `{ cwd, cols? (80), rows? (24) }` | `TerminalInfo` — `cwd` must be a directory; does not attach |
| `terminal.attach` | `{ id, cols?, rows? }` | `{ data: String }` — the recent output (≤ 256 KiB); this connection then receives `terminal.output`/`terminal.exit` until `terminal.detach` or disconnect. Output may arrive before this reply: apply it after `data` |
| `terminal.detach` | `{ id }` | `{}` — the shell keeps running |
| `terminal.input` | `{ id, data }` | `{}` — keystrokes/paste as UTF-8 text (≤ 256 KiB) |
| `terminal.resize` | `{ id, cols, rows }` | `{}` — last resize wins when several clients are attached |
| `terminal.kill` | `{ id }` | `{}` — SIGHUP to the shell's process group |

### agents — `coocoo-agents` (omp only)
| Method | Params | Result |
|---|---|---|
| `agents.list` | `{ projectId? }` | `AgentSummary[]` |
| `agents.get` | `{ id }` | `AgentDetail` |
| `agents.create` | `{ projectId?, cwd?, title?, prompt?, worktree?: bool \| { branch?, base? }, model?, thinking?, parentId? }` | `AgentSummary` — one of `projectId`/`cwd` required |
| `agents.prompt` | `{ id, message, mode?: "auto" \| "steer" \| "followUp", images?: ImageInput[] }` | `{}` — `auto`: prompt when idle, follow-up when running; wakes hibernated agents |
| `agents.abort` | `{ id }` | `{}` |
| `agents.respond` | `{ id, requestId, response }` | `{}` — see `UiRequest` docs for `response` shapes |
| `agents.timeline` | `{ id, before?: seq, limit? (200) }` | `TimelinePage` |
| `agents.output` | `{ id }` | `{ status: AgentStatus, text: String? }` — full last assistant text |
| `agents.markRead` | `{ id }` | `{}` |
| `agents.rename` | `{ id, title }` | `AgentSummary` |
| `agents.setModel` | `{ id, model: "provider/id" }` | `AgentSummary` |
| `agents.setThinking` | `{ id, level }` | `AgentSummary` |
| `agents.models` | – | `ModelInfo[]` (cached per host) |
| `agents.commands` | `{ id }` | `SlashCommand[]` — the agent's slash-command catalog from omp (`name`, `source` builtin/skill/extension/custom/mcp_prompt/file, `aliases`, `description`, `hint`, `subcommands`), as reported by its process; an agent that never ran one gets it from a throwaway omp in its directory. Sending `/name args` with `agents.prompt` runs the command: omp-local ones answer with a `command` timeline item (plain text) after an echoed `user` item; skills and custom commands start a turn |
| `agents.usage` | `{ provider, model? }` | `ProviderUsage` — the provider's subscription quota on this host from `omp usage --json --provider <provider>`: `windows` (one per window and quota pool, summed over all logged-in accounts: `used` in accounts' worth, earliest `resetsAt`; `name` labels a pool only some models draw from, such as an Anthropic tier, Codex Spark or an Antigravity counter like `Gemini`; `gating` marks the windows `model`, the model id within the provider, draws from) and per-account `limits` (`unit` percent/usd/tokens/…, `usedFraction`, `status`, `resetsAt`; a quota omp reports once per routing family is listed once). Empty for pay-per-token providers. Cached 60 s per provider; `unavailable` if omp fails or takes over 20 s |
| `agents.subagentTimeline` | `{ id, sessionFile, live? }` | `TimelinePage` — read-only transcript of one of the agent's subagents, read from its omp session file (`Subagent.sessionFile`; only files below the agent's own session directory, else `forbidden`); newest 500 items. `live`: the subagent still runs, so unanswered tool calls show as running instead of interrupted |
| `agents.hibernate` | `{ id }` | `AgentSummary` |
| `agents.archive` | `{ id }` | `AgentSummary` (status `stopped`, process gone, read-only) |
| `agents.delete` | `{ id, removeWorktree?: bool }` | `{}` |

### boards — `coocoo-agents` (one Kanban board per project)
Stored in `<data_dir>/boards/<project id>.json`; boards that earlier versions kept per agent move to
the agent's project on startup (cards in doing/done keep that agent). Every method takes
`projectId` (`not_found` for an unknown project).

| Method | Params | Result |
|---|---|---|
| `boards.get` | `{ projectId }` | `Board` — `cards` (`column` inbox/todo/doing/done; a column's order is the order of its cards, first = top; `agentId`/`agentTitle` = the agent it was handed to or that finished it; `refining` while its agent works out a refinement; `refinement`: `{ text (Markdown), attachments?: [{ name, mime, size }], agentId?, agentTitle?, at }`) and `sources` |
| `boards.cardAdd` | `{ projectId, title, body?, column? (todo), top?: bool, startUrl?, doneUrl? }` | `Card` — `startUrl`/`doneUrl`: http(s) webhooks, see below |
| `boards.cardUpdate` | `{ projectId, cardId, title?, body?, startUrl?, doneUrl? ("" clears each), column?, index? }` | `Card` — `index` = position in the (target) column, 0 = top, past the end = bottom |
| `boards.cardDelete` | `{ projectId, cardId }` | `{}` (its refinement attachments are deleted) |
| `boards.cardStart` | `{ projectId, cardId, agentId?, worktree?: bool, model? }` | `Card` — prompts `agentId` (one of the project's agents, not archived) with the card (title, body, link, how to finish it) like `agents.prompt` (`auto`), or without `agentId` creates a new agent for it (title = card title, optionally in its own worktree); then records the agent on the card and moves it to the top of `doing`. `conflict` if it is already in doing/done, or if that agent is busy (`starting`, `running` or `waiting`): a card never queues behind a running answer. A refined card's prompt carries its refinement and the attachments' paths |
| `boards.cardRefine` | `{ projectId, cardId, agentId?, worktree?: bool, model? }` | `Card` — like `boards.cardStart`, but the prompt asks for a refinement instead of the work: research what the agent can find out itself, ask the user (`ask`) only the rest, optionally make UI mocks, then call `kanban_refine`. The card stays in `todo` (an inbox card moves to the bottom of `todo`) with the agent recorded and `refining: true`; an earlier refinement is included for revision |
| `boards.refinementSave` | `{ projectId, cardId, agentId?, text (Markdown, ≤ 32 KiB), attachments?: [path] }` | `Card` — what `kanban_refine` calls: replaces the card's refinement and clears `refining`. Attachments (≤ 10, ≤ 5 MiB each; png, jpg, gif, webp, svg, html) are copied to `<data_dir>/boards/attachments/<card id>/`; relative paths resolve in the agent's directory |
| `boards.attachment` | `{ projectId, cardId, name }` | `{ mime, data }` (base64) — a file of the card's refinement. The UI shows HTML mocks only in `sandbox="allow-scripts"` frames (no app origin) |
| `boards.cardDone` | `{ projectId, cardId, agentId? (the card's agent), commitMessage? (card title), note? }` | `{ card, commit?: CommitInfo, commitSkipped?: "not a git repository" \| "nothing to commit" }` — `git.commit` in that agent's directory (the project's without one), records the agent, then moves the card to the top of `done`; a failing commit leaves the card in place and returns the error |
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
| `agent.commands` | `{ agentId, commands: SlashCommand[] }` — the catalog changed (startup, skills/plugins reloaded) | all |
| `browser.updated` | `BrowserProfileInfo` | all |
| `browser.frame` | `BrowserFrame` | watching connection only |
| `browser.console` | `BrowserConsoleEntry` | watching connection only |
| `terminal.output` | `{ id, data }` (UTF-8 text) | attached connections only |
| `terminal.exit` | `{ id, code? }` (`null` when killed by a signal) | attached connections only |
| `system.resync` | `{}` | connection that lagged; refetch everything |

## omp integration

Each agent is one `omp --mode rpc-ui` process (same wire protocol as `rpc`, plus omp's tool dialogs
such as `ask`; via the vendored official Rust client `crates/omp-rpc`). The daemon:

- passes `--config <overlay>` setting `browser.cdpUrl` to the profile's discovery root from
  `browser.cdpEndpoint` (`http://127.0.0.1:<port>`; profile = project id, or `default`), so omp's
  own browser tool drives the daemon-managed, per-project Chromium that the UI live-views; the
  spawn fails with an agent error if the endpoint cannot be obtained;
- registers host tools (`coocoo_hosts`, `coocoo_projects`, `coocoo_agents`, `coocoo_spawn`,
  `coocoo_send`, `coocoo_wait`) so agents orchestrate other agents on any host of the fleet, and
  `kanban_list`, `kanban_add`, `kanban_move`, `kanban_done`, `kanban_refine` (attach a refinement with mocks, see `boards.refinementSave`) for the board of the agent's project, and `kanban_importers`, `kanban_importer_save` (create/change a script or feed importer; runs it once and reports the result), `kanban_importer_delete` for its inbox importers (agents cannot create or change `command` importers);
- enables `set_ask_dialog` and surfaces omp `extension_ui_request`s as `UiRequest`s;
- hibernates idle agents (process exit, session kept) and resumes them with `--resume <sessionFile>`.
