# Problem: the coocoo daemon disappears (systemd-oomd kills the user session)

The root cause is a systemd 255 bug in `systemd-oomd` (see "Why oomd kills `init.scope`").
A user-level mitigation is installed on this host (see "Mitigation").

## Symptom

`coocoo serve` (the daemon on `:7421`) is suddenly gone. The daemon's own log has no
error, panic or shutdown message. The last lines before it vanished are routine
(`stopping idle Chromium …`). After the kill it stays down until the user logs in again.

## What actually happens

The daemon does not crash. The whole systemd user manager is SIGKILLed, and the daemon dies with it.

1. On this host coocoo runs as a **systemd user unit**: `~/.config/systemd/user/coocoo.service`
   (`Restart=always`, `KillMode=mixed`). It lives under
   `/user.slice/user-1000.slice/user@1000.service/app.slice/coocoo.service`.
2. Every process the daemon starts stays in that same cgroup: `omp --mode rpc` agents
   (`coocoo-agents/src/api.rs`), terminal shells (`coocoo-workspace/src/terminal.rs`),
   Chromium (`coocoo-browser/src/chrome.rs`), `sh -c` board commands, `git`, `gh`, `curl`, …
   `setsid()` in the terminal code creates a new session/process group. It does **not** move
   the process to another cgroup. So everything the user runs inside a coocoo terminal or agent
   is counted as memory of `coocoo.service`, including `cargo build`, `vite`, test runs and the
   `coocoo-desktop` debug build.
3. `systemd-oomd` watches memory pressure on `user@1000.service`. Its rule: pressure above
   **50% for more than 20s, with reclaim activity**, triggers a kill.
4. When that happens, oomd lists `coocoo.service` as the top candidate, but the cgroup it
   reports as killed is `user@1000.service/init.scope`. That cgroup holds the **user manager
   process itself (`systemd --user`)**. In fact oomd SIGKILLs *every* candidate in its list down
   to `init.scope`, `coocoo.service` first (see below).
5. With the user manager dead, `user@1000.service` fails with `result 'signal'`, and systemd
   SIGKILLs every remaining user unit: coocoo, its agents and terminals, Chrome, pipewire,
   the hermes gateways, and so on.
   - `Restart=always` on coocoo.service does nothing here, because the manager that would
     restart it is the thing that died.
   - `KillMode=mixed` and the graceful "pause agents, then stop" path never run. Everything gets SIGKILL.
6. Linger is off (`loginctl show-user … Linger=no`), so the user manager, and coocoo with it,
   only come back on the next login (SSH or desktop).

## Observed incidents

| time | pressure on user@1000.service | `coocoo.service` memory at kill | user@1000 peak |
|---|---|---|---|
| 2026-10-24 22:20:43 | 75.58% | 3.5G | 10.4G |
| 2026-10-25 00:20:23 | 71.95% (coocoo.service Avg10 73.02) | 8.4G | 9.2G |

At both kills every other user cgroup was under ~250M. The coocoo daemon process itself is
small: about 12–30M right after a restart. The gigabytes come from its child processes.

Before the 00:20 kill, a fresh `target/debug/coocoo-desktop` had been built (binary mtime
00:18) and launched at 00:18:28 (pid 1147143), both by a coocoo agent, so they were charged to
`coocoo.service` (see "Who used the memory").

The oomd journal shows the same `init.scope` kill on 2026-10-17 (twice) and on 2026-10-18,
plus kills of the user `dbus`/`pipewire` units. Whether coocoo was the main consumer in those
earlier cases was not checked.

## Host context

- 16G RAM, 12G swap, about 6G of swap already in use from a large set of Docker containers outside
  the user slice. That leaves little headroom, so a single Rust debug build, or several agents
  plus a build, can cross the pressure threshold.

## Why oomd kills `init.scope`

