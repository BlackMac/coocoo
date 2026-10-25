//! SSH transport for peers: `ssh -L` tunnels, remote token lookup and one-shot install.
//!
//! Everything runs through the system `ssh`/`scp` binaries in `BatchMode`, so the user's
//! keys, agent, `~/.ssh/config` aliases and jump hosts all just work and no password prompt
//! can ever block the daemon.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::peer::DEFAULT_PORT;

const TUNNEL_READY_TIMEOUT: Duration = Duration::from_secs(25);
const REMOTE_CMD_TIMEOUT: Duration = Duration::from_secs(90);
/// Copying or downloading the binary.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);
/// The release install script (`curl … | sh`), run on remotes this binary can't be copied to.
const INSTALL_SCRIPT: &str = include_str!("../../../install.sh");
/// Bytes of ssh stderr kept for error messages.
const STDERR_KEEP: usize = 2048;

/// A running `ssh -N -L` port forward; the process is killed when this is dropped.
pub(crate) struct Tunnel {
    child: Child,
    /// Local loopback port forwarded to the remote daemon.
    pub port: u16,
}

impl Tunnel {
    pub async fn spawn(target: &str, ssh_port: Option<u16>, remote_port: u16) -> Result<Tunnel, String> {
        let port = free_port().map_err(|e| format!("no free local port: {e}"))?;
        let mut cmd = Command::new("ssh");
        cmd.arg("-N")
            .arg("-L")
            .arg(format!("127.0.0.1:{port}:127.0.0.1:{remote_port}"))
            .args(["-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"])
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"]);
        if let Some(p) = ssh_port {
            cmd.arg("-p").arg(p.to_string());
        }
        cmd.arg(target);
        prepare(&mut cmd);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("cannot run ssh: {e}"))?;

        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = child.stderr.take() {
            let sink = stderr.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut buf = sink.lock();
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(line.trim());
                    if buf.len() > STDERR_KEEP {
                        let mut cut = buf.len() - STDERR_KEEP;
                        while !buf.is_char_boundary(cut) {
                            cut += 1;
                        }
                        buf.drain(..cut);
                    }
                }
            });
        }

        let deadline = Instant::now() + TUNNEL_READY_TIMEOUT;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                // Let the stderr reader catch up with the last lines.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let err = last_line(&stderr.lock()).unwrap_or_else(|| status.to_string());
                return Err(format!("ssh to {target} failed: {err}"));
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return Ok(Tunnel { child, port });
            }
            if Instant::now() >= deadline {
                return Err(format!("ssh tunnel to {target} did not come up within {}s", TUNNEL_READY_TIMEOUT.as_secs()));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// `false` once the ssh process has exited.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Failure of a remote command.
#[derive(Debug)]
pub(crate) enum RemoteError {
    /// ssh itself failed (unreachable, auth refused, host key…): exit status 255.
    Ssh(String),
    /// The remote command ran and failed.
    Command(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteError::Ssh(msg) | RemoteError::Command(msg) => f.write_str(msg),
        }
    }
}

/// Run `script` with `sh -c` on the remote (independent of the remote login shell).
async fn run_remote(target: &str, ssh_port: Option<u16>, script: &str) -> Result<String, RemoteError> {
    run_remote_within(target, ssh_port, script, REMOTE_CMD_TIMEOUT).await
}

async fn run_remote_within(target: &str, ssh_port: Option<u16>, script: &str, timeout: Duration) -> Result<String, RemoteError> {
    let mut cmd = Command::new("ssh");
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"]);
    if let Some(p) = ssh_port {
        cmd.arg("-p").arg(p.to_string());
    }
    cmd.arg(target).arg(format!("sh -c {}", shell_quote(script)));
    prepare(&mut cmd);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(err)) => return Err(RemoteError::Ssh(format!("cannot run ssh: {err}"))),
        Err(_) => return Err(RemoteError::Ssh(format!("ssh to {target} timed out"))),
    };
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let detail = last_line(&stderr).unwrap_or_else(|| out.status.to_string());
    if out.status.code() == Some(255) {
        Err(RemoteError::Ssh(format!("ssh to {target} failed: {detail}")))
    } else {
        Err(RemoteError::Command(format!("on {target}: {detail}")))
    }
}

/// The remote daemon's token via `coocoo token` (creates the remote config on first use).
pub(crate) async fn fetch_token(target: &str, ssh_port: Option<u16>) -> Result<String, RemoteError> {
    let out = run_remote(target, ssh_port, "coocoo token 2>/dev/null || ~/.local/bin/coocoo token").await?;
    let token = out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or_default();
    if token.len() >= 32 && token.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(token.to_owned())
    } else {
        Err(RemoteError::Command(format!("`coocoo token` on {target} printed no token")))
    }
}

/// Install `coocoo` to `~/.local/bin/coocoo` on the remote and start it. Copies this build when
/// the remote runs the same OS/arch and the CLI binary is at hand; otherwise the remote downloads
/// this version's release build with the install script.
pub(crate) async fn install(target: &str, ssh_port: Option<u16>, remote_port: u16) -> Result<(), String> {
    let uname = run_remote(target, ssh_port, "uname -sm").await.map_err(|e| e.to_string())?;
    let local = if uname_matches(uname.trim()) { coocoo_binary() } else { None };
    match local {
        Some(exe) => copy_binary(target, ssh_port, &exe).await?,
        None => {
            let script = format!(
                "COOCOO_VERSION=v{}\nCOOCOO_INSTALL_DIR=\"$HOME/.local/bin\"\nexport COOCOO_VERSION COOCOO_INSTALL_DIR\n{INSTALL_SCRIPT}",
                env!("CARGO_PKG_VERSION")
            );
            run_remote_within(target, ssh_port, &script, TRANSFER_TIMEOUT).await.map_err(|e| e.to_string())?;
        }
    }
    run_remote(target, ssh_port, "~/.local/bin/coocoo token >/dev/null").await.map_err(|e| e.to_string())?;
    start_remote(target, ssh_port, remote_port).await
}

