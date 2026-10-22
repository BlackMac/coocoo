//! Daemon-owned Chromium, one process per profile (profile = project id), driven over raw CDP.
//! omp agents attach to the same browser through `browser.cdpUrl`; clients watch and control
//! tabs through a screencast. Handles the `browser.*` methods.

mod cdp;
mod chrome;
mod discovery;
mod http;
mod input;
mod inspect;
mod limiter;
mod profile;
mod stream;
mod tabs;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use coocoo_proto::error::{ok, params as parse};
use coocoo_proto::{ApiError, ApiResult, ConnSink, Emitter, Paths};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::profile::{Profile, ScreencastParams};

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

/// State shared by every profile.
pub(crate) struct Shared {
    emitter: Emitter,
    /// `<data_dir>/browser`; one user-data-dir per profile below it.
    root: PathBuf,
    config: BrowserConfig,
    /// Per-profile CDP discovery listeners, bound on the first `browser.cdpEndpoint`.
    endpoints: Mutex<HashMap<String, discovery::Endpoint>>,
    generation: AtomicU64,
    shutting_down: AtomicBool,
}

impl Shared {
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// URL of the profile's discovery listener, or `""` while none is bound.
    fn cdp_url(&self, profile: &str) -> String {
        self.endpoints.lock().get(profile).map(discovery::Endpoint::url).unwrap_or_default()
    }
}

pub struct Browser {
    shared: Arc<Shared>,
    profiles: Mutex<HashMap<String, Arc<Profile>>>,
}

#[derive(Deserialize)]
struct ProfileParams {
    profile: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TabParams {
    profile: String,
    target_id: String,
}

#[derive(Deserialize)]
struct OpenParams {
    profile: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigateParams {
    profile: String,
    target_id: String,
    url: Option<String>,
    action: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchParams {
    profile: String,
    target_id: String,
    max_width: Option<u32>,
    max_height: Option<u32>,
    quality: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeParams {
    profile: String,
    target_id: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputParams {
    profile: String,
    target_id: String,
    event: input::InputEvent,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PointParams {
    profile: String,
    target_id: String,
    x: Option<f64>,
    y: Option<f64>,
}

/// Largest viewport accepted from `browser.resize` (CSS px per side).
const MAX_VIEWPORT: u32 = 8192;

impl Browser {
    pub fn new(emitter: Emitter, paths: &Paths, config: BrowserConfig) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                emitter,
                root: paths.data_dir.join("browser"),
                config,
                endpoints: Mutex::new(HashMap::new()),
                generation: AtomicU64::new(0),
                shutting_down: AtomicBool::new(false),
            }),
            profiles: Mutex::new(HashMap::new()),
        })
    }

    fn profile(&self, id: &str) -> ApiResult<Arc<Profile>> {
        validate_profile_id(id)?;
        let profile = self
            .profiles
            .lock()
            .entry(id.to_owned())
            .or_insert_with(|| Profile::new(id.to_owned(), self.shared.clone()))
            .clone();
        profile.touch();
        Ok(profile)
    }

