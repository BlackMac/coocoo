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
  host stats), `coocoo-agents` (omp), and `coocoo-browser` (Chromium/CDP) depend only on
  `coocoo-proto`. Cross-cutting calls (an agent creating a worktree, an agent spawning another
  agent on a different host) go through the `Orchestrator` trait, which is the same routed call
  path a client uses.

## omp integration (the only harness)

Each agent is one `omp --mode rpc-ui` process (the RPC protocol with omp's tool dialogs enabled)
driven by the official Rust client (vendored in `crates/omp-rpc`):

- **Browser:** a per-agent `--config` overlay sets `browser.cdpUrl` to the daemon's
  `/cdp/<project>` discovery endpoint. omp keeps its full browser tool (aria snapshots with
  delta refs, screenshots with change detection, console, network, emulation…) and drives the
  daemon-owned Chromium for that project — the same tabs every client live-views and can take
  over.
- **Orchestration:** host tools (`coocoo_spawn`, `coocoo_send`, `coocoo_wait`, `coocoo_agents`,
  `coocoo_projects`, `coocoo_hosts`, `coocoo_output`) are registered over RPC — no MCP server, no
  extra process — and route through the Orchestrator, so an agent on a laptop can start and
  supervise agents on any server in the fleet.
- **Attention:** omp `ask`/`confirm`/`select`/`input` dialogs become `UiRequest`s, surfaced in the
  overview's needs-you queue and answerable from any client.
- **Lifecycle:** status follows `prompt_result`/`session_settled`, not just `agent_end`. Idle
  agents hibernate (process exits, session kept) and resume with `--resume <sessionFile>` on the
  next prompt. A crashing omp process only marks its own agent as errored.

## Resource discipline

| Concern | Approach |
|---|---|
| Desktop shell | Tauri 2 + system webview; the daemon runs in the same process |
| Idle cost | No timers or polling while nobody watches: stats sample only during `stats.watch`, git refresh is driven by a `.git` watcher, Chromium starts on first use and stops when idle |
| Agents | omp processes hibernate after inactivity; timelines live on disk (JSONL), memory holds summaries |
| Event fan-out | Each event is serialized once into `Arc<str>` and shared by all connections; bounded queues; slow clients get `system.resync` instead of unbounded buffers |
| Browser | One Chromium per project, headless; screencast only while a client watches, fps-capped, JPEG passed through without re-encoding; lossy queue for frames |
| git | `git` CLI with `GIT_OPTIONAL_LOCKS=0`; no libgit2; diffs size-capped |
| UI | Svelte 5, no UI framework, lazy-loaded heavy views, tail-rendered transcripts |
