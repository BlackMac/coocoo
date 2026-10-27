//! Workload cgroups under a delegated systemd unit (Linux).
//!
//! systemd-oomd picks its victims among leaf cgroups. When every agent, terminal and Chromium
//! shares the daemon's cgroup, one memory-hungry build gets the daemon and every other agent
//! killed with it. If the unit delegates its cgroup (`Delegate=yes`), [`init`] moves the daemon
//! into `<unit>/daemon`, which oomd skips (`user.oomd_omit`), and each heavy workload runs in its
//! own leaf ([`leaf`]), so oomd kills only the workload that eats the memory. Short-lived helpers
//! (`git`, `gh`, board commands) stay with the daemon. Without delegation (no systemd, macOS, the
//! desktop app) nothing changes.

use std::ffi::CString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The daemon's own leaf.
const DAEMON: &str = "daemon";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Set up workload cgroups once at daemon start, before any child process is spawned. Returns
/// the unit's cgroup when they are in use, `None` when the daemon does not run in a delegated
/// systemd unit.
pub fn init() -> io::Result<Option<&'static Path>> {
    #[cfg(target_os = "linux")]
    if let Some(root) = linux::setup()? {
        return Ok(Some(ROOT.get_or_init(|| root)));
    }
    Ok(None)
}

/// A workload cgroup (`<unit>/<name>`); removed when dropped, unless processes still live in it
/// (the next daemon start sweeps it then).
#[derive(Debug)]
pub struct Leaf {
    path: PathBuf,
    procs: CString,
}

/// The leaf `name` (created if missing), or `None` when workload cgroups are not in use.
pub fn leaf(name: &str) -> Option<Leaf> {
    let path = ROOT.get()?.join(name);
    match std::fs::create_dir(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let procs = CString::new(path.join("cgroup.procs").into_os_string().into_encoded_bytes()).ok()?;
    Some(Leaf { path, procs })
}

impl Leaf {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A `pre_exec` hook that moves the child into this cgroup before it runs, so everything it
    /// starts lands there too. Async-signal-safe (open/write/close, no allocation). Best effort:
    /// a failed move leaves the child with the daemon rather than failing the spawn.
    #[cfg(unix)]
    pub fn joiner(&self) -> impl FnMut() -> io::Result<()> + Send + Sync + 'static {
        let procs = self.procs.clone();
        move || {
            // SAFETY: plain syscalls on a NUL-terminated path owned by the closure.
            unsafe {
                let fd = libc::open(procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                if fd >= 0 {
                    // "0": the writing process itself.
                    libc::write(fd, b"0".as_ptr().cast(), 1);
                    libc::close(fd);
                }
            }
            Ok(())
        }
    }
}

impl Drop for Leaf {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::DAEMON;

    const CGROUP_FS: &str = "/sys/fs/cgroup";

    pub(super) fn setup() -> io::Result<Option<PathBuf>> {
        // Not started by systemd.
        if std::env::var_os("INVOCATION_ID").is_none() {
            return Ok(None);
        }
        let Some(rel) = own_cgroup()? else { return Ok(None) };
        let Some(unit) = rel.rsplit('/').next().filter(|u| u.ends_with(".service")) else { return Ok(None) };
        if !delegated(unit, rel.contains("/user@")) {
            return Ok(None);
        }
        let root = Path::new(CGROUP_FS).join(rel.trim_start_matches('/'));
        let daemon = root.join(DAEMON);
        match std::fs::create_dir(&daemon) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        std::fs::write(daemon.join("cgroup.procs"), std::process::id().to_string())?;
        // oomd ranks leaves by their memory accounting, so they need the controllers.
        let available = std::fs::read_to_string(root.join("cgroup.controllers"))?;
        let enable: Vec<String> =
            ["memory", "pids"].into_iter().filter(|c| available.split_whitespace().any(|a| a == *c)).map(|c| format!("+{c}")).collect();
        if !enable.is_empty() {
            std::fs::write(root.join("cgroup.subtree_control"), enable.join(" "))?;
        }
        // Leaves of an earlier run of this daemon whose processes are gone.
        for entry in std::fs::read_dir(&root)?.flatten() {
            if entry.file_name() != DAEMON && entry.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = std::fs::remove_dir(entry.path());
            }
        }
        // Last: if anything above failed, the children share a leaf oomd may still kill.
        set_xattr(&daemon, "user.oomd_omit", b"1")?;
        Ok(Some(root))
    }

    /// The cgroup v2 path of this process (`0::<path>` in `/proc/self/cgroup`).
    fn own_cgroup() -> io::Result<Option<String>> {
        let text = std::fs::read_to_string("/proc/self/cgroup")?;
        Ok(text.lines().find_map(|l| l.strip_prefix("0::")).map(str::to_owned))
    }

    /// Whether systemd delegates the unit's cgroup to it (`Delegate=yes`); otherwise systemd
    /// owns the tree and sub-cgroups must not be created.
    fn delegated(unit: &str, user: bool) -> bool {
        let mut cmd = Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        cmd.args(["show", "--property=Delegate", "--value", unit]).stdin(Stdio::null()).stderr(Stdio::null());
        cmd.output().is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"yes")
    }

    fn set_xattr(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let name = CString::new(name).map_err(io::Error::other)?;
        // SAFETY: NUL-terminated strings and a valid buffer for the duration of the call.
        let ret = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
        if ret == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}
