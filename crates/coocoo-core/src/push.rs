//! Web Push (`push.*`): VAPID-authorized push notifications to subscribed devices.
//!
//! A device registers on the daemon it talks to; each daemon pushes only its own agents'
//! events (local-origin bus events), so a phone connected to two daemons gets every event
//! exactly once and no daemon depends on another for delivery.
//!
//! Storage: `<data_dir>/push/subscriptions.json` (atomic writes), VAPID keypair generated
//! once into `<data_dir>/push/vapid.key` (0600). The public key (uncompressed point,
//! base64url) goes to the client for `PushManager.subscribe`.
//!
//! Payloads stay small (notification title/body only, never transcript text): they are
//! readable on the lock screen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use coocoo_proto::bus::BusEvent;
use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{now_ms, read_json, truncate, write_json_atomic};
use coocoo_proto::{ApiError, ApiResult, Paths};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// Shared push HTTP client (rustls, webpki roots).
type PushClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<hyper::body::Bytes>,
>;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use web_push::{SubscriptionInfo, VapidSignatureBuilder, WebPushMessageBuilder};

use crate::app::App;

/// How often dead subscriptions are pruned.
const SWEEP_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Messages are held by the push service for at most this long (TTL, seconds).
const TTL: u32 = 24 * 3600;
/// A subscription whose device has not been seen for this long is pruned.
const STALE_AFTER_MS: i64 = 90 * 24 * 3600 * 1000;
/// Max body bytes of a notification payload.
const MAX_BODY: usize = 200;

/// One registered device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushSubscription {
    /// Browser push endpoint (capability URL: secret, never logged).
    pub endpoint: String,
    /// Encryption keys of the subscription (base64url).
    pub keys: PushKeys,
    /// Client installation id (usage log vocabulary) and device kind.
    pub client_id: String,
    pub device: String,
    /// What the device wants pushed.
    #[serde(default)]
    pub prefs: PushPrefs,
    pub created_at: i64,
    /// Last `push.subscribe`/`push.ack` from this device (Unix ms).
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Per-device notification preferences (defaults mirror the refinement).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushPrefs {
    /// `ask`/`secret` requests ("Agent needs you").
    pub needs_you: bool,
    /// Turn finished or failed ("Ready for review" / "Agent failed").
    pub finished: bool,
    /// Include the agent title; without it notifications name only the event.
    pub agent_names: bool,
}

impl Default for PushPrefs {
    fn default() -> Self {
        Self { needs_you: true, finished: true, agent_names: true }
    }
}

/// A device as `push.devices` reports it (no endpoint or keys: they are secrets).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInfo {
    client_id: String,
    device: String,
    prefs: PushPrefs,
    created_at: i64,
    last_seen: i64,
}

/// What happened, as the payload tells the service worker.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    title: &'a str,
    body: String,
    /// In-app route the notification opens (`#/a/<host>/<agent>`).
    hash: String,
    /// Replaces an earlier notification of the same agent and kind.
    tag: String,
}

pub(crate) struct Push {
    store: PathBuf,
    /// Private key scalar, base64url — the form `VapidSignatureBuilder::from_base64_no_sub` takes.
    vapid_private: String,
    /// Public key (uncompressed point, base64url) for `PushManager.subscribe`.
    vapid_public: String,
    subscriptions: Mutex<Vec<PushSubscription>>,
    /// Client ids with a connected, visible client; pushes skip them (the tab already shows it).
    present: Mutex<HashSet<String>>,
    client: PushClient,
}

impl Push {
    /// Loads (or creates) the VAPID keypair and subscriptions. Must run inside the runtime.
    pub fn new(paths: &Paths) -> Arc<Self> {
        crate::ece_backend::install();
        let dir = paths.data_dir.join("push");
        let (vapid_private, vapid_public) = load_or_create_keys(&dir.join("vapid.key"));
        let store = dir.join("subscriptions.json");
        let subscriptions = read_json::<Vec<PushSubscription>>(&store).ok().flatten().unwrap_or_default();
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Arc::new(Self {
            store,
            vapid_private,
            vapid_public,
            subscriptions: Mutex::new(subscriptions),
            present: Mutex::new(HashSet::new()),
            client: Client::builder(TokioExecutor::new()).build(https),
        })
    }

