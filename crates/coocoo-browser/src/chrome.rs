//! Locating, launching and stopping Chromium.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use coocoo_proto::cgroup::Leaf;
use coocoo_proto::{ApiError, ApiResult};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

/// Executable names searched on `PATH`, in order.
const PATH_NAMES: &[&str] =
    &["google-chrome-stable", "google-chrome", "chromium", "chromium-browser", "microsoft-edge", "brave-browser"];

const MAC_APPS: &[&str] = &[
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Brave Browser.app/Contents/MacOS/Brave Browser",
];

const WINDOWS_APPS: &[&str] = &[
    "Google\\Chrome\\Application\\chrome.exe",
    "Chromium\\Application\\chrome.exe",
    "Microsoft\\Edge\\Application\\msedge.exe",
    "BraveSoftware\\Brave-Browser\\Application\\brave.exe",
];

/// Inputs to Chrome discovery, separated from the process environment for testing.
pub(crate) struct Discovery {
    pub configured: Option<String>,
    pub env_override: Option<OsString>,
    pub path_var: Option<OsString>,
    pub home: Option<PathBuf>,
    /// `(ProgramFiles, ProgramFiles(x86), LocalAppData)` roots on Windows.
    pub windows_roots: Vec<PathBuf>,
}

impl Discovery {
    pub fn from_env(configured: Option<String>) -> Self {
        let windows_roots = ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
            .iter()
            .filter_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
            .collect();
        Self {
            configured: configured.filter(|s| !s.trim().is_empty()),
            env_override: std::env::var_os("COOCOO_CHROME").filter(|v| !v.is_empty()),
            path_var: std::env::var_os("PATH"),
            home: home_dir(),
            windows_roots,
        }
    }

    /// Explicit settings (`chrome_path`, then `COOCOO_CHROME`) are authoritative: when set but
    /// unusable, discovery fails instead of silently picking another browser.
    pub fn find(&self) -> ApiResult<PathBuf> {
        for (origin, value) in [
            ("browser.chromePath", self.configured.as_ref().map(OsString::from)),
            ("COOCOO_CHROME", self.env_override.clone()),
        ] {
            if let Some(value) = value {
                return self.resolve_explicit(&value).ok_or_else(|| {
                    ApiError::unavailable(format!(
                        "{origin} = {} is not an executable Chromium-family browser",
                        Path::new(&value).display()
                    ))
                });
            }
        }
        self.auto().ok_or_else(|| {
            ApiError::unavailable(
                "no Chromium-family browser found: install Google Chrome or Chromium, or set \
                 COOCOO_CHROME / [browser] chromePath",
            )
        })
    }

    fn resolve_explicit(&self, value: &OsString) -> Option<PathBuf> {
        let path = PathBuf::from(value);
        if path.components().count() > 1 || path.is_absolute() {
            return is_executable(&path).then_some(path);
        }
        self.on_path(&value.to_string_lossy())
    }

    fn on_path(&self, name: &str) -> Option<PathBuf> {
        let path_var = self.path_var.as_ref()?;
        let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        std::env::split_paths(path_var).map(|dir| dir.join(&file)).find(|p| is_executable(p))
    }

    fn auto(&self) -> Option<PathBuf> {
        if let Some(found) = PATH_NAMES.iter().find_map(|name| self.on_path(name)) {
            return Some(found);
        }
        if let Some(home) = &self.home
            && let Some(found) = playwright_chromium(&home.join(".cache/ms-playwright"))
        {
            return Some(found);
        }
        let mut mac_roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = &self.home {
            mac_roots.push(home.join("Applications"));
        }
        let mac = mac_roots.iter().flat_map(|root| MAC_APPS.iter().map(move |app| root.join(app)));
        let win = self.windows_roots.iter().flat_map(|root| WINDOWS_APPS.iter().map(move |app| root.join(app)));
        mac.chain(win).find(|p| is_executable(p))
    }
}

