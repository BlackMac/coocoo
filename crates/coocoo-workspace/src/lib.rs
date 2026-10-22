//! Projects registry, git (status, diff, worktrees, commit, merge), filesystem browsing and
//! host resource stats. Handles the `projects.*`, `git.*`, `fs.*` and `stats.*` methods.
//!
//! git is driven through the `git` CLI (no libgit2): smaller binary, exact user semantics.

use std::sync::Arc;

use coocoo_proto::{ApiResult, ConnSink, Emitter, Paths, Project};
use serde_json::Value;

/// Returns `(key, pid)` for every process tree whose resources should be reported
/// (`key` = agent id or `browser:<profile>`).
pub type PidSource = Arc<dyn Fn() -> Vec<(String, u32)> + Send + Sync>;

pub struct Workspace {}

impl Workspace {
    /// Loads `<data_dir>/projects.json`.
    pub fn new(emitter: Emitter, paths: &Paths) -> Arc<Self> {
        let _ = (emitter, paths);
        todo!()
    }

    /// `projects.*`, `git.*`, `fs.*`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value) -> ApiResult<Value> {
        let _ = (method, params);
        todo!()
    }

    pub fn projects(&self) -> Vec<Project> {
        todo!()
    }
}

/// Host stats sampler; samples only while at least one connection watches.
pub struct Stats {}

impl Stats {
    pub fn new(emitter: Emitter, pids: PidSource) -> Arc<Self> {
        let _ = (emitter, pids);
        todo!()
    }

    /// `stats.watch`, `stats.get`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value, conn: &ConnSink) -> ApiResult<Value> {
        let _ = (method, params, conn);
        todo!()
    }

    /// Drop the connection's watch, if any.
    pub fn conn_closed(&self, conn_id: u64) {
        let _ = conn_id;
        todo!()
    }
}