    /// The background task: watch the bus for this daemon's agents, prune dead subscriptions.
    pub fn start(self: &Arc<Self>, app: &Arc<App>) {
        let (me, mut shutdown) = (self.clone(), app.shutdown.subscribe());
        let agents = app.agents.clone();
        let mut bus = app.emitter.subscribe();
        let host_id = app.host_id.clone();
        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(SWEEP_EVERY);
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break; }
                    }
                    _ = sweep.tick() => me.sweep(),
                    event = bus.recv() => {
                        match event {
                            Ok(event) => me.on_event(&agents, &host_id, &event),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                }
            }
        });
    }

    /// One bus event. Only local-origin agent events: this daemon pushes its own agents.
    fn on_event(self: &Arc<Self>, agents: &Arc<coocoo_agents::Agents>, host_id: &str, event: &BusEvent) {
        if !event.local {
            return;
        }
        let data = match serde_json::from_str::<Value>(&event.json) {
            Ok(v) => v.get("data").cloned().unwrap_or(Value::Null),
            Err(_) => return,
        };
        let title_of = |id: &str| agents.summary(id).map(|s| s.title).unwrap_or_default();
        self.on_event_data(host_id, event.name.as_str(), &data, title_of);
    }

    /// The event with the agent-title lookup already resolved (tests inject their own).
    fn on_event_data(
        self: &Arc<Self>,
        host_id: &str,
        name: &str,
        data: &Value,
        title_of: impl Fn(&str) -> String,
    ) {
        match name {
            "agent.request" => {
                let agent_id = data.get("agentId").and_then(Value::as_str).unwrap_or_default().to_owned();
                if agent_id.is_empty() {
                    return;
                }
                let title = title_of(&agent_id);
                self.notify(host_id, "needs_you", &agent_id, &title);
            }
            "agent.updated" => {
                let agent_id = data.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                let unread = data.get("unread").and_then(Value::as_bool).unwrap_or(false);
                let status = data.get("status").and_then(Value::as_str).unwrap_or_default();
                let pending = data.get("pendingRequests").and_then(Value::as_u64).unwrap_or(0);
                // The tab-notification's trigger: a turn ended (unread flips on) without an
                // open request. `error` already flipped unread when the turn failed.
                if agent_id.is_empty() || !unread || pending > 0 {
                    return;
                }
                let kind = if status == "error" { "failed" } else { "finished" };
                let title = data.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
                self.notify(host_id, kind, &agent_id, &title);
            }
            _ => {}
        }
    }

    /// Fan one notification out to every subscribed, non-present device that wants it.
    fn notify(self: &Arc<Self>, host_id: &str, kind: &str, agent_id: &str, title: &str) {

        let subs = self.subscriptions.lock().clone();
        if subs.is_empty() {
            return;
        }
        let present = self.present.lock().clone();
        for sub in subs {
            let wants = match kind {
                "needs_you" => sub.prefs.needs_you,
                "finished" | "failed" => sub.prefs.finished,
                _ => false,
            };
            if !wants || present.contains(&sub.client_id) {
                continue;
            }
            let (title_text, body) = notification_text(kind, sub.prefs.agent_names.then(|| title.trim()).unwrap_or(""));
            let payload = Payload {
                title: title_text,
                body: truncate(&body, MAX_BODY),
                hash: format!("#/a/{}/{}", host_id, agent_id),
                tag: format!("{kind}:{agent_id}"),
            };
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(err) = me.send(&sub, &payload).await {
                    tracing::debug!(client = %sub.client_id, "push to a device failed: {err}");
                }
            });
        }
    }

    /// Encrypt and POST one notification; `404`/`410` prunes the subscription.
    async fn send(&self, sub: &PushSubscription, payload: &Payload<'_>) -> Result<(), String> {
        let info = SubscriptionInfo::new(&sub.endpoint, &sub.keys.p256dh, &sub.keys.auth);
        let mut sig = VapidSignatureBuilder::from_base64_no_sub(&self.vapid_private)
            .map_err(|e| e.to_string())?
            .add_sub_info(&info);
        sig.add_claim("sub", "mailto:coocoo@localhost");
        let signature = sig.build().map_err(|e| e.to_string())?;
        let mut builder = WebPushMessageBuilder::new(&info);
        builder.set_vapid_signature(signature);
        let bytes = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
        builder.set_payload(web_push::ContentEncoding::Aes128Gcm, &bytes);
        // The Topic header collapses queued messages while offline; it must be base64url
        // (no ':'), so the SW tag keeps the readable form and this is a compact alias.
        let topic = payload.tag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect::<String>();
        builder.set_topic(topic);
        builder.set_ttl(TTL);
        let message = builder.build().map_err(|e| e.to_string())?;
        let request = web_push::request_builder::build_request::<Vec<u8>>(message);
        // `web-push` builds with http 0.2; our client speaks http 1 — copy the parts.
        let (parts, body) = request.into_parts();
        let mut req = hyper::Request::builder().method("POST").uri(parts.uri.to_string());
        for (name, value) in parts.headers.iter() {
            req = req.header(name.as_str(), value.as_bytes());
        }
        let req = req.body(http_body_util::Full::new(hyper::body::Bytes::from(body))).map_err(|e| e.to_string())?;
        let response = self.client.request(req).await.map_err(|e| e.to_string())?;
        let status = response.status();
        let body = response.into_body();
        let collected = BodyExt::collect(body)
            .await
            .map_err(|e| e.to_string())?
            .to_bytes()
            .to_vec();
        match status.as_u16() {
            404 | 410 => {
                let mut subs = self.subscriptions.lock();
                subs.retain(|s| s.endpoint != sub.endpoint);
                let _ = write_json_atomic(&self.store, &*subs);
                Err("subscription expired".into())
            }
            200..=299 => Ok(()),
            _ => Err(format!("push service answered {status}: {collected:?}")),
        }
    }

    fn sweep(&self) {
        let cutoff = now_ms() - STALE_AFTER_MS;
        let mut subs = self.subscriptions.lock();
        let before = subs.len();
        subs.retain(|s| s.last_seen >= cutoff);
        if subs.len() != before {
            let _ = write_json_atomic(&self.store, &*subs);
        }
    }

    // ─── RPC handlers ─────────────────────────────────────────────────────────────────────────

    pub async fn dispatch(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "push.state" => ok(json!({ "vapidPublicKey": self.vapid_public })),
            "push.subscribe" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    endpoint: String,
                    keys: PushKeys,
                    client_id: String,
                    #[serde(default)]
                    device: String,
                    #[serde(default)]
                    prefs: Option<PushPrefs>,
                }
                let p: P = params(p)?;
                if !p.endpoint.starts_with("https://") {
                    return Err(ApiError::invalid("endpoint must be an https push-service URL"));
                }
                if p.client_id.chars().count() > 64 {
                    return Err(ApiError::invalid("clientId too long"));
                }
                let now = now_ms();
                let prefs = p.prefs.unwrap_or_default();
                let mut subs = self.subscriptions.lock();
                let existing = subs.iter().position(|s| s.endpoint == p.endpoint);
                let sub = PushSubscription {
                    endpoint: p.endpoint,
                    keys: p.keys,
                    client_id: p.client_id,
                    device: p.device,
                    prefs,
                    created_at: existing.map_or(now, |i| subs[i].created_at),
                    last_seen: now,
                };
                match existing {
                    Some(i) => subs[i] = sub,
                    None => subs.push(sub),
                }
                let _ = write_json_atomic(&self.store, &*subs);
                ok(json!({}))
            }
            "push.unsubscribe" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    endpoint: String,
                }
                let p: P = params(p)?;
                let mut subs = self.subscriptions.lock();
                subs.retain(|s| s.endpoint != p.endpoint);
                let _ = write_json_atomic(&self.store, &*subs);
                ok(json!({}))
            }
            "push.setPrefs" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    client_id: String,
                    prefs: PushPrefs,
                }
                let p: P = params(p)?;
                let mut subs = self.subscriptions.lock();
                for s in subs.iter_mut() {
                    if s.client_id == p.client_id {
                        s.prefs = p.prefs;
                        s.last_seen = now_ms();
                    }
                }
                let _ = write_json_atomic(&self.store, &*subs);
                ok(json!({}))
            }
            "push.devices" => {
                let subs = self.subscriptions.lock();
                let devices: Vec<DeviceInfo> = subs
                    .iter()
                    .map(|s| DeviceInfo {
                        client_id: s.client_id.clone(),
                        device: s.device.clone(),
                        prefs: s.prefs,
                        created_at: s.created_at,
                        last_seen: s.last_seen,
                    })
                    .collect();
                ok(serde_json::to_value(devices).unwrap_or(Value::Null))
            }
            "push.ack" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    client_id: String,
                    visible: bool,
                }
                let p: P = params(p)?;
                let mut present = self.present.lock();
                if p.visible {
                    present.insert(p.client_id);
                } else {
                    present.remove(&p.client_id);
                }
                ok(json!({}))
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }
}