    /// `browser.*`. Screencast frames for `browser.watch` go to `conn`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, params: Value, conn: &ConnSink) -> ApiResult<Value> {
        match method {
            "browser.profiles" => ok(self.list_profiles()),
            "browser.cdpEndpoint" => {
                let p: ProfileParams = parse(params)?;
                ok(json!({ "url": self.cdp_endpoint(&p.profile)? }))
            }
            "browser.start" => {
                let p: ProfileParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                profile.ensure_running().await?;
                ok(profile.info())
            }
            "browser.stop" => {
                let p: ProfileParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                profile.stop().await;
                ok(profile.info())
            }
            "browser.open" => {
                let p: OpenParams = parse(params)?;
                let url = input::normalize_url(&p.url)?;
                let profile = self.profile(&p.profile)?;
                ok(open(&profile, &url).await?)
            }
            "browser.navigate" => {
                let p: NavigateParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                navigate(&profile, &p.target_id, p.url.as_deref(), p.action.as_deref()).await?;
                ok(json!({}))
            }
            "browser.activate" => {
                let p: TabParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                let run = profile.running()?;
                known_tab(&profile, &p.target_id)?;
                run.cdp.call("Target.activateTarget", json!({ "targetId": p.target_id }), None).await?;
                profile.activate_tab(&p.target_id, None);
                ok(json!({}))
            }
            "browser.closeTab" => {
                let p: TabParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                let run = profile.running()?;
                known_tab(&profile, &p.target_id)?;
                // Closing the last window ends a headed Chromium; keep one blank tab instead.
                if profile.tab_count() <= 1 {
                    run.cdp.call("Target.createTarget", json!({ "url": "about:blank", "background": true }), None).await?;
                }
                run.cdp.call("Target.closeTarget", json!({ "targetId": p.target_id }), None).await?;
                ok(json!({}))
            }
            "browser.watch" => {
                let p: WatchParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                let bound = |v: Option<u32>| v.filter(|v| *v > 0).map(|v| v.min(MAX_VIEWPORT));
                let sc = ScreencastParams {
                    max_width: bound(p.max_width),
                    max_height: bound(p.max_height),
                    quality: p.quality.unwrap_or(self.shared.config.quality).clamp(1, 100),
                };
                profile.watch(conn, &p.target_id, sc).await?;
                ok(json!({}))
            }
            "browser.unwatch" => {
                let p: TabParams = parse(params)?;
                self.profile(&p.profile)?.unwatch(conn.id, &p.target_id);
                ok(json!({}))
            }
            "browser.resize" => {
                let p: ResizeParams = parse(params)?;
                if !(1..=MAX_VIEWPORT).contains(&p.width) || !(1..=MAX_VIEWPORT).contains(&p.height) {
                    return Err(ApiError::invalid(format!("viewport must be 1–{MAX_VIEWPORT} px per side")));
                }
                let profile = self.profile(&p.profile)?;
                let (run, session) = profile.session(&p.target_id).await?;
                run.cdp
                    .call(
                        "Emulation.setDeviceMetricsOverride",
                        json!({ "width": p.width, "height": p.height, "deviceScaleFactor": 1, "mobile": false }),
                        Some(&session),
                    )
                    .await?;
                profile.expect_frame(&p.target_id);
                ok(json!({}))
            }
            "browser.input" => {
                let p: InputParams = parse(params)?;
                let (method, cdp_params) = input::to_cdp(p.event)?;
                let profile = self.profile(&p.profile)?;
                let (run, session) = profile.session(&p.target_id).await?;
                run.cdp.call(method, cdp_params, Some(&session)).await?;
                ok(json!({}))
            }
            "browser.console" => {
                let p: TabParams = parse(params)?;
                ok(self.profile(&p.profile)?.console(&p.target_id))
            }
            "browser.screenshot" => {
                let p: TabParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                let (run, session) = profile.session(&p.target_id).await?;
                let res = run.cdp.call("Page.captureScreenshot", json!({ "format": "png" }), Some(&session)).await?;
                ok(json!({ "data": res["data"] }))
            }
            "browser.inspect" => {
                let p: PointParams = parse(params)?;
                let (Some(x), Some(y)) = (p.x, p.y) else { return Err(ApiError::invalid("x and y are required")) };
                let profile = self.profile(&p.profile)?;
                inspect(&profile, &p.target_id, x, y).await
            }
            "browser.highlight" => {
                let p: PointParams = parse(params)?;
                let profile = self.profile(&p.profile)?;
                highlight(&profile, &p.target_id, p.x.zip(p.y)).await?;
                ok(json!({}))
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    fn list_profiles(&self) -> Vec<coocoo_proto::BrowserProfileInfo> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.shared.root)
            .map(|dir| {
                dir.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|id| validate_profile_id(id).is_ok())
                    .collect()
            })
            .unwrap_or_default();
        let mut profiles = self.profiles.lock();
        ids.extend(profiles.keys().cloned());
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                profiles.entry(id.clone()).or_insert_with(|| Profile::new(id, self.shared.clone())).clone()
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|p| p.info())
            .collect()
    }

    /// Loopback URL (`http://127.0.0.1:<port>`) whose root serves the profile's CDP discovery
    /// (`/json/version`, …) — what omp gets as `browser.cdpUrl`. Binds the listener on first use;
    /// Chromium itself launches only on the first discovery request.
    pub fn cdp_endpoint(self: &Arc<Self>, profile: &str) -> ApiResult<String> {
        validate_profile_id(profile)?;
        if self.shared.shutting_down.load(Ordering::Acquire) {
            return Err(ApiError::unavailable("browser is shutting down"));
        }
        let mut endpoints = self.shared.endpoints.lock();
        if let Some(endpoint) = endpoints.get(profile) {
            return Ok(endpoint.url());
        }
        let endpoint = discovery::bind(self, profile)?;
        let url = endpoint.url();
        endpoints.insert(profile.to_owned(), endpoint);
        Ok(url)
    }

    /// Chrome's discovery JSON for `path` (`json/version`, `json/list`, `json`, `json/protocol`),
    /// launching the profile's Chromium on demand.
    async fn cdp_discovery(self: &Arc<Self>, profile: &str, path: &str) -> ApiResult<Vec<u8>> {
        let path = path.trim_matches('/');
        if !matches!(path, "json" | "json/list" | "json/version" | "json/protocol") {
            return Err(ApiError::not_found(format!("CDP endpoint `/{path}`")));
        }
        let profile = self.profile(profile)?;
        let run = profile.ensure_running().await?;
        let body = http::get(run.port, &format!("/{path}")).await?;
        profile.touch();
        serde_json::from_slice::<serde::de::IgnoredAny>(&body)
            .map_err(|e| ApiError::internal(format!("Chromium returned invalid JSON: {e}")))?;
        Ok(body)
    }

    /// Stop every screencast owned by the connection.
    pub fn conn_closed(&self, conn_id: u64) {
        let profiles: Vec<Arc<Profile>> = self.profiles.lock().values().cloned().collect();
        for profile in profiles {
            profile.conn_closed(conn_id);
        }
    }

    /// `("browser:<profile>", pid)` for every running Chromium.
    pub fn pids(&self) -> Vec<(String, u32)> {
        let profiles: Vec<Arc<Profile>> = self.profiles.lock().values().cloned().collect();
        profiles.iter().filter_map(|p| p.pid().map(|pid| (format!("browser:{}", p.id), pid))).collect()
    }

    pub async fn shutdown(&self) {
        self.shared.shutting_down.store(true, Ordering::Release);
        self.shared.endpoints.lock().clear();
        let profiles: Vec<Arc<Profile>> = self.profiles.lock().values().cloned().collect();
        futures_util::future::join_all(profiles.iter().map(|p| p.stop())).await;
    }
}

