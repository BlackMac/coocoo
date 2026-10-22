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
  disconnects, and is re-established after a reconnect.
- Error codes: `invalid_params`, `not_found`, `unknown_method`, `conflict`, `unauthorized`,
  `unavailable`, `forbidden`, `internal`.
- HTTP (same port): `GET /api/health` (no auth, CORS `*`) → `{ok, hostId, name, version}`;
  `GET /cdp/<profile>/json/version` (loopback clients and loopback `Host` header only, no token) →
  Chrome's discovery JSON, launching that profile's Chromium on demand; static UI at `/` (web/phone
  access, SPA fallback to `index.html`).

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
| `projects.refresh` | `{ id }` | `Project` (recomputes `git`) |
| `git.diff` | `{ cwd, base? }` | `DiffResult` — working tree (incl. untracked) vs `HEAD`, or vs `merge-base(base, HEAD)` when `base` is given |
| `git.diffStat` | `{ cwd, base? }` | `DiffStat` — same comparison as `git.diff`, counts only |
| `git.log` | `{ cwd, limit? (20) }` | `CommitInfo[]` |
| `git.commit` | `{ cwd, message }` | `CommitInfo` — stages everything (`git add -A`) then commits |
| `git.merge` | `{ projectId, branch }` | `{ ok: bool, output: String }` — merges `branch` into the project's checked-out branch (no fast-forward commit message prompt; aborts and reports on conflict) |
| `git.worktreeCreate` | `{ projectId, branch?, base? }` | `WorktreeRef` — new branch (default `coocoo/<id>`) from `base` (default current branch) under `<data_dir>/worktrees/<project-name>-<project-id>/<branch-slug>` |
| `git.worktreeRemove` | `{ projectId, path, deleteBranch?: bool }` | `{}` |
| `fs.list` | `{ path? }` (default home) | `{ path, parent?, entries: FsEntry[] }` — directories first, hidden entries omitted |

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
| `agents.hibernate` | `{ id }` | `AgentSummary` |
| `agents.archive` | `{ id }` | `AgentSummary` (status `stopped`, process gone, read-only) |
| `agents.delete` | `{ id, removeWorktree?: bool }` | `{}` |

### browser — `coocoo-browser`
| Method | Params | Result |
|---|---|---|
| `browser.profiles` | – | `BrowserProfileInfo[]` |
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
| `agent.timeline` | `TimelineEvent` (upsert by `item.id`) | all |
| `agent.delta` | `TimelineDelta` (append to partial item text) | all |
| `agent.request` / `agent.requestResolved` | `UiRequest` / `{ agentId, requestId }` | all |
| `browser.updated` | `BrowserProfileInfo` | all |
| `browser.frame` | `BrowserFrame` | watching connection only |
| `browser.console` | `BrowserConsoleEntry` | watching connection only |
| `system.resync` | `{}` | connection that lagged; refetch everything |

## omp integration

Each agent is one `omp --mode rpc-ui` process (same wire protocol as `rpc`, plus omp's tool dialogs
such as `ask`; via the vendored official Rust client `crates/omp-rpc`). The daemon:

- passes `--config <overlay>` setting `browser.cdpUrl = http://127.0.0.1:<port>/cdp/<profile>`, so
  omp's own browser tool drives the daemon-managed, per-project Chromium that the UI live-views;
- registers host tools (`coocoo_hosts`, `coocoo_projects`, `coocoo_agents`, `coocoo_spawn`,
  `coocoo_send`, `coocoo_wait`) so agents orchestrate other agents on any host of the fleet;
- enables `set_ask_dialog` and surfaces omp `extension_ui_request`s as `UiRequest`s;
- hibernates idle agents (process exit, session kept) and resumes them with `--resume <sessionFile>`.