/// Notification title/body per trigger; `agent` empty means privacy mode (no names).
fn notification_text(kind: &str, agent: &str) -> (&'static str, String) {
    match kind {
        "needs_you" => ("Agent needs you", agent.to_owned()),
        "finished" => ("Ready for review", agent.to_owned()),
        "failed" => ("Agent failed", agent.to_owned()),
        _ => ("Coocoo", String::new()),
    }
}

/// The VAPID keypair, as (private base64url, public base64url); generated on first call.
fn load_or_create_keys(path: &Path) -> (String, String) {
    if let Ok(Some(text)) = read_json::<Value>(path)
        && let (Some(private), Some(public)) = (
            text.get("private").and_then(Value::as_str),
            text.get("public").and_then(Value::as_str),
        )
    {
        return (private.to_owned(), public.to_owned());
    }
    let secret = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
    let private = B64.encode(secret.to_bytes());
    let builder = VapidSignatureBuilder::from_base64_no_sub(&private).expect("fresh key decodes");
    let public = B64.encode(builder.get_public_key());
    let _ = write_json_atomic(path, &json!({ "private": private, "public": public }));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    (private, public)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(needs_you: bool, finished: bool, agent_names: bool) -> PushPrefs {
        PushPrefs { needs_you, finished, agent_names }
    }

    #[test]
    fn notification_text_kinds() {
        assert_eq!(notification_text("needs_you", "Fix login"), ("Agent needs you", "Fix login".to_owned()));
        assert_eq!(notification_text("finished", "Fix login"), ("Ready for review", "Fix login".to_owned()));
        assert_eq!(notification_text("failed", ""), ("Agent failed", String::new()));
        // Privacy mode: the name never reaches the payload.
        assert_eq!(notification_text("needs_you", "").1, "");
    }

    #[test]
    fn prefs_default_all_on() {
        assert!(prefs(true, true, true).needs_you);
        let d = PushPrefs::default();
        assert!(d.needs_you && d.finished && d.agent_names);
    }

    /// VAPID key generation is deterministic in shape: 32-byte private, 65-byte public point.
    #[test]
    fn vapid_key_shape() {
        let dir = std::env::temp_dir().join(format!("coocoo-push-{}", coocoo_proto::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (private, public) = load_or_create_keys(&dir.join("vapid.key"));
        assert_eq!(B64.decode(&private).unwrap().len(), 32);
        assert_eq!(B64.decode(&public).unwrap().len(), 65);
        // Reload returns the same key.
        let (private2, public2) = load_or_create_keys(&dir.join("vapid.key"));
        assert_eq!((private, public), (private2, public2));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// End-to-end: an `agent.request` on the bus pushes an encrypted notification to a
    /// subscribed device — against a local mock push service over plain HTTP (the real
    /// services are https; the client accepts both, the sweep/404 logic is unaffected).
    #[tokio::test]
    async fn delivers_encrypted_push_on_agent_request() {
        crate::ece_backend::install();
        let dir = std::env::temp_dir().join(format!("coocoo-push-e2e-{}", coocoo_proto::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();

        // One-shot push service: capture method + encrypted body, answer 201.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let mut read = socket.read(&mut buf).await.unwrap();
            let head = String::from_utf8_lossy(&buf[..read]).to_string();
            let content_length: usize = head
                .split("\r\n")
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse().ok())
                })
                .unwrap_or(0);
            let header_end = head.find("\r\n\r\n").unwrap() + 4;
            let mut body = buf[header_end..read].to_vec();
            while body.len() < content_length {
                read = socket.read(&mut buf).await.unwrap();
                body.extend_from_slice(&buf[..read]);
            }
            let _ = socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n").await;
            let method = head.split(' ').next().unwrap().to_owned();
            let vapid = head.to_ascii_lowercase().contains("vapid ");
            let _ = tx.send((method, body, vapid)).await;
        });

        let push = Push::new(&coocoo_proto::Paths {
            config_dir: dir.join("config"),
            data_dir: dir.clone(),
            cache_dir: dir.join("cache"),
        });
        // Direct store insert (the RPC validates https; the mock service is plain HTTP).
        {
            let now = now_ms();
            let mut subs = push.subscriptions.lock();
            subs.push(PushSubscription {
                endpoint: format!("http://{addr}/wpush/v1/test"),
                keys: PushKeys {
                    p256dh: "BGa4N1PI79lboMR_YrwCiCsgp35DRvedt7opHcf0yM3iOBTSoQYqQLwWxAfRKE6tsDnReWmhsImkhDF_DBdkNSU".into(),
                    auth: "EvcWjEgzr4rbvhfi3yds0A".into(),
                },
                client_id: "e2e-client".into(),
                device: "phone".into(),
                prefs: PushPrefs::default(),
                created_at: now,
                last_seen: now,
            });
        }

        // The event the agents subsystem emits when an agent asks the user something.
        let data = json!({ "id": "r1", "agentId": "agent1", "method": "ask", "payload": {}, "createdAt": 1 });
        push.on_event_data("e2ehost", "agent.request", &data, |id| format!("Title of {id}"));

        let (method, body, vapid) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("push service was called").unwrap();
        server.abort();
        assert_eq!(method, "POST");
        assert!(vapid, "the request must carry the VAPID Authorization header");
        assert!(body.len() > 100, "payload must be encrypted aes128gcm content, got {} bytes", body.len());
        std::fs::remove_dir_all(&dir).ok();
    }
}
