use std::pin::Pin;

use serde_json::Value;

use crate::ApiResult;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Routed access back into the daemon, implemented by `coocoo-core`.
///
/// Subsystems use it for cross-cutting work without depending on each other — e.g. the omp
/// host tools let an agent spawn another agent on a different host:
/// `orchestrator.call(Some("<host id>"), "agents.create", json!({...}))`.
pub trait Orchestrator: Send + Sync {
    /// Invoke a protocol method on `host` (`None` = this daemon), exactly as a client would.
    fn call(&self, host: Option<String>, method: String, params: Value) -> BoxFuture<'_, ApiResult<Value>>;

    /// This daemon's host id.
    fn local_host_id(&self) -> String;

    /// Loopback base URL of this daemon's HTTP server, e.g. `http://127.0.0.1:7421`.
    /// The browser CDP discovery endpoint for a profile is `{http_base}/cdp/{profile}`.
    fn http_base(&self) -> String;
}
