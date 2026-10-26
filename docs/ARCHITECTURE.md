# Architecture

```
            ┌──────────── desktop (Tauri 2) ────────────┐        ┌──── server (headless) ────┐
 phone/web ─┤  system webview  ◀─ws─▶  coocoo-core       │◀─ws──▶│  coocoo serve             │
   (any     │  (ui/, Svelte 5)        (in-process daemon)│ direct │  (same daemon, no GUI)    │
  browser)  │                          │   │   │         │ or SSH │   │    │    │             │
            └──────────────────────────┼───┼───┼─────────┘ tunnel └───┼────┼────┼─────────────┘
                                       ▼   ▼   ▼                      ▼    ▼    ▼
                           omp --mode rpc-ui Chromium  git          omp   Chromium  git
                           (per agent)     (per project, CDP)
```

- **One daemon per machine** (`crates/coocoo-core`). The desktop app embeds it; servers run the
  `coocoo` binary. Every daemon serves the same WebSocket protocol (`docs/PROTOCOL.md`) and the
  same web UI, so a phone can open any daemon in a browser.
- **Home daemon + peer links.** A client connects to one daemon. That daemon keeps a link to
  each peer (direct `ws://` over LAN/Tailscale/WireGuard, or an `ssh -L` tunnel it spawns and
  supervises) and forwards requests carrying a `host` field. Events carry their origin host id
  and are relayed unchanged; peer links subscribe to local-origin events only, so there are no
  relay loops. Screencast frames are routed only to the client connections watching that tab.
- **Subsystems never depend on each other.** `coocoo-workspace` (projects, git, worktrees, fs,
  host stats, terminals), `coocoo-agents` (omp), and `coocoo-browser` (Chromium/CDP) depend only on
  `coocoo-proto`. Cross-cutting calls (an agent creating a worktree, an agent spawning another
  agent on a different host) go through the `Orchestrator` trait, which is the same routed call
  path a client uses.
- **Usage log and insights.** `crates/coocoo-insights` (used only by `coocoo-core`) records a
  content-free event log per host and answers the Stats/Workflows queries. Actions are recorded at
  the one routing point (`App::call`) by the host that executes them, with the actor carried along
  (`Origin`: a client's user and device, an agent's host tool, or forwarded by a peer link); the
  agents subsystem reports domain events (turns, dialogs, card moves, crashes) through the narrow
  `InsightsSink` trait in `coocoo-proto`, and they inherit the actor of the request that caused
  them via a task-local. Queries fan out to every online host and are merged once on the home
  daemon, so the UI and the `coocoo_insights` agent tool see the same numbers.