/// Profile ids become directory names: keep them to a safe, portable alphabet.
fn validate_profile_id(id: &str) -> ApiResult<()> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid { Ok(()) } else { Err(ApiError::invalid(format!("invalid browser profile id `{id}`"))) }
}

fn known_tab(profile: &Profile, target_id: &str) -> ApiResult<()> {
    profile.tab(target_id).map(|_| ()).ok_or_else(|| ApiError::not_found(format!("tab {target_id}")))
}

async fn open(profile: &Arc<Profile>, url: &str) -> ApiResult<coocoo_proto::BrowserTab> {
    let run = profile.ensure_running().await?;
    // Reuse the startup tab instead of spawning a second renderer next to an empty one.
    let target_id = match profile.sole_blank_tab() {
        Some(target_id) => {
            let (run, session) = profile.session(&target_id).await?;
            run.cdp.call("Page.navigate", json!({ "url": url }), Some(&session)).await?;
            target_id
        }
        None => {
            let res = run.cdp.call("Target.createTarget", json!({ "url": url, "background": true }), None).await?;
            res["targetId"].as_str().ok_or_else(|| ApiError::internal("Target.createTarget returned no targetId"))?.to_owned()
        }
    };
    profile.activate_tab(&target_id, Some(url));
    let mut tab = profile.tab(&target_id).ok_or_else(|| ApiError::not_found(format!("tab {target_id}")))?;
    // The target event with the new URL may still be in flight.
    if tab.url == "about:blank" && url != "about:blank" {
        tab.url = url.to_owned();
        tab.title.clear();
    }
    Ok(tab)
}

async fn navigate(profile: &Arc<Profile>, target_id: &str, url: Option<&str>, action: Option<&str>) -> ApiResult<()> {
    let url = match (action, url) {
        (Some(_), Some(_)) => return Err(ApiError::invalid("pass either `url` or `action`, not both")),
        (None, None) => return Err(ApiError::invalid("`url` or `action` is required")),
        (None, Some(url)) => Some(input::normalize_url(url)?),
        (Some(_), None) => None,
    };
    let (run, session) = profile.session(target_id).await?;
    let s = Some(session.as_str());
    match (action, url) {
        (_, Some(url)) => {
            run.cdp.call("Page.navigate", json!({ "url": url }), s).await?;
        }
        (Some("reload"), _) => {
            run.cdp.call("Page.reload", json!({}), s).await?;
        }
        (Some(dir @ ("back" | "forward")), _) => {
            let history = run.cdp.call("Page.getNavigationHistory", json!({}), s).await?;
            let current = history["currentIndex"].as_i64().unwrap_or(0);
            let index = if dir == "back" { current - 1 } else { current + 1 };
            let entry = usize::try_from(index).ok().and_then(|i| history["entries"].get(i));
            match entry.and_then(|e| e["id"].as_i64()) {
                Some(entry_id) => {
                    run.cdp.call("Page.navigateToHistoryEntry", json!({ "entryId": entry_id }), s).await?;
                }
                None => return Err(ApiError::conflict(format!("no history entry to go {dir}"))),
            }
        }
        (Some(other), _) => return Err(ApiError::invalid(format!("unknown navigation action `{other}`"))),
        (None, None) => unreachable!("validated above"),
    }
    profile.activate_tab(target_id, None);
    Ok(())
}