systemd 255 bug, fixed upstream in v256 by
[systemd#33084](https://github.com/systemd/systemd/pull/33084) ("cgroup-util: Don't try to open
pidfd for pids from cgroup.threads"); also reported as
[systemd#32304](https://github.com/systemd/systemd/issues/32304). Ubuntu's `255.4-1ubuntu8.18`
does not carry the fix.

1. `oomd_kill_by_pgscan_rate()` (`src/oom/oomd-util.c`) sorts the candidates and calls
   `oomd_cgroup_kill()` on them in order. A negative return means "failed, try the next one";
   the loop stops at the first candidate that returns `>= 0`.
2. `oomd_cgroup_kill()` → `cg_kill()` SIGKILLs every pid in `cgroup.procs`, then walks
   `cgroup.threads` and calls `pidfd_open()` on each thread ID. For a non-leader thread the
   kernel returns `EINVAL`. So for any cgroup with a multi-threaded process (the daemon has ~24
   threads, `omp`/bun, rustc, Chromium, pipewire, the hermes gateways, …) the kill *does*
   happen but is reported as failed.
3. oomd moves on and kills the next candidate, and the next, until it reaches one with only
   single-threaded processes. In a typical user session that is `init.scope`
   (`systemd --user`). Only that last one appears in the `Killed …` line.

Evidence in the journal:

- The candidate dump is `DUMP_ON_KILL_COUNT` = 10 entries, extended to the index of the
  "successful" kill. At 2026-10-25 00:20:23 it lists 12 entries, ending with `init.scope`: the
  11 candidates before it all returned an error.
- PID 1 hits the same bug while tearing down `user@1000.service`:
  `Failed to kill control group /user.slice/user-1000.slice/user@1000.service, ignoring: Invalid argument`.
- 2026-10-17 00:36:52: oomd logs `Killed …/filter-chain.service`, and within 6 ms the user
  manager reports `systemd-oomd killed some process(es)` for ten more units (dbus, pipewire,
  wireplumber, runpany, yuzudash, …).

## Who used the memory

Per-process memory at the kill is not recorded anywhere. What was running (from the omp
session logs, times local):

- 2026-10-25 00:20: two coocoo agents in parallel. One ran a debug `cargo build -p coocoo-desktop`
  (00:12:51–00:18:11), then started the desktop app (WebKit) at 00:18:28. The other ran
  `cargo test -p coocoo-workspace` (00:15:42–00:17:38), `cargo clippy --workspace --all-targets`
  (00:17:40–00:18:37, includes the desktop crate) and a second `cargo clippy` that was still
  running at the kill. Plus both `omp` processes and their MCP servers.
- 2026-10-24 22:20: no cargo build. The VoltPlan agent had fanned out parallel `omp`
  subagents at 22:14:37, plus `node`/`vitest` and a local PocketBase.

So it is the workload started by agents (Rust builds/clippy, parallel subagents, the desktop
app), not a leak in the daemon process.

## Mitigation

Installed on this host (user level, no root needed):

```ini
# ~/.config/systemd/user/init.scope.d/10-oomd-omit.conf
[Scope]
ManagedOOMPreference=omit
```

After `systemctl --user daemon-reload` the user manager sets `user.oomd_omit=1` on
`user@1000.service/init.scope`. oomd honours it there because that cgroup and
`user@1000.service` have the same owner (`oomd_fetch_cgroup_oom_preference()`), and drops
omitted cgroups from the candidate list (`oomd_sort_cgroup_contexts()`). The user manager then
survives a pressure kill, so `Restart=always` brings coocoo back after 3 s.

This does not stop the cascade: oomd still SIGKILLs every multi-threaded candidate in the list
(coocoo, hermes, pipewire, …). Fixes that need root:

- Stop oomd from acting on user sessions:
  `/etc/systemd/system/user@.service.d/99-oomd.conf` with `[Service]` `ManagedOOMMemoryPressure=auto`.
  The kernel OOM killer stays as the backstop and kills a single process.
- Or raise the threshold (`ManagedOOMMemoryPressureLimit=80%`) to make kills rarer. Every kill
  still cascades.
- Or a systemd ≥ 256 with #33084.

## How to see it

```sh
# kills and the per-cgroup memory dump just before each kill
journalctl -u systemd-oomd --since today --no-pager | grep -E 'Killed|Path:|Current Memory Usage'
# user manager being torn down
journalctl --since today --no-pager | grep 'user@1000.service: \(Main\|Failed\|Consumed\)'
# live memory of the daemon's cgroup (includes all children)
systemctl --user status coocoo.service
systemd-cgls --user-unit coocoo.service
```
