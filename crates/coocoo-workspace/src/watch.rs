//! One filesystem watcher for all projects, feeding a debounced refresh queue.
//!
//! Watched per project: git metadata (HEAD, index, refs, worktree registry) for the git refresh,
//! and — non-recursively, filtered by name — the project directory and its `.coocoo/` for the
//! actions file. Never the working tree as a whole, so idle repositories cost nothing and large
//! checkouts don't exhaust inotify watches.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use tokio::sync::Notify;

/// Coalescing window: a burst of git writes (checkout, rebase, commit) yields one refresh.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// What a watch is for, and so which refresh an event under it schedules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// Git metadata → `projects.rs` refresh (git info, icon, actions).
    Git,
    /// `.coocoo/actions.toml` → reload of the actions only.
    Actions,
}

/// A path to watch. With `only`, an event counts only for the watched path itself or its
/// direct child of that name (a non-recursive directory watch filtered to one entry).
pub(crate) struct Target {
    pub path: PathBuf,
    pub mode: RecursiveMode,
    pub only: Option<&'static str>,
}

struct Route {
    root: PathBuf,
    only: Option<&'static str>,
    id: String,
    kind: Kind,
}

impl Route {
    fn matches(&self, path: &Path) -> bool {
        match self.only {
            None => path.starts_with(&self.root),
            Some(name) => path == self.root || (path.parent() == Some(&self.root) && path.file_name() == Some(OsStr::new(name))),
        }
    }
}

/// State shared with the watcher callback thread.
struct Shared {
    /// An event a route matches schedules that project's refresh of the route's kind.
    routes: RwLock<Vec<Route>>,
    /// `(project id, kind)` → refresh deadline (bounded by twice the number of projects).
    pending: Mutex<HashMap<(String, Kind), Instant>>,
    wake: Notify,
    closed: AtomicBool,
}

impl Shared {
    fn schedule(&self, id: &str, kind: Kind) {
        self.pending.lock().entry((id.to_owned(), kind)).or_insert_with(|| Instant::now() + DEBOUNCE);
        self.wake.notify_one();
    }

    /// Wait for the next batch of due refreshes; `None` once closed.
    async fn next_due(&self) -> Option<Vec<(String, Kind)>> {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            let next = self.pending.lock().values().min().copied();
            match next {
                Some(deadline) => {
                    tokio::select! {
                        _ = self.wake.notified() => {}
                        _ = tokio::time::sleep_until(deadline.into()) => {}
                    }
                }
                None => self.wake.notified().await,
            }
            let now = Instant::now();
            let mut pending = self.pending.lock();
            let due: Vec<(String, Kind)> = pending.iter().filter(|(_, t)| **t <= now).map(|(key, _)| key.clone()).collect();
            for key in &due {
                pending.remove(key);
            }
            if !due.is_empty() {
                return Some(due);
            }
        }
    }
}

#[derive(Default)]
struct Registry {
    watcher: Option<RecommendedWatcher>,
    /// Set once watcher creation failed; refreshes then only happen on demand.
    unavailable: bool,
    /// Watched path → number of routes using it.
    refs: HashMap<PathBuf, usize>,
}

pub(crate) struct Watches {
    shared: Arc<Shared>,
    registry: Mutex<Registry>,
}

