//! HTTP routes: `/ws`, `/api/health`, `/cdp/{profile}/…`, static UI.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{ConnectInfo, Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use coocoo_proto::ApiError;
use serde::Deserialize;
use serde_json::json;

use crate::app::{App, VERSION};

pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/ws", get(ws))
        .route("/api/health", get(health))
        .route("/cdp/{profile}", get(cdp_root))
        .route("/cdp/{profile}/{*path}", get(cdp))
        .fallback(crate::ui::serve)
        .with_state(app)
}

#[derive(Deserialize)]
struct WsQuery {
    #[serde(default)]
    token: String,
    #[serde(default)]
    scope: Option<String>,
}

async fn ws(
    State(app): State<Arc<App>>,
    Query(q): Query<WsQuery>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    if !constant_time_eq(q.token.as_bytes(), app.token.as_bytes()) {
        return (StatusCode::UNAUTHORIZED, Json(ApiError::unauthorized())).into_response();
    }
    if app.is_shutting_down() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError::unavailable("daemon is shutting down"))).into_response();
    }
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return rejection.into_response(),
    };
    let scope_local = q.scope.as_deref() == Some("local");
    upgrade.on_upgrade(move |socket| crate::conn::run(app, socket, scope_local))
}

/// Unauthenticated liveness probe; CORS-open so UIs on other origins (desktop webview, dev
/// server) can tell "daemon down" from "bad token". Reveals no secrets.
async fn health(State(app): State<Arc<App>>) -> Response {
    let name = app.config.lock().name.clone();
    let body = Json(json!({ "ok": true, "hostId": app.host_id, "name": name, "version": VERSION }));
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"), (header::CACHE_CONTROL, "no-store")], body).into_response()
}

async fn cdp_root(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(profile): Path<String>,
) -> Response {
    discovery(&app, peer, &headers, &profile, "json/version").await
}

async fn cdp(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((profile, path)): Path<(String, String)>,
) -> Response {
    discovery(&app, peer, &headers, &profile, &path).await
}

/// CDP discovery for local tools (omp's browser tool). Loopback clients only, and the `Host`
/// header must be loopback too so a web page cannot reach it through DNS rebinding.
async fn discovery(app: &Arc<App>, peer: SocketAddr, headers: &HeaderMap, profile: &str, path: &str) -> Response {
    if !is_loopback(peer.ip()) || !host_header_is_loopback(headers) {
        return (StatusCode::FORBIDDEN, Json(ApiError::new("forbidden", "CDP discovery is loopback-only"))).into_response();
    }
    match app.browser.cdp_discovery(profile, path).await {
        Ok(value) => Json(value).into_response(),
        Err(err) => (status_for(&err), Json(err)).into_response(),
    }
}

fn status_for(err: &ApiError) -> StatusCode {
    match err.code.as_str() {
        "not_found" | "unknown_method" => StatusCode::NOT_FOUND,
        "invalid_params" => StatusCode::BAD_REQUEST,
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "forbidden" => StatusCode::FORBIDDEN,
        "conflict" => StatusCode::CONFLICT,
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

fn host_header_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else { return true };
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(is_loopback)
}

/// Compare secrets without an early exit on the first differing byte.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn loopback_hosts() {
        let mut h = HeaderMap::new();
        for (value, ok) in [("127.0.0.1:7421", true), ("localhost", true), ("[::1]:7421", true), ("evil.com:7421", false), ("10.0.0.1", false)] {
            h.insert(header::HOST, value.parse().unwrap());
            assert_eq!(host_header_is_loopback(&h), ok, "{value}");
        }
    }
}
