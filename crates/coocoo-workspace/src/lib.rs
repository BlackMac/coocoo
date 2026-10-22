//! Projects registry, git (status, diff, worktrees, commit, merge), filesystem browsing, host
//! resource stats and terminals. Handles the `projects.*`, `git.*`, `fs.*`, `stats.*` and
//! `terminal.*` methods.
//!
//! git is driven through the `git` CLI (no libgit2): smaller binary, exact user semantics.

mod diff;
mod fs;
mod git;
mod icon;
mod projects;
mod stats;
mod terminal;
mod watch;

pub use terminal::Terminals;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use coocoo_proto::{ApiResult, ConnSink, Emitter, Paths, Project};
use parking_lot::Mutex;
use serde_json::Value;

/// Returns `(key, pid)` for every process tree whose resources should be reported
/// (`key` = agent id or `browser:<profile>`).
pub type PidSource = Arc<dyn Fn() -> Vec<(String, u32)> + Send + Sync>;

/// Projects registry plus git and filesystem operations.
///
/// Git state is refreshed event-driven: one filesystem watcher observes each project's git
/// directory (HEAD, index, refs — never the working tree) and triggers a debounced refresh.
pub struct Workspace {
    emitter: Emitter,
    data_dir: PathBuf,
    file: PathBuf,
    state: Mutex<projects::State>,
    watches: watch::Watches,
    /// Serializes git refreshes per project so an older result never overwrites a newer one.
    refresh_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    started: AtomicBool,
}

impl Workspace {
    /// Loads `<data_dir>/projects.json`.
    pub fn new(emitter: Emitter, paths: &Paths) -> Arc<Self> {
        projects::create(emitter, paths)
    }

    /// `projects.*`, `git.*`, `fs.*`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value) -> ApiResult<Value> {
        self.handle(method, params).await
    }

    pub fn projects(&self) -> Vec<Project> {
        self.state.lock().projects.clone()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.watches.close();
    }
}

/// Host stats sampler; samples only while at least one connection watches.
pub struct Stats {
    emitter: Emitter,
    pids: PidSource,
    watch: Mutex<stats::WatchState>,
    sampler: Arc<Mutex<stats::Sampler>>,
}

impl Stats {
    pub fn new(emitter: Emitter, pids: PidSource) -> Arc<Self> {
        Arc::new(Self {
            emitter,
            pids,
            watch: Mutex::new(stats::WatchState::default()),
            sampler: Arc::new(Mutex::new(stats::Sampler::default())),
        })
    }

    /// `stats.watch`, `stats.get`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value, conn: &ConnSink) -> ApiResult<Value> {
        self.handle(method, params, conn).await
    }

    /// Drop the connection's watch, if any.
    pub fn conn_closed(&self, conn_id: u64) {
        self.set_watch(conn_id, false, None);
    }
}
