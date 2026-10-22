//! Filesystem sandbox for importer scripts. On Linux ≥ 5.13 it uses Landlock, which needs no
//! privileges (unlike bubblewrap, which needs unprivileged user namespaces that distributions
//! increasingly block): the process may read and run the system (`/usr`, `/etc`, …), read the
//! paths in `read`, and change only the paths in `write`. The rest of the filesystem, including
//! the home directory, is off limits. Network access stays open (importers fetch feeds and
//! changelogs). Elsewhere [`confine`] reports why it could not sandbox.

use std::path::Path;

/// Read-only system locations every process needs (missing ones are skipped).
#[cfg(target_os = "linux")]
const SYSTEM: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/proc", "/sys", "/run", "/nix", "/snap"];

/// Restrict `command` (from exec on) to reading the system and `read`, and changing `write`.
/// `Err` = not sandboxed, with the reason.
#[cfg(target_os = "linux")]
pub(crate) fn confine(command: &mut tokio::process::Command, read: &[&Path], write: &[&Path]) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let system: Vec<&Path> = SYSTEM.iter().map(Path::new).chain(read.iter().copied()).collect();
    // `/dev/null` and friends are written to.
    let write: Vec<&Path> = write.iter().copied().chain([Path::new("/dev")]).collect();
    let ruleset = landlock::ruleset(&system, &write)?;
    // SAFETY: the closure runs between fork and exec and only makes two syscalls on an fd the
    // parent opened (no allocation, no locks).
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0u32) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn confine(_command: &mut tokio::process::Command, _read: &[&Path], _write: &[&Path]) -> Result<(), String> {
    Err("sandboxing needs Linux (Landlock)".into())
}

#[cfg(target_os = "linux")]
mod landlock {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    const EXECUTE: u64 = 1 << 0;
    const WRITE_FILE: u64 = 1 << 1;
    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const TRUNCATE: u64 = 1 << 14;
    const IOCTL_DEV: u64 = 1 << 15;
    /// Rights that make sense on a file (directory rights on a file rule are rejected).
    const FILE_RIGHTS: u64 = EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV;
    const READ: u64 = EXECUTE | READ_FILE | READ_DIR;
    const CREATE_RULESET_VERSION: u32 = 1;
    const RULE_PATH_BENEATH: u32 = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C, packed)]
    struct PathBeneath {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// Every filesystem right the kernel's Landlock ABI knows: all of them are denied unless a
    /// rule grants them.
    fn handled(abi: i64) -> u64 {
        let mut all = (1 << 13) - 1; // ABI 1: execute … make_sym
        if abi >= 2 {
            all |= 1 << 13; // refer
        }
        if abi >= 3 {
            all |= TRUNCATE;
        }
        if abi >= 5 {
            all |= IOCTL_DEV;
        }
        all
    }

    fn os_error(what: &str) -> String {
        format!("{what}: {}", std::io::Error::last_os_error())
    }

    pub(super) fn ruleset(read: &[&Path], write: &[&Path]) -> Result<OwnedFd, String> {
        // SAFETY: the version query takes no pointer.
        let abi = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<RulesetAttr>(), 0usize, CREATE_RULESET_VERSION) };
        if abi < 1 {
            return Err(os_error("Landlock is not available"));
        }
        let handled = handled(abi);
        let attr = RulesetAttr { handled_access_fs: handled };
        // SAFETY: `attr` outlives the call and its size is passed along.
        let fd = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr as *const RulesetAttr, size_of::<RulesetAttr>(), 0u32) };
        if fd < 0 {
            return Err(os_error("cannot create a Landlock ruleset"));
        }
        // SAFETY: a fresh fd owned by nobody else (the kernel sets O_CLOEXEC on it).
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        for (paths, access) in [(read, READ), (write, handled)] {
            for path in paths {
                let Ok(file) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_PATH | libc::O_CLOEXEC).open(path) else { continue };
                let is_dir = file.metadata().is_ok_and(|m| m.is_dir());
                let rule = PathBeneath { allowed_access: access & handled & if is_dir { u64::MAX } else { FILE_RIGHTS }, parent_fd: file.as_raw_fd() };
                // SAFETY: `rule` and both fds outlive the call.
                let rc = unsafe { libc::syscall(libc::SYS_landlock_add_rule, ruleset.as_raw_fd(), RULE_PATH_BENEATH, &rule as *const PathBeneath, 0u32) };
                if rc != 0 {
                    return Err(os_error(&format!("cannot allow {}", path.display())));
                }
            }
        }
        Ok(ruleset)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn confined_process_reads_system_and_writes_only_its_directory() {
        let root = std::env::temp_dir().join(format!("coocoo-sandbox-{}", std::process::id()));
        let (state, project) = (root.join("state"), root.join("project"));
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(root.join("secret"), "secret").unwrap();
        std::fs::write(project.join("README"), "readme\n").unwrap();
        let script = format!(
            "cat /etc/passwd >/dev/null && echo system; cat {p}/README; echo hi >{s}/out && echo wrote; cat {r}/secret || echo no-secret; echo x >{p}/new || echo no-project-write",
            p = project.display(),
            s = state.display(),
            r = root.display()
        );
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", &script]);
        if let Err(reason) = confine(&mut command, &[&project], &[&state]) {
            eprintln!("skipped: {reason}");
            return;
        }
        let out = command.output().await.unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(stdout.split_whitespace().collect::<Vec<_>>(), ["system", "readme", "wrote", "no-secret", "no-project-write"], "{stdout}");
    }
}
