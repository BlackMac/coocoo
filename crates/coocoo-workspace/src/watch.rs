//! One filesystem watcher for all projects' git directories, feeding a debounced refresh queue.
//!
//! Only git metadata is watched (HEAD, index, refs, worktree registry) — never the working
//! tree — so idle repositories cost nothing and large checkouts don't exhaust inotify watches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use tokio::sync::Notify;

/// Coalescing window: a burst of git writes (checkout, rebase, commit) yields one refresh.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// State shared with the watcher callback thread.
struct Shared {
    /// `(watched root, project id)`; an event under a root schedules that project.
    routes: RwLock<Vec<(PathBuf, String)>>,
    /// Project id → refresh deadline (bounded by the number of projects).
    pending: Mutex<HashMap<String, Instant>>,
    wake: Notify,
    closed: AtomicBool,
}

impl Shared {
    fn schedule(&self, id: &str) {
        self.pending.lock().entry(id.to_owned()).or_insert_with(|| Instant::now() + DEBOUNCE);
        self.wake.notify_one();
    }

    /// Wait for the next batch of due project ids; `None` once closed.
    async fn next_due(&self) -> Option<Vec<String>> {
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
            let due: Vec<String> = pending.iter().filter(|(_, t)| **t <= now).map(|(id, _)| id.clone()).collect();
            for id in &due {
                pending.remove(id);
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
    /// Watched path → number of projects using it.
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
    pub fn schedule(&self, id: &str) {
        self.shared.schedule(id);
    }

    /// Start watching `targets` for project `id`; returns the paths actually watched.
    pub fn add(&self, id: &str, targets: &[(PathBuf, RecursiveMode)]) -> Vec<PathBuf> {
        let mut watched = Vec::new();
        {
            let mut reg = self.registry.lock();
            if reg.watcher.is_none() && !reg.unavailable {
                match self.create_watcher() {
                    Ok(w) => reg.watcher = Some(w),
                    Err(err) => {
                        tracing::warn!("git watcher unavailable, project status refreshes on demand only: {err}");
                        reg.unavailable = true;
                    }
                }
            }
            let reg = &mut *reg;
            let Some(watcher) = reg.watcher.as_mut() else { return watched };
            for (path, mode) in targets {
                if let Some(count) = reg.refs.get_mut(path) {
                    *count += 1;
                } else if let Err(err) = watcher.watch(path, *mode) {
                    tracing::debug!("cannot watch {}: {err}", path.display());
                    continue;
                } else {
                    reg.refs.insert(path.clone(), 1);
                }
                watched.push(path.clone());
            }
        }
        // Never hold `routes` while calling into the watcher: its thread takes `routes`.
        let mut routes = self.shared.routes.write();
        routes.extend(watched.iter().map(|p| (p.clone(), id.to_owned())));
        watched
    }

    /// Stop watching `paths` on behalf of project `id`.
    pub fn remove(&self, id: &str, paths: &[PathBuf]) {
        self.shared.routes.write().retain(|(_, pid)| pid != id);
        self.shared.pending.lock().remove(id);
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
            let mut hits: Vec<String> = Vec::new();
            {
                let routes = shared.routes.read();
                for path in &event.paths {
                    // Lock files come and go even for read-only git commands.
                    if path.extension().is_some_and(|ext| ext == "lock") {
                        continue;
                    }
                    for (root, id) in routes.iter() {
                        if path.starts_with(root) && !hits.contains(id) {
                            hits.push(id.clone());
                        }
                    }
                }
            }
            for id in hits {
                shared.schedule(&id);
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
    pub async fn next_due(&self) -> Option<Vec<String>> {
        self.0.next_due().await
    }
}

/// Paths to watch for a repository with `git_dir` / `common_dir`.
pub(crate) fn targets(git_dir: &Path, common_dir: &Path) -> Vec<(PathBuf, RecursiveMode)> {
    let mut out = vec![(git_dir.to_path_buf(), RecursiveMode::NonRecursive)];
    if common_dir != git_dir {
        out.push((common_dir.to_path_buf(), RecursiveMode::NonRecursive));
    }
    out.push((common_dir.join("refs"), RecursiveMode::Recursive));
    // Recursive: a linked worktree's HEAD, index and FETCH_HEAD live in `worktrees/<name>/`.
    let worktrees = common_dir.join("worktrees");
    if worktrees.is_dir() {
        out.push((worktrees, RecursiveMode::Recursive));
    }
    out
}
