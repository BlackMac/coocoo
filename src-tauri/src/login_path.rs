//! `PATH` of the user's login shell for the in-process daemon.
//!
//! Started from Finder, the Dock or a desktop launcher, the app inherits launchd's or the
//! session's minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin` on macOS): no Homebrew, no
//! `~/.local/bin`. The daemon would then not find `gh`, `omp` or the tools agents call, although
//! a terminal does. The login shell's `PATH` is what a terminal has.

use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A profile that waits for input or does network work must not hold up the window for long.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Brackets the value: profile scripts may print banners or other noise to stdout.
const MARK: &str = "__COOCOO_PATH__";

/// Prepend the login shell's `PATH` entries to the process's own (entries already there keep
/// their place after them).
///
/// # Safety
///
/// Changes the process environment: call before any other thread exists.
pub(crate) unsafe fn import() {
    let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
    let login = match login_path(&shell) {
        Ok(login) => login,
        Err(err) => {
            tracing::warn!("PATH of the login shell {}: {err}", shell.to_string_lossy());
            return;
        }
    };
    let current = std::env::var_os("PATH").unwrap_or_default();
    match merge(&login, &current) {
        // SAFETY: the caller guarantees that no other thread reads the environment.
        Ok(path) => unsafe { std::env::set_var("PATH", path) },
        Err(err) => tracing::warn!("cannot use the login shell's PATH: {err}"),
    }
}

fn login_path(shell: &OsStr) -> Result<OsString, String> {
    let mut child = Command::new(shell)
        .args(["-l", "-c", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Its own group, so a timeout also stops what the profile started.
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    // Non-blocking: a background job the profile starts may hold the pipe open after the shell
    // exited, so reading up to EOF could wait forever.
    // SAFETY: plain fcntl calls on a descriptor `stdout` owns.
    unsafe {
        let fd = stdout.as_raw_fd();
        libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + TIMEOUT;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let exited = child.try_wait().map_err(|e| e.to_string())?.is_some();
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if exited {
            break;
        }
        if Instant::now() >= deadline {
            // SAFETY: signals the process group created for the shell above.
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
            let _ = child.wait();
            return Err(format!("no answer within {} s", TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = String::from_utf8_lossy(&out);
    let value = out.split_once(MARK).and_then(|(_, rest)| rest.split_once(MARK)).map(|(value, _)| value);
    value.filter(|v| !v.is_empty()).map(OsString::from).ok_or_else(|| "no PATH in its output".to_owned())
}

/// `login`'s entries, then those of `current` that `login` lacks; duplicates and empty entries
/// dropped.
fn merge(login: &OsStr, current: &OsStr) -> Result<OsString, std::env::JoinPathsError> {
    let mut entries = Vec::new();
    for entry in std::env::split_paths(login).chain(std::env::split_paths(current)) {
        if !entry.as_os_str().is_empty() && !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    std::env::join_paths(entries)
}
