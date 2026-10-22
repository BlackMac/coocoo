//! HTTP routes: `/ws`, `/api/health`, static UI.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::{StatusCode, header};
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
}
