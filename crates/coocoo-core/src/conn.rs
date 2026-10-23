//! One client (or peer-link) WebSocket connection.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use coocoo_proto::bus::BusEvent;
use coocoo_proto::{ApiError, ConnSink, Request, util};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Semaphore, broadcast, mpsc};

use crate::app::{App, Caller, ConnState, encode_reply};

/// Outbound queue per connection (responses, bus events, screencast frames).
const QUEUE: usize = 512;
/// Concurrent in-flight requests per connection; further requests wait (backpressure).
const MAX_IN_FLIGHT: usize = 64;
/// Frames written per flush when the queue has a backlog.
const WRITE_BATCH: usize = 64;
const PING_EVERY: Duration = Duration::from_secs(30);
const DEAD_AFTER_MS: i64 = 90_000;

pub(crate) async fn run(app: Arc<App>, socket: WebSocket, scope_local: bool) {
    let id = app.next_conn_id();
    let (tx, mut rx) = mpsc::channel::<Arc<str>>(QUEUE);
    let sink = ConnSink::new(id, tx);
    let state = Arc::new(ConnState::default());
    state.scope_local.store(scope_local, Ordering::Relaxed);
    state.peer.store(scope_local, Ordering::Relaxed);
    // Subscribe before reading the first request so nothing after `system.hello` is missed.
    let bus = app.emitter.subscribe();
    let last_seen = AtomicI64::new(util::now_ms());
    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut shutdown = app.shutdown.subscribe();
    tracing::debug!(conn = id, "connection opened");

    tokio::select! {
        _ = write_loop(&mut ws_tx, &mut rx, &last_seen) => {}
        _ = read_loop(&app, &mut ws_rx, &sink, &state, &last_seen) => {}
        _ = forward_bus(bus, &sink, &state, &app.resync) => {}
        _ = async { let _ = shutdown.wait_for(|stop| *stop).await; } => {
            let _ = tokio::time::timeout(Duration::from_secs(1), ws_tx.send(Message::Close(None))).await;
        }
    }
    app.conn_closed(id);
    tracing::debug!(conn = id, "connection closed");
}

async fn write_loop(ws: &mut SplitSink<WebSocket, Message>, rx: &mut mpsc::Receiver<Arc<str>>, last_seen: &AtomicI64) {
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(text) = msg else { return };
                if ws.feed(text_message(text)).await.is_err() {
                    return;
                }
                // Drain a backlog with one flush instead of one syscall per frame.
                for _ in 1..WRITE_BATCH {
                    let Ok(text) = rx.try_recv() else { break };
                    if ws.feed(text_message(text)).await.is_err() {
                        return;
                    }
                }
                if ws.flush().await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if util::now_ms() - last_seen.load(Ordering::Relaxed) > DEAD_AFTER_MS {
                    tracing::debug!("closing unresponsive connection");
                    return;
                }
                if ws.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Wrap shared event text without copying it.
fn text_message(text: Arc<str>) -> Message {
    struct Shared(Arc<str>);
    impl AsRef<[u8]> for Shared {
        fn as_ref(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }
    let bytes = Bytes::from_owner(Shared(text));
    // Always valid: the bytes come from a `str`.
    Message::Text(Utf8Bytes::try_from(bytes).unwrap_or_default())
}

async fn read_loop(
    app: &Arc<App>,
    ws: &mut SplitStream<WebSocket>,
    sink: &ConnSink,
    state: &Arc<ConnState>,
    last_seen: &AtomicI64,
) {
    let permits = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    while let Some(Ok(msg)) = ws.next().await {
        last_seen.store(util::now_ms(), Ordering::Relaxed);
        match msg {
            Message::Text(text) => {
                let Ok(permit) = permits.clone().acquire_owned().await else { return };
                let req = match serde_json::from_str::<Request>(text.as_str()) {
                    Ok(req) => req,
                    Err(err) => {
                        #[derive(Deserialize)]
                        struct IdOnly {
                            id: u64,
                        }
                        if let Ok(IdOnly { id }) = serde_json::from_str(text.as_str()) {
                            sink.send(encode_reply(id, Err(ApiError::invalid(err.to_string())))).await;
                        } else {
                            tracing::debug!("ignoring malformed frame: {err}");
                        }
                        continue;
                    }
                };
                let app = app.clone();
                let caller = Caller { sink: sink.clone(), conn: Some(state.clone()), origin: state.origin(req.origin) };
                tokio::spawn(async move {
                    let reply = app.call(&caller, req.host, &req.method, req.params).await;
                    caller.sink.send(encode_reply(req.id, reply)).await;
                    drop(permit);
                });
            }
            Message::Close(_) => return,
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}

async fn forward_bus(mut bus: broadcast::Receiver<Arc<BusEvent>>, sink: &ConnSink, state: &ConnState, resync: &Arc<str>) {
    loop {
        match bus.recv().await {
            Ok(event) => {
                if !event.local && state.scope_local.load(Ordering::Relaxed) {
                    continue;
                }
                // Reliable: a slow client blocks here, the broadcast buffer absorbs the burst and
                // past its capacity the client is told to resync instead of growing memory.
                if !sink.send(event.json.clone()).await {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::debug!(conn = sink.id, missed, "connection lagged; sending resync");
                if !sink.send(resync.clone()).await {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}
