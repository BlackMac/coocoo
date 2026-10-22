//! Raw Chrome DevTools Protocol client: one browser-level WebSocket, flattened sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use coocoo_proto::{ApiError, ApiResult};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Event pushed by Chromium (`method` + `params`), optionally scoped to a flattened session.
pub(crate) struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Incoming {
    id: Option<u64>,
    #[serde(default)]
    result: Value,
    error: Option<CdpError>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct CdpError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<ApiResult<Value>>>>;

pub(crate) struct Cdp {
    out: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    pending: Pending,
    closed: AtomicBool,
}

impl Cdp {
    /// Connect to Chromium's browser endpoint. `on_event` runs on the reader task (must not
    /// block); `on_close` runs once when the socket ends.
    pub async fn connect<E, C>(ws_url: &str, on_event: E, on_close: C) -> ApiResult<Arc<Self>>
    where
        E: Fn(&Arc<Cdp>, CdpEvent) + Send + 'static,
        C: FnOnce() + Send + 'static,
    {
        let config = WebSocketConfig::default().max_message_size(Some(256 << 20)).max_frame_size(Some(256 << 20));
        let (ws, _) = tokio_tungstenite::connect_async_with_config(ws_url, Some(config), true)
            .await
            .map_err(|e| ApiError::unavailable(format!("CDP connect {ws_url}: {e}")))?;
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let cdp = Arc::new(Cdp { out, next_id: AtomicU64::new(1), pending: Mutex::new(HashMap::new()), closed: AtomicBool::new(false) });

        tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader = cdp.clone();
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let text = match msg {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) => break,
                    Err(err) => {
                        // Chromium exiting resets the socket without a close handshake.
                        tracing::debug!(%err, "CDP socket ended");
                        break;
                    }
                    Ok(_) => continue,
                };
                let incoming: Incoming = match serde_json::from_str(text.as_str()) {
                    Ok(m) => m,
                    Err(err) => {
                        tracing::debug!(%err, "unparseable CDP message");
                        continue;
                    }
                };
                drop(text);
                if let Some(id) = incoming.id {
                    if let Some(tx) = reader.pending.lock().remove(&id) {
                        let result = match incoming.error {
                            Some(e) => Err(ApiError::new("cdp_error", format!("{} ({})", e.message, e.code))),
                            None => Ok(incoming.result),
                        };
                        let _ = tx.send(result);
                    }
                } else if let Some(method) = incoming.method {
                    on_event(&reader, CdpEvent { method, params: incoming.params, session_id: incoming.session_id });
                }
            }
            reader.closed.store(true, Ordering::Release);
            // Fail in-flight calls instead of letting them run into their timeouts.
            reader.pending.lock().clear();
            on_close();
        });
        Ok(cdp)
    }

    fn encode(&self, method: &str, params: Value, session: Option<&str>) -> (u64, String) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            msg["sessionId"] = json!(session);
        }
        (id, msg.to_string())
    }

    /// Send a command and await its result.
    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> ApiResult<Value> {
        self.call_with_timeout(method, params, session, CALL_TIMEOUT).await
    }

    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> ApiResult<Value> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ApiError::unavailable("browser connection closed"));
        }
        let (id, text) = self.encode(method, params, session);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);
        if self.out.send(text).is_err() {
            self.pending.lock().remove(&id);
            return Err(ApiError::unavailable("browser connection closed"));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ApiError::unavailable("browser connection closed")),
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(ApiError::unavailable(format!("CDP {method} timed out")))
            }
        }
    }

    /// Fire-and-forget command (acks, teardown); the response is discarded.
    pub fn send(&self, method: &str, params: Value, session: Option<&str>) {
        let (_, text) = self.encode(method, params, session);
        let _ = self.out.send(text);
    }
}