/// Newest `chromium-<rev>/chrome-linux*/chrome` in a Playwright browser cache.
fn playwright_chromium(cache: &Path) -> Option<PathBuf> {
    let mut revisions: Vec<(u64, PathBuf)> = std::fs::read_dir(cache)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let rev = name.strip_prefix("chromium-")?.parse().ok()?;
            Some((rev, e.path()))
        })
        .collect();
    revisions.sort_by_key(|r| std::cmp::Reverse(r.0));
    revisions.into_iter().find_map(|(_, dir)| {
        let mut builds: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("chrome-linux"))
            .map(|e| e.path().join("chrome"))
            .collect();
        builds.sort();
        builds.into_iter().find(|p| is_executable(p))
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Command-line flags for a profile's Chromium.
pub(crate) fn launch_args(user_data_dir: &Path, headless: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--remote-debugging-port=0",
        "--remote-debugging-address=127.0.0.1",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-sync",
        "--disable-component-update",
        "--disable-features=Translate,MediaRouter,OptimizationHints",
        "--mute-audio",
        "--password-store=basic",
        "--window-size=1280,800",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let mut dir = OsString::from("--user-data-dir=");
    dir.push(user_data_dir);
    args.insert(2, dir);
    if headless {
        args.push("--headless=new".into());
    }
    // Chromium refuses to start its sandbox as root (containers); it cannot work there anyway.
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 {
        args.push("--no-sandbox".into());
    }
    args.push("about:blank".into());
    args
}

/// A launched Chromium whose DevTools endpoint is up.
pub(crate) struct Launched {
    pub child: Child,
    pub pid: u32,
    pub port: u16,
    /// Browser-level DevTools WebSocket URL.
    pub ws_url: String,
    /// Its workload cgroup; keep it until the process exited.
    pub cgroup: Option<Leaf>,
}

const PORT_FILE: &str = "DevToolsActivePort";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Launch Chromium in its own process group and wait for `DevToolsActivePort`.
pub(crate) async fn launch(exe: &Path, user_data_dir: &Path, headless: bool) -> ApiResult<Launched> {
    std::fs::create_dir_all(user_data_dir)?;
    let port_file = user_data_dir.join(PORT_FILE);
    match std::fs::remove_file(&port_file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    kill_orphan(user_data_dir);

    let mut cmd = Command::new(exe);
    cmd.args(launch_args(user_data_dir, headless))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    // Own cgroup: oomd can kill a bloated Chromium without the daemon.
    let name = user_data_dir.file_name().map_or_else(|| "default".into(), |n| n.to_string_lossy());
    let cgroup = coocoo_proto::cgroup::leaf(&format!("browser-{name}"));
    #[cfg(unix)]
    if let Some(leaf) = &cgroup {
        // SAFETY: the hook only makes async-signal-safe syscalls.
        unsafe { cmd.pre_exec(leaf.joiner()) };
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ApiError::unavailable(format!("failed to start {}: {e}", exe.display())))?;
    let pid = child.id().unwrap_or(0);

    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&port_file)
            && let Some((port, path)) = parse_port_file(&text)
        {
            return Ok(Launched { child, pid, port, ws_url: format!("ws://127.0.0.1:{port}{path}"), cgroup });
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ApiError::unavailable(format!("{} exited during startup ({status})", exe.display())));
        }
        if tokio::time::Instant::now() >= deadline {
            kill_group(pid, true);
            let _ = child.wait().await;
            return Err(ApiError::unavailable(format!(
                "{} did not open its DevTools port within {}s",
                exe.display(),
                STARTUP_TIMEOUT.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `DevToolsActivePort` = `<port>\n<browser ws path>`; the file can be observed half-written.
fn parse_port_file(text: &str) -> Option<(u16, &str)> {
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse().ok().filter(|p| *p != 0)?;
    let path = lines.next()?.trim();
    // Require the whole browser GUID; a shorter id means the file is still being written.
    let id = path.strip_prefix("/devtools/browser/")?;
    (id.len() >= 32).then_some((port, path))
}

/// A previous daemon that died without cleanup may have left Chromium running on this profile;
/// a new launch would then just hand its URL to that instance and exit. Kill such orphans.
fn kill_orphan(user_data_dir: &Path) {
    #[cfg(target_os = "linux")]
    {
        let Ok(target) = std::fs::read_link(user_data_dir.join("SingletonLock")) else { return };
        let Some(pid) = target.to_string_lossy().rsplit_once('-').and_then(|(_, p)| p.parse::<i32>().ok()) else {
            return;
        };
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { return };
        let mut flag = b"--user-data-dir=".to_vec();
        flag.extend_from_slice(user_data_dir.as_os_str().as_encoded_bytes());
        if cmdline.split(|b| *b == 0).any(|arg| arg == flag.as_slice()) {
            tracing::warn!(pid, dir = %user_data_dir.display(), "killing orphaned Chromium");
            kill_group(pid as u32, true);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = user_data_dir;
}

/// Signal Chromium's whole process group (renderers, GPU, zygotes).
pub(crate) fn kill_group(pid: u32, force: bool) {
    #[cfg(unix)]
    if pid != 0 {
        let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: plain syscalls on a pid/pgid we spawned; failures (ESRCH) are harmless.
        unsafe {
            if libc::killpg(pid as libc::pid_t, sig) != 0 {
                libc::kill(pid as libc::pid_t, sig);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}

/// Own the child until it exits. A kill request sends SIGTERM to the group, escalating to
/// SIGKILL after a grace period. Helpers left in the group are swept after the main process exits.
pub(crate) async fn supervise(mut child: Child, pid: u32, kill_rx: oneshot::Receiver<()>) -> Option<ExitStatus> {
    let status = tokio::select! {
        status = child.wait() => status.ok(),
        Ok(()) = kill_rx => {
            kill_group(pid, false);
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(status) => status.ok(),
                Err(_) => {
                    kill_group(pid, true);
                    #[cfg(not(unix))]
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
            }
        }
    };
    kill_group(pid, true);
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("coocoo-browser-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn exe(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn discovery(s: &Scratch) -> Discovery {
        Discovery {
            configured: None,
            env_override: None,
            path_var: Some(std::env::join_paths([s.0.join("bin1"), s.0.join("bin2")]).unwrap()),
            home: Some(s.0.join("home")),
            windows_roots: vec![],
        }
    }

    #[cfg(unix)]
    #[test]
    fn discovery_order() {
        let s = Scratch::new("order");
        let mut d = discovery(&s);
        assert_eq!(d.find().unwrap_err().code, "unavailable");

        let pw_old = s.exe("home/.cache/ms-playwright/chromium-900/chrome-linux/chrome");
        let pw_new = s.exe("home/.cache/ms-playwright/chromium-1200/chrome-linux64/chrome");
        s.exe("home/.cache/ms-playwright/chromium_headless_shell-1300/chrome-linux/chrome");
        assert_eq!(d.find().unwrap(), pw_new);
        std::fs::remove_file(&pw_new).unwrap();
        assert_eq!(d.find().unwrap(), pw_old);

        // PATH beats the Playwright cache; earlier names beat later ones regardless of dir order.
        let brave = s.exe("bin1/brave-browser");
        assert_eq!(d.find().unwrap(), brave);
        let chromium = s.exe("bin2/chromium");
        assert_eq!(d.find().unwrap(), chromium);
        let stable = s.exe("bin2/google-chrome-stable");
        assert_eq!(d.find().unwrap(), stable);

        // COOCOO_CHROME beats PATH, config beats COOCOO_CHROME; bare names resolve on PATH.
        let custom = s.exe("custom/chrome");
        d.env_override = Some(custom.clone().into());
        assert_eq!(d.find().unwrap(), custom);
        d.configured = Some("chromium".into());
        assert_eq!(d.find().unwrap(), chromium);

        // Explicit but broken settings fail loudly.
        d.configured = Some(s.0.join("missing").to_string_lossy().into_owned());
        let err = d.find().unwrap_err();
        assert_eq!(err.code, "unavailable");
        assert!(err.message.contains("browser.chromePath"), "{}", err.message);
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_is_skipped() {
        let s = Scratch::new("noexec");
        let d = discovery(&s);
        let path = s.0.join("bin1/google-chrome");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        assert!(d.find().is_err());
    }

    #[test]
    fn port_file_parsing() {
        let full = "/devtools/browser/0b9e1c3f-5d1a-4c55-9a39-2f7d0c4b8e11";
        assert_eq!(parse_port_file(&format!("4567\n{full}\n")), Some((4567, full)));
        assert_eq!(parse_port_file("4567\n/devtools/browser/0b9e1c3f"), None);
        assert_eq!(parse_port_file("4567\n"), None);
        assert_eq!(parse_port_file("45"), None);
        assert_eq!(parse_port_file(&format!("0\n{full}")), None);
    }

    #[test]
    fn args_include_profile_and_headless() {
        let args = launch_args(Path::new("/tmp/p"), true);
        let args: Vec<_> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--user-data-dir=/tmp/p".to_owned()));
        assert!(args.contains(&"--headless=new".to_owned()));
        assert_eq!(args.last().map(String::as_str), Some("about:blank"));
        assert!(!launch_args(Path::new("/tmp/p"), false).iter().any(|a| a == "--headless=new"));
    }
}