/// `scp` this build to `~/.local/bin/coocoo` on the remote.
async fn copy_binary(target: &str, ssh_port: Option<u16>, exe: &std::path::Path) -> Result<(), String> {
    run_remote(target, ssh_port, "mkdir -p ~/.local/bin").await.map_err(|e| e.to_string())?;
    let mut scp = Command::new("scp");
    scp.args(["-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15"]);
    if let Some(p) = ssh_port {
        scp.arg("-P").arg(p.to_string());
    }
    scp.arg(exe).arg(format!("{target}:.local/bin/coocoo.new"));
    prepare(&mut scp);
    scp.stdout(Stdio::null()).stderr(Stdio::piped());
    match tokio::time::timeout(TRANSFER_TIMEOUT, scp.output()).await {
        Ok(Ok(out)) if out.status.success() => {}
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("copying coocoo to {target} failed: {}", last_line(&stderr).unwrap_or_default()));
        }
        Ok(Err(err)) => return Err(format!("cannot run scp: {err}")),
        Err(_) => return Err(format!("copying coocoo to {target} timed out")),
    }
    run_remote(target, ssh_port, "chmod 755 ~/.local/bin/coocoo.new && mv -f ~/.local/bin/coocoo.new ~/.local/bin/coocoo")
        .await
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Start `coocoo serve` on the remote unless it already runs: a lingering systemd user unit
/// when available (survives logout and reboots), otherwise a detached `nohup` process.
pub(crate) async fn start_remote(target: &str, ssh_port: Option<u16>, remote_port: u16) -> Result<(), String> {
    let listen = if remote_port == DEFAULT_PORT { String::new() } else { format!(" --listen 127.0.0.1:{remote_port}") };
    let script = format!(
        r#"set -e
BIN="$HOME/.local/bin/coocoo"
[ -x "$BIN" ] || BIN="$(command -v coocoo || true)"
[ -n "$BIN" ] || {{ echo "coocoo is not installed (expected ~/.local/bin/coocoo)" >&2; exit 1; }}
"$BIN" status --listen 127.0.0.1:{remote_port} >/dev/null 2>&1 && {{ echo running; exit 0; }}
USER_NAME="$(id -un)"
if systemctl --user show-environment >/dev/null 2>&1; then
  loginctl enable-linger "$USER_NAME" >/dev/null 2>&1 || true
  if [ "$(loginctl show-user "$USER_NAME" -p Linger --value 2>/dev/null)" = yes ]; then
    mkdir -p "$HOME/.config/systemd/user"
    cat > "$HOME/.config/systemd/user/coocoo.service" <<EOF
[Unit]
Description=Coocoo daemon
After=network-online.target

[Service]
ExecStart=$BIN serve{listen}
Restart=on-failure
RestartSec=3
# SIGTERM to the daemon only: on a restart it lets running agents pause, then stops them itself.
KillMode=mixed

[Install]
WantedBy=default.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now coocoo.service >/dev/null 2>&1
    echo systemd
    exit 0
  fi
fi
mkdir -p "$HOME/.cache"
# Own session: immune to the ssh session's hangup and process-group signals.
DETACH=""
command -v setsid >/dev/null 2>&1 && DETACH="setsid"
$DETACH nohup "$BIN" serve{listen} >"$HOME/.cache/coocoo-daemon.log" 2>&1 </dev/null &
echo nohup
"#
    );
    let how = run_remote(target, ssh_port, &script).await.map_err(|e| format!("starting coocoo on {target}: {e}"))?;
    tracing::info!("coocoo on {target}: {}", how.trim());
    Ok(())
}

/// The `coocoo` CLI binary to copy: this executable, or a `coocoo` sidecar next to it.
fn coocoo_binary() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if exe.file_stem().is_some_and(|s| s == "coocoo") {
        return Some(exe);
    }
    let sidecar = exe.with_file_name(format!("coocoo{}", std::env::consts::EXE_SUFFIX));
    sidecar.is_file().then_some(sidecar)
}

/// Does `uname -sm` output describe the platform this binary was built for?
fn uname_matches(uname: &str) -> bool {
    let mut parts = uname.split_whitespace();
    let (Some(sys), Some(machine)) = (parts.next(), parts.next()) else { return false };
    let os_ok = match std::env::consts::OS {
        "linux" => sys == "Linux",
        "macos" => sys == "Darwin",
        "freebsd" => sys == "FreeBSD",
        _ => false,
    };
    let arch_ok = match std::env::consts::ARCH {
        "x86_64" => matches!(machine, "x86_64" | "amd64"),
        "aarch64" => matches!(machine, "aarch64" | "arm64"),
        other => machine == other,
    };
    os_ok && arch_ok
}

fn prepare(cmd: &mut Command) {
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    // Own process group: a Ctrl-C in the daemon's terminal must not kill tunnels before
    // the daemon shuts them down itself.
    #[cfg(unix)]
    cmd.process_group(0);
}

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn last_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn uname() {
        let local = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "Linux x86_64",
            ("linux", "aarch64") => "Linux aarch64",
            ("macos", "aarch64") => "Darwin arm64",
            ("macos", "x86_64") => "Darwin x86_64",
            _ => return,
        };
        assert!(uname_matches(local));
        assert!(!uname_matches("Plan9 mips"));
        assert!(!uname_matches(""));
    }
}
