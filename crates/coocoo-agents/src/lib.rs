//! Agent supervisor. Every agent is one `omp --mode rpc` process driven through the vendored
//! official client (`omp-rpc`). Handles the `agents.*` methods.

use std::sync::Arc;

use coocoo_proto::{ApiResult, Emitter, Orchestrator, Paths};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `[agents]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentsConfig {
    /// omp executable; default: `omp` on PATH, then `~/.local/bin/omp`, `~/.bun/bin/omp`.
    pub omp_path: Option<String>,
    /// Stop idle agent processes after this many seconds (session kept, resumed on demand).
    /// `0` disables hibernation.
    pub hibernate_after_secs: u64,
    /// Default `provider/modelId` for new agents; `None` = omp's configured default.
    pub default_model: Option<String>,
    pub default_thinking: Option<String>,
    /// Extra CLI args appended to every `omp --mode rpc` invocation.
    pub extra_args: Vec<String>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self { omp_path: None, hibernate_after_secs: 900, default_model: None, default_thinking: None, extra_args: Vec::new() }
    }
}

pub struct Agents {}

impl Agents {
    /// Loads persisted agents from `<data_dir>/agents/`; previously live agents come back as
    /// `hibernated` (no process is spawned until they are prompted).
    pub fn new(emitter: Emitter, paths: &Paths, config: AgentsConfig) -> Arc<Self> {
        let _ = (emitter, paths, config);
        todo!()
    }

    /// Must be called once before the first `dispatch`.
    pub fn set_orchestrator(&self, orchestrator: Arc<dyn Orchestrator>) {
        let _ = orchestrator;
        todo!()
    }

    /// `agents.*`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value) -> ApiResult<Value> {
        let _ = (method, params);
        todo!()
    }

    /// `(agent id, omp pid)` for every live process.
    pub fn pids(&self) -> Vec<(String, u32)> {
        todo!()
    }

    /// Persist state and stop every omp process (sessions stay resumable).
    pub async fn shutdown(&self) {
        todo!()
    }
}