impl Watches {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                routes: RwLock::new(Vec::new()),
                pending: Mutex::new(HashMap::new()),
                wake: Notify::new(),
                closed: AtomicBool::new(false),
            }),
            registry: Mutex::new(Registry::default()),
        }
    }

    /// Queue a debounced refresh of `id`.
    pub fn schedule(&self, id: &str, kind: Kind) {
        self.shared.schedule(id, kind);
    }

    /// Start watching `targets` for project `id`; returns the paths actually watched.
    pub fn add(&self, id: &str, kind: Kind, targets: &[Target]) -> Vec<PathBuf> {
        let mut watched = Vec::new();
        let mut routes = Vec::new();
        {
            let mut reg = self.registry.lock();
            if reg.watcher.is_none() && !reg.unavailable {
                match self.create_watcher() {
                    Ok(w) => reg.watcher = Some(w),
                    Err(err) => {
                        tracing::warn!("file watcher unavailable, project status refreshes on demand only: {err}");
                        reg.unavailable = true;
                    }
                }
            }
            let reg = &mut *reg;
            let Some(watcher) = reg.watcher.as_mut() else { return watched };
            for target in targets {
                if let Some(count) = reg.refs.get_mut(&target.path) {
                    *count += 1;
                } else if let Err(err) = watcher.watch(&target.path, target.mode) {
                    tracing::debug!("cannot watch {}: {err}", target.path.display());
                    continue;
                } else {
                    reg.refs.insert(target.path.clone(), 1);
                }
                watched.push(target.path.clone());
                routes.push(Route { root: target.path.clone(), only: target.only, id: id.to_owned(), kind });
            }
        }
        // Never hold `routes` while calling into the watcher: its thread takes `routes`.
        self.shared.routes.write().extend(routes);
        watched
    }

    /// Stop watching `paths` on behalf of project `id` for `kind`.
    pub fn remove(&self, id: &str, kind: Kind, paths: &[PathBuf]) {
        self.shared.routes.write().retain(|r| !(r.id == id && r.kind == kind));
        self.shared.pending.lock().remove(&(id.to_owned(), kind));
        let mut reg = self.registry.lock();
        let reg = &mut *reg;
        for path in paths {
            let Some(count) = reg.refs.get_mut(path) else { continue };
            *count -= 1;
            if *count == 0 {
                reg.refs.remove(path);
                if let Some(watcher) = reg.watcher.as_mut() {
                    let _ = watcher.unwatch(path);
                }
            }
        }
        if reg.refs.is_empty() {
            // Release the inotify instance and its thread when nothing is watched.
            reg.watcher = None;
        }
    }

    fn create_watcher(&self) -> notify::Result<RecommendedWatcher> {
        let shared = Arc::clone(&self.shared);
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let mut hits: Vec<(String, Kind)> = Vec::new();
            {
                let routes = shared.routes.read();
                for path in &event.paths {
                    // Lock files come and go even for read-only git commands.
                    if path.extension().is_some_and(|ext| ext == "lock") {
                        continue;
                    }
                    for route in routes.iter() {
                        if route.matches(path) && !hits.iter().any(|(id, kind)| *id == route.id && *kind == route.kind) {
                            hits.push((route.id.clone(), route.kind));
                        }
                    }
                }
            }
            for (id, kind) in hits {
                shared.schedule(&id, kind);
            }
        })
    }

    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }

    /// A clone sharing the queue, for the refresh task (does not own the watcher).
    pub fn queue(&self) -> Queue {
        Queue(Arc::clone(&self.shared))
    }
}

/// Handle to the refresh queue used by the background refresh task; holding it does not keep
/// the [`Workspace`](crate::Workspace) alive.
pub(crate) struct Queue(Arc<Shared>);

impl Queue {
    pub async fn next_due(&self) -> Option<Vec<(String, Kind)>> {
        self.0.next_due().await
    }
}

/// Paths to watch for a repository with `git_dir` / `common_dir`.
pub(crate) fn targets(git_dir: &Path, common_dir: &Path) -> Vec<Target> {
    let target = |path: PathBuf, mode| Target { path, mode, only: None };
    let mut out = vec![target(git_dir.to_path_buf(), RecursiveMode::NonRecursive)];
    if common_dir != git_dir {
        out.push(target(common_dir.to_path_buf(), RecursiveMode::NonRecursive));
    }
    out.push(target(common_dir.join("refs"), RecursiveMode::Recursive));
    // Recursive: a linked worktree's HEAD, index and FETCH_HEAD live in `worktrees/<name>/`.
    let worktrees = common_dir.join("worktrees");
    if worktrees.is_dir() {
        out.push(target(worktrees, RecursiveMode::Recursive));
    }
    out
}

/// Paths to watch for `<project>/.coocoo/actions.toml`: the project directory (to see `.coocoo`
/// appear) and `.coocoo` itself once it exists, each filtered to that one entry.
pub(crate) fn action_targets(project: &Path) -> Vec<Target> {
    let mut out = vec![Target { path: project.to_path_buf(), mode: RecursiveMode::NonRecursive, only: Some(crate::actions::DIR) }];
    let dir = project.join(crate::actions::DIR);
    if dir.is_dir() {
        out.push(Target { path: dir, mode: RecursiveMode::NonRecursive, only: Some(crate::actions::FILE) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_routes_match_only_their_entry() {
        let route = Route { root: PathBuf::from("/p"), only: Some(".coocoo"), id: "x".into(), kind: Kind::Actions };
        assert!(route.matches(Path::new("/p/.coocoo")));
        assert!(route.matches(Path::new("/p")));
        assert!(!route.matches(Path::new("/p/Cargo.toml")));
        assert!(!route.matches(Path::new("/p/.coocoo/other")));
        let git = Route { root: PathBuf::from("/p/.git/refs"), only: None, id: "x".into(), kind: Kind::Git };
        assert!(git.matches(Path::new("/p/.git/refs/heads/main")));
    }
}
