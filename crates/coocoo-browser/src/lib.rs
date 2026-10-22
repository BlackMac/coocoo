//! Daemon-owned Chromium, one process per profile (profile = project id), driven over raw CDP.
//! omp agents attach to the same browser through `browser.cdpUrl`; clients watch and control
//! tabs through a screencast. Handles the `browser.*` methods.

use std::sync::Arc;

use coocoo_proto::{ApiResult, ConnSink, Emitter, Paths};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `[browser]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrowserConfig {
    /// Chromium-family executable; default: auto-detect (`COOCOO_CHROME`, PATH, Playwright cache).
    pub chrome_path: Option<String>,
    pub headless: bool,
    /// Stop a profile's Chromium after this many seconds without tabs, watchers or CDP clients.
    pub idle_shutdown_secs: u64,
    /// Screencast frame-rate cap per watched tab.
    pub max_fps: u32,
    /// Screencast JPEG quality (1–100).
    pub quality: u32,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self { chrome_path: None, headless: true, idle_shutdown_secs: 600, max_fps: 12, quality: 70 }
    }
}

pub struct Browser {}

impl Browser {
    /// `http_base` = loopback base URL of the daemon (for `BrowserProfileInfo::cdp_url`).
    pub fn new(emitter: Emitter, paths: &Paths, config: BrowserConfig, http_base: String) -> Arc<Self> {
        let _ = (emitter, paths, config, http_base);
        todo!()
    }

    /// `browser.*`. Screencast frames for `browser.watch` go to `conn`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value, conn: &ConnSink) -> ApiResult<Value> {
        let _ = (method, params, conn);
        todo!()
    }

    /// Serves `GET /cdp/<profile>/<path>` (`json/version`, `json/list`, `json`), launching the
    /// profile's Chromium on demand. Returns Chrome's JSON unchanged.
    pub async fn cdp_discovery(self: &Arc<Self>, profile: &str, path: &str) -> ApiResult<Value> {
        let _ = (profile, path);
        todo!()
    }

    /// Stop every screencast owned by the connection.
    pub fn conn_closed(&self, conn_id: u64) {
        let _ = conn_id;
        todo!()
    }

    /// `("browser:<profile>", pid)` for every running Chromium.
    pub fn pids(&self) -> Vec<(String, u32)> {
        todo!()
    }

    pub async fn shutdown(&self) {
        todo!()
    }
}