/// Backend node id of the element at a viewport point.
async fn node_at(run: &profile::Running, session: &str, x: f64, y: f64) -> ApiResult<i64> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err(ApiError::invalid("x/y must be non-negative viewport coordinates"));
    }
    let res = run
        .cdp
        .call(
            "DOM.getNodeForLocation",
            json!({ "x": x.round() as i64, "y": y.round() as i64, "includeUserAgentShadowDOM": false, "ignorePointerEventsNone": true }),
            Some(session),
        )
        .await
        .map_err(|_| ApiError::not_found(format!("element at ({x}, {y})")))?;
    res["backendNodeId"].as_i64().ok_or_else(|| ApiError::not_found(format!("element at ({x}, {y})")))
}

async fn highlight(profile: &Arc<Profile>, target_id: &str, point: Option<(f64, f64)>) -> ApiResult<()> {
    let (run, session) = profile.session(target_id).await?;
    let s = Some(session.as_str());
    let Some((x, y)) = point else {
        if profile.design_enabled(&session) {
            run.cdp.call("Overlay.hideHighlight", json!({}), s).await?;
        }
        return Ok(());
    };
    profile.ensure_design(&run, &session).await?;
    match node_at(&run, &session, x, y).await {
        Ok(node) => {
            run.cdp
                .call(
                    "Overlay.highlightNode",
                    json!({ "highlightConfig": inspect::highlight_config(), "backendNodeId": node }),
                    s,
                )
                .await?;
        }
        Err(_) => {
            run.cdp.call("Overlay.hideHighlight", json!({}), s).await?;
        }
    }
    Ok(())
}

/// Cut `s` to at most `max` bytes on a char boundary.
fn cap_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

async fn inspect(profile: &Arc<Profile>, target_id: &str, x: f64, y: f64) -> ApiResult<Value> {
    let (run, session) = profile.session(target_id).await?;
    let s = Some(session.as_str());
    profile.ensure_design(&run, &session).await?;
    let node = node_at(&run, &session, x, y).await?;
    let resolved = run.cdp.call("DOM.resolveNode", json!({ "backendNodeId": node }), s).await?;
    let object_id = resolved["object"]["objectId"]
        .as_str()
        .ok_or_else(|| ApiError::not_found(format!("element at ({x}, {y})")))?
        .to_owned();
    let res = run
        .cdp
        .call(
            "Runtime.callFunctionOn",
            json!({ "functionDeclaration": inspect::inspect_fn(), "objectId": object_id, "returnByValue": true }),
            s,
        )
        .await;
    run.cdp.send("Runtime.releaseObject", json!({ "objectId": object_id }), s);
    let mut res = res?;
    if let Some(err) = res.get("exceptionDetails") {
        return Err(ApiError::internal(format!("inspect script failed: {}", err["exception"]["description"].as_str().unwrap_or("exception"))));
    }
    let mut info = res["result"]["value"].take();
    if !info.is_object() {
        return Err(ApiError::not_found(format!("element at ({x}, {y})")));
    }

    let screenshot = match info["clip"].take() {
        Value::Object(clip) => {
            let num = |k: &str| clip.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let (width, height) = (num("width"), num("height"));
            let scale = inspect::shot_scale(width, height);
            let shot = run
                .cdp
                .call(
                    "Page.captureScreenshot",
                    json!({
                        "format": "png",
                        "clip": { "x": num("x"), "y": num("y"), "width": width, "height": height, "scale": scale },
                        "captureBeyondViewport": false,
                    }),
                    s,
                )
                .await?;
            // The scaled clip briefly re-renders the page; give watchers the normal view back.
            profile.refresh_frame(target_id);
            shot["data"].clone()
        }
        _ => Value::Null,
    };
    let text = |v: &Value, max: usize| cap_bytes(v.as_str().unwrap_or_default(), max).to_owned();
    let styles: BTreeMap<String, String> = serde_json::from_value(info["styles"].take()).unwrap_or_default();
    Ok(json!({
        "selector": info["selector"],
        "tag": info["tag"],
        "html": text(&info["html"], inspect::HTML_MAX),
        "text": text(&info["text"], inspect::TEXT_MAX),
        "styles": styles,
        "box": info["box"],
        "screenshot": screenshot,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ids() {
        for ok in ["default", "abc123def456", "my-project_1.2"] {
            assert!(validate_profile_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", "../x", "a/b", "a b", ".hidden", &"x".repeat(65)] {
            assert!(validate_profile_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn caps_on_char_boundary() {
        assert_eq!(cap_bytes("héllo", 2), "h");
        assert_eq!(cap_bytes("héllo", 3), "hé");
        assert_eq!(cap_bytes("abc", 10), "abc");
    }
}