- **Web Push.** `crates/coocoo-core/src/push.rs` subscribes a device on the daemon it talks to
  (`push.*`, `docs/PROTOCOL.md`) and pushes that daemon's own agent events (`agent.request`,
  unread `agent.updated`) — encrypted (RFC 8188) and VAPID-authorized — over rustls with
  webpki roots; no OpenSSL: `vendor/ece` drops ece's openssl default backend, and
  `crates/coocoo-core/src/ece_backend.rs` installs a pure-Rust `Cryptographer` (p256,
  aes-gcm, hkdf, sha2). Payloads carry only the notification title, agent title (per the
  device's privacy preference) and deep link — never transcript text. Keys and subscriptions
  live in `<data_dir>/push/` (`vapid.key` 0600, `subscriptions.json` atomic). The UI registers
  `/sw.js` (root scope, so pushes arrive while the app is closed; iOS needs the Home Screen
  install) and acks visibility (`push.ack`) so a visible tab suppresses pushes.

## omp integration (the only harness)

Each agent is one `omp --mode rpc-ui` process (the RPC protocol with omp's tool dialogs enabled)
driven by the official Rust client (vendored in `crates/omp-rpc`):

- **Browser:** a per-agent `--config` overlay sets `browser.cdpUrl` to the project's loopback
  CDP discovery root (`browser.cdpEndpoint`, a dedicated `http://127.0.0.1:<port>` per profile,
  since puppeteer drops any path from `cdpUrl`). omp keeps its full browser tool (aria snapshots with
  delta refs, screenshots with change detection, console, network, emulation…) and drives the
  daemon-owned Chromium for that project — the same tabs every client live-views and can take
  over.
- **Orchestration:** host tools (`coocoo_spawn`, `coocoo_send`, `coocoo_wait`, `coocoo_agents`,
  `coocoo_projects`, `coocoo_hosts`, `coocoo_output`) are registered over RPC — no MCP server, no
  extra process — and route through the Orchestrator, so an agent on a laptop can start and
  supervise agents on any server in the fleet. `coocoo_schedules`, `coocoo_schedule_save` and
  `coocoo_schedule_delete` let an agent schedule runs in its project (`schedules.*`).
- **Attention:** omp `ask`/`confirm`/`select`/`input` dialogs become `UiRequest`s, surfaced in the
  overview's needs-you queue and answerable from any client.
- **Secrets:** `coocoo_secret_request` opens a `secret` `UiRequest`; the value goes from the client
  to the daemon of the agent's host only (`secrets.provide`), never through omp. It is stored per
  project (`<data_dir>/secrets/`, 0600) and given to every process of the project as an env var
  (optionally also written into a `.env`/raw file in the agent's directory, kept out of git via
  `.git/info/exclude`), with omp's secret obfuscation on, so the model sees placeholders and
  commands still run with the value. omp collects secrets only at session start, so the agent is
  restarted (hibernate + resume + "continue" prompt) once its turn ends. omp masks only what goes
  to the provider, so Coocoo redacts the values (`••••NAME`) from every timeline item, delta,
  dialog and output before storing or sending them. Not a sandbox: the agent runs as the same
  user and could read the store; a deliberately re-encoded value (base64, hex, substrings) or one
  under 8 characters reaches the model unmasked.
- **Lifecycle:** status follows `prompt_result`/`session_settled`, not just `agent_end`. Idle
  agents hibernate (process exits, session kept) and resume with `--resume <sessionFile>` on the
  next prompt. A crashing omp process only marks its own agent as errored.
- **Restarts:** SIGTERM/Ctrl+C to `coocoo serve` (a redeploy, e.g. from an agent working on
  Coocoo itself) first drains: agents in a turn are steered to pause and end their turn, new
  prompts and agents are refused, and the daemon waits until those turns ended (at most
  `agents.restartGraceSecs`, default 60 s; agents waiting for a user answer are not waited for; a
  second signal stops at once). Agents that paused or were cut off are marked in `meta.json`, and
  the next start prompts them that the restart is done so they continue. Under systemd this needs
  `KillMode=mixed` (the unit Coocoo installs over SSH sets it); with the default, systemd sends
  SIGTERM to the omp processes too. The desktop app's Quit stops without draining.

## Resource discipline

| Concern | Approach |
|---|---|
| Desktop shell | Tauri 2 + system webview; the daemon runs in the same process |
| Idle cost | No timers or polling while nobody watches: stats sample only during `stats.watch`, git refresh is driven by a `.git` watcher (the project actions reload by a name-filtered watch on the project directory and `.coocoo/`), Chromium starts on first use and stops when idle; the scheduler sleeps until the next scheduled run (re-reading the wall clock at least once a minute) and has no timer without an enabled schedule |
| Agents | omp processes hibernate after inactivity; timelines live on disk (JSONL), memory holds summaries |
| Event fan-out | Each event is serialized once into `Arc<str>` and shared by all connections; bounded queues; slow clients get `system.resync` instead of unbounded buffers |
| Browser | One Chromium per project, headless; screencast only while a client watches, fps-capped, JPEG passed through without re-encoding; lossy queue for frames |
| git | `git` CLI with `GIT_OPTIONAL_LOCKS=0`; no libgit2; diffs size-capped. GitHub pull requests through the `gh` CLI; their status is network, so it is looked up only after a push/PR action or fetch and when a client opens the git popover (cached, ≥ 60 s apart), never on watcher refreshes |
| UI | Svelte 5, no UI framework, lazy-loaded heavy views (Stats, Workflows), tail-rendered transcripts; charts are hand-built SVG |
| Usage log | Append-only JSONL per UTC month, buffered and written ≤ 2 s after the first event of a batch (no timer while nothing happens); clients batch their events (10 s, on hide). omp's `stats.db` is synced (`omp stats --summary`) only when a query asks, at most once a minute, and read-only; nothing polls |
