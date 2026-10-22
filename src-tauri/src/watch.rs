//! Fleet-wide agent tracking for the tray summary and native notifications.
//!
//! Events come straight from the in-process bus when the daemon is embedded (filtered by name,
//! no parsing of irrelevant events) or from a WebSocket subscription when attached to an
//! external daemon. Baselines come from short-lived `agents.list` snapshots taken at startup,
//! when a host comes online and after a lag/resync — never by polling.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use coocoo_proto::bus::BusEvent;
use coocoo_proto::{AgentStatus, events};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Wry};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;

use crate::backend::Backend;

/// Tray icon id.
pub const TRAY_ID: &str = "main";
/// Summary line when no agent is running or waiting.
pub const IDLE_LABEL: &str = "No agents active";

/// Events the tracker reacts to; everything else is skipped before parsing.
const RELEVANT: [&str; 6] = [
    events::AGENT_UPDATED,
    events::AGENT_REMOVED,
    events::AGENT_REQUEST,
    events::HOST_UPDATED,
    events::HOST_REMOVED,
    events::SYSTEM_RESYNC,
];
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
const NOTIFICATION_BODY_MAX: usize = 200;

/// Starts tracking. Lives for the rest of the process.
pub fn spawn(app: AppHandle, backend: &Backend, status_item: MenuItem<Wry>, focused: Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel(256);
    let conn = Conn { ws_url: backend.ws_url.clone(), token: backend.token.clone() };
    match &backend.daemon {
        Some(daemon) => {
            let bus = daemon.emitter.subscribe();
            tauri::async_runtime::spawn(forward_bus(bus, tx.clone()));
            let _ = tx.try_send(Input::Resync);
        }
        None => {
            tauri::async_runtime::spawn(forward_ws(conn.clone(), tx.clone()));
        }
    }
    let tracker = Tracker {
        app,
        status_item,
        focused,
        conn,
        tx,
        fleet: Fleet::default(),
        label: IDLE_LABEL.to_owned(),
    };
    tauri::async_runtime::spawn(tracker.run(rx));
}

enum Input {
    Event { name: String, host: String, json: Arc<str> },
    Snapshot { host: String, agents: Vec<AgentLite> },
    /// Refetch every host's agents.
    Resync,
}

#[derive(Clone)]
struct Conn {
    ws_url: String,
    token: String,
}

impl Conn {
    async fn connect(&self) -> anyhow::Result<Ws> {
        let url = format!("{}?token={}", self.ws_url, encode_query(&self.token));
        Ok(tokio_tungstenite::connect_async(url).await?.0)
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ─── Payload subsets (only the fields the tray needs) ────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentLite {
    id: String,
    title: String,
    status: AgentStatus,
    unread: bool,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    pending_requests: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestLite {
    agent_id: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Deserialize)]
struct HostLite {
    id: String,
    #[serde(default)]
    online: bool,
}

#[derive(Deserialize)]
struct IdOnly {
    id: String,
}

/// `{"data": …}` of an event envelope; other envelope fields are skipped.
#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

/// Head of any server frame, to route it without materializing `data`/`result`.
#[derive(Deserialize)]
struct FrameHead {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    host: Option<String>,
}

#[derive(Deserialize)]
struct Reply {
    id: u64,
    #[serde(default)]
    result: Option<Value>,
}

// ─── Event sources ───────────────────────────────────────────────────────────────────────────

async fn forward_bus(mut bus: broadcast::Receiver<Arc<BusEvent>>, tx: mpsc::Sender<Input>) {
    loop {
        let input = match bus.recv().await {
            Ok(event) if RELEVANT.contains(&event.name.as_str()) => {
                Input::Event { name: event.name.clone(), host: event.host.clone(), json: event.json.clone() }
            }
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => Input::Resync,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if tx.send(input).await.is_err() {
            return;
        }
    }
}

/// Subscribes to every host's events on the external daemon; reconnects with backoff.
async fn forward_ws(conn: Conn, tx: mpsc::Sender<Input>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match conn.connect().await {
            Ok(mut ws) => {
                backoff = Duration::from_secs(1);
                let subscribe = json!({ "id": 1, "method": "system.subscribe", "params": { "scope": "all" } });
                if ws.send(Message::text(subscribe.to_string())).await.is_ok() {
                    if tx.send(Input::Resync).await.is_err() {
                        return;
                    }
                    while let Some(Ok(message)) = ws.next().await {
                        let Message::Text(text) = message else { continue };
                        let Ok(head) = serde_json::from_str::<FrameHead>(&text) else { continue };
                        let (Some(name), Some(host)) = (head.event, head.host) else { continue };
                        if !RELEVANT.contains(&name.as_str()) {
                            continue;
                        }
                        if tx.send(Input::Event { name, host, json: Arc::from(text.as_str()) }).await.is_err() {
                            return;
                        }
                    }
                }
                tracing::info!("lost connection to the coocoo daemon; reconnecting");
            }
            Err(err) => tracing::debug!("connecting to the coocoo daemon failed: {err:#}"),
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

/// Fetches the agents of `host` (or of every online host) over a short-lived connection.
async fn snapshot(conn: Conn, host: Option<String>, tx: mpsc::Sender<Input>) {
    let result = tokio::time::timeout(SNAPSHOT_TIMEOUT, fetch_agents(&conn, host)).await;
    let snapshots = match result {
        Ok(Ok(snapshots)) => snapshots,
        Ok(Err(err)) => return tracing::debug!("agent snapshot failed: {err:#}"),
        Err(_) => return tracing::debug!("agent snapshot timed out"),
    };
    for (host, agents) in snapshots {
        if tx.send(Input::Snapshot { host, agents }).await.is_err() {
            return;
        }
    }
}

async fn fetch_agents(conn: &Conn, host: Option<String>) -> anyhow::Result<Vec<(String, Vec<AgentLite>)>> {
    let mut ws = conn.connect().await?;
    let hosts = match host {
        Some(host) => vec![host],
        None => {
            #[derive(Deserialize)]
            struct Hello {
                hosts: Vec<HostLite>,
            }
            send(&mut ws, 0, "system.hello", None).await?;
            let hello: Hello = recv_result(&mut ws, 0).await?;
            hello.hosts.into_iter().filter(|h| h.online).map(|h| h.id).collect()
        }
    };
    for (i, host) in hosts.iter().enumerate() {
        send(&mut ws, i as u64 + 1, "agents.list", Some(host)).await?;
    }
    let mut lists: Vec<Option<Vec<AgentLite>>> = hosts.iter().map(|_| None).collect();
    let mut pending = hosts.len();
    while pending > 0 {
        let reply: Reply = next_reply(&mut ws).await?;
        let Some(slot) = (reply.id as usize).checked_sub(1).and_then(|i| lists.get_mut(i)) else { continue };
        if slot.is_none() {
            pending -= 1;
            // Hosts answering with an error (offline meanwhile, old version) are just skipped.
            *slot = Some(reply.result.and_then(|r| serde_json::from_value(r).ok()).unwrap_or_default());
        }
    }
    let _ = ws.close(None).await;
    Ok(hosts.into_iter().zip(lists).map(|(host, list)| (host, list.unwrap_or_default())).collect())
}

async fn send(ws: &mut Ws, id: u64, method: &str, host: Option<&str>) -> anyhow::Result<()> {
    let mut request = json!({ "id": id, "method": method });
    if let Some(host) = host {
        request["host"] = json!(host);
    }
    ws.send(Message::text(request.to_string())).await?;
    Ok(())
}

async fn recv_result<T: DeserializeOwned>(ws: &mut Ws, id: u64) -> anyhow::Result<T> {
    loop {
        let reply = next_reply(ws).await?;
        if reply.id == id {
            let result = reply.result.ok_or_else(|| anyhow::anyhow!("request {id} failed"))?;
            return Ok(serde_json::from_value(result)?);
        }
    }
}

/// Next response frame; events on the connection are skipped.
async fn next_reply(ws: &mut Ws) -> anyhow::Result<Reply> {
    loop {
        let message = ws.next().await.ok_or_else(|| anyhow::anyhow!("connection closed"))??;
        let Message::Text(text) = message else { continue };
        let Ok(head) = serde_json::from_str::<FrameHead>(&text) else { continue };
        if head.event.is_none() && head.id.is_some() {
            return Ok(serde_json::from_str(&text)?);
        }
    }
}

// ─── Fleet state (pure) ──────────────────────────────────────────────────────────────────────

struct Agent {
    title: String,
    status: AgentStatus,
    pending: u32,
    /// A "finished" notification was shown for the current unread turn.
    finish_notified: bool,
    /// A "needs input" notification was shown for the current waiting period.
    input_notified: bool,
}

/// What the tracker must do after an input.
#[derive(Debug, Clone, PartialEq)]
enum Effect {
    Notify { title: String, body: String },
    /// Fetch the agents of one host, or of all online hosts.
    Snapshot(Option<String>),
}

/// Agent states of the whole fleet and the notification decisions derived from them.
#[derive(Default)]
struct Fleet {
    /// Keyed by `(host id, agent id)`.
    agents: HashMap<(String, String), Agent>,
    online: HashMap<String, bool>,
}

impl Fleet {
    fn on_event(&mut self, name: &str, host: String, json: &str) -> Option<Effect> {
        match name {
            events::AGENT_UPDATED => self.on_agent(host, parse(json)?),
            events::AGENT_REMOVED => {
                let IdOnly { id } = parse(json)?;
                self.agents.remove(&(host, id));
                None
            }
            events::AGENT_REQUEST => self.on_request(host, parse(json)?),
            events::HOST_UPDATED => {
                let info: HostLite = parse(json)?;
                let was_online = self.online.insert(info.id.clone(), info.online).unwrap_or(false);
                if !info.online {
                    self.agents.retain(|(h, _), _| *h != info.id);
                }
                (info.online && !was_online).then_some(Effect::Snapshot(Some(info.id)))
            }
            events::HOST_REMOVED => {
                let IdOnly { id } = parse(json)?;
                self.agents.retain(|(h, _), _| *h != id);
                self.online.remove(&id);
                None
            }
            events::SYSTEM_RESYNC => Some(Effect::Snapshot(None)),
            _ => None,
        }
    }

    fn on_agent(&mut self, host: String, update: AgentLite) -> Option<Effect> {
        let entry = self.agents.entry((host, update.id)).or_insert_with(|| Agent {
            title: String::new(),
            status: update.status,
            pending: 0,
            finish_notified: false,
            input_notified: false,
        });
        let finished = matches!(entry.status, AgentStatus::Running | AgentStatus::Starting)
            && update.status == AgentStatus::Idle
            && update.unread;
        entry.title = update.title;
        entry.status = update.status;
        entry.pending = update.pending_requests;
        if !update.unread {
            entry.finish_notified = false;
        }
        if update.pending_requests == 0 && update.status != AgentStatus::Waiting {
            entry.input_notified = false;
        }
        if !finished || entry.finish_notified {
            return None;
        }
        entry.finish_notified = true;
        Some(Effect::Notify {
            title: format!("Agent finished: {}", display_title(&entry.title)),
            body: update.preview.unwrap_or_default(),
        })
    }

    fn on_request(&mut self, host: String, request: RequestLite) -> Option<Effect> {
        let entry = self.agents.entry((host, request.agent_id)).or_insert_with(|| Agent {
            title: String::new(),
            status: AgentStatus::Waiting,
            pending: 1,
            finish_notified: false,
            input_notified: false,
        });
        if entry.input_notified {
            return None;
        }
        entry.input_notified = true;
        let body = ["title", "message", "question"]
            .iter()
            .find_map(|key| request.payload.get(key).and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        Some(Effect::Notify { title: format!("Agent needs input: {}", display_title(&entry.title)), body })
    }

    /// Replaces a host's agents with a fresh listing (no notifications for baselines).
    fn on_snapshot(&mut self, host: String, agents: Vec<AgentLite>) {
        self.agents.retain(|(h, _), _| *h != host);
        for update in agents {
            let agent = Agent {
                title: update.title,
                status: update.status,
                pending: update.pending_requests,
                // State that predates the baseline never interrupts the user.
                finish_notified: update.unread,
                input_notified: update.pending_requests > 0 || update.status == AgentStatus::Waiting,
            };
            self.agents.insert((host.clone(), update.id), agent);
        }
        self.online.insert(host, true);
    }

    /// Tray summary line.
    fn label(&self) -> String {
        let (mut running, mut waiting) = (0usize, 0usize);
        for agent in self.agents.values() {
            if matches!(agent.status, AgentStatus::Running | AgentStatus::Starting) {
                running += 1;
            }
            if agent.status == AgentStatus::Waiting || agent.pending > 0 {
                waiting += 1;
            }
        }
        let needs = |w: usize| if w == 1 { "needs" } else { "need" };
        match (running, waiting) {
            (0, 0) => IDLE_LABEL.to_owned(),
            (r, 0) => format!("{r} running"),
            (0, w) => format!("{w} {} you", needs(w)),
            (r, w) => format!("{r} running · {w} {} you", needs(w)),
        }
    }
}

// ─── Tracker (side effects) ──────────────────────────────────────────────────────────────────

struct Tracker {
    app: AppHandle,
    status_item: MenuItem<Wry>,
    focused: Arc<AtomicBool>,
    conn: Conn,
    tx: mpsc::Sender<Input>,
    fleet: Fleet,
    label: String,
}

impl Tracker {
    async fn run(mut self, mut rx: mpsc::Receiver<Input>) {
        while let Some(input) = rx.recv().await {
            let effect = match input {
                Input::Event { name, host, json } => self.fleet.on_event(&name, host, &json),
                Input::Snapshot { host, agents } => {
                    self.fleet.on_snapshot(host, agents);
                    None
                }
                Input::Resync => Some(Effect::Snapshot(None)),
            };
            match effect {
                Some(Effect::Notify { title, body }) => self.notify(title, &body),
                Some(Effect::Snapshot(host)) => {
                    tauri::async_runtime::spawn(snapshot(self.conn.clone(), host, self.tx.clone()));
                }
                None => {}
            }
            self.refresh_tray();
        }
    }

    fn refresh_tray(&mut self) {
        let label = self.fleet.label();
        if label == self.label {
            return;
        }
        if let Err(err) = self.status_item.set_text(&label) {
            tracing::debug!("tray menu update failed: {err}");
        }
        if let Some(tray) = self.app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(format!("Coocoo — {label}")));
        }
        self.label = label;
    }

    /// Native notification, only while the user is not looking at the app.
    fn notify(&self, title: String, body: &str) {
        if self.focused.load(Ordering::Relaxed) {
            return;
        }
        let mut builder = self.app.notification().builder().title(title);
        let body = truncate(body.trim(), NOTIFICATION_BODY_MAX);
        if !body.is_empty() {
            builder = builder.body(body);
        }
        if let Err(err) = builder.show() {
            tracing::debug!("notification failed: {err}");
        }
    }
}

fn parse<T: DeserializeOwned>(json: &str) -> Option<T> {
    serde_json::from_str::<Data<T>>(json).ok().map(|d| d.data)
}

fn display_title(title: &str) -> &str {
    if title.trim().is_empty() { "untitled agent" } else { title }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Percent-encodes everything outside RFC 3986 "unreserved" for a query value.
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_event(id: &str, status: &str, unread: bool, pending: u32) -> String {
        json!({
            "event": "agent.updated",
            "host": "h1",
            "data": {
                "id": id, "hostId": "h1", "title": "Fix login", "cwd": "/p", "status": status,
                "createdAt": 1, "lastActivityAt": 2, "unread": unread, "pendingRequests": pending,
                "preview": "All tests pass."
            }
        })
        .to_string()
    }

    fn update(fleet: &mut Fleet, id: &str, status: &str, unread: bool, pending: u32) -> Option<Effect> {
        fleet.on_event(events::AGENT_UPDATED, "h1".into(), &agent_event(id, status, unread, pending))
    }

    fn request(fleet: &mut Fleet, id: &str, agent: &str) -> Option<Effect> {
        let json = json!({
            "event": "agent.request", "host": "h1",
            "data": { "id": id, "agentId": agent, "method": "confirm", "payload": { "title": "Delete build/?" }, "createdAt": 3 }
        });
        fleet.on_event(events::AGENT_REQUEST, "h1".into(), &json.to_string())
    }

    #[test]
    fn finished_turn_notifies_once_per_unread_cycle() {
        let mut fleet = Fleet::default();
        assert_eq!(update(&mut fleet, "a", "running", false, 0), None);
        assert_eq!(fleet.label(), "1 running");
        let done = Effect::Notify { title: "Agent finished: Fix login".into(), body: "All tests pass.".into() };
        assert_eq!(update(&mut fleet, "a", "idle", true, 0), Some(done.clone()));
        // Follow-up turn while still unread: no second notification.
        assert_eq!(update(&mut fleet, "a", "running", true, 0), None);
        assert_eq!(update(&mut fleet, "a", "idle", true, 0), None);
        // Read, then a new turn finishes: notify again.
        assert_eq!(update(&mut fleet, "a", "idle", false, 0), None);
        assert_eq!(update(&mut fleet, "a", "running", false, 0), None);
        assert_eq!(update(&mut fleet, "a", "idle", true, 0), Some(done));
        assert_eq!(fleet.label(), IDLE_LABEL);
    }

    #[test]
    fn requests_notify_once_per_waiting_period() {
        let mut fleet = Fleet::default();
        update(&mut fleet, "a", "running", false, 0);
        let ask = Effect::Notify { title: "Agent needs input: Fix login".into(), body: "Delete build/?".into() };
        assert_eq!(request(&mut fleet, "r1", "a"), Some(ask.clone()));
        update(&mut fleet, "a", "waiting", false, 1);
        assert_eq!(fleet.label(), "1 needs you");
        assert_eq!(request(&mut fleet, "r2", "a"), None);
        update(&mut fleet, "a", "running", false, 0);
        assert_eq!(request(&mut fleet, "r3", "a"), Some(ask));
    }

    #[test]
    fn snapshots_set_baselines_and_hosts_going_offline_drop_agents() {
        let mut fleet = Fleet::default();
        let listing: Vec<AgentLite> = serde_json::from_value(json!([
            { "id": "a", "title": "A", "status": "waiting", "unread": false, "pendingRequests": 1 },
            { "id": "b", "title": "B", "status": "running", "unread": false },
            { "id": "c", "title": "C", "status": "idle", "unread": true }
        ]))
        .unwrap();
        fleet.on_snapshot("h1".into(), listing);
        assert_eq!(fleet.label(), "1 running · 1 needs you");
        // Already waiting at baseline time: no notification.
        assert_eq!(request(&mut fleet, "r1", "a"), None);

        let offline = json!({ "event": "host.updated", "host": "h1", "data": { "id": "h1", "online": false } });
        assert_eq!(fleet.on_event(events::HOST_UPDATED, "h1".into(), &offline.to_string()), None);
        assert_eq!(fleet.label(), IDLE_LABEL);
        let online = json!({ "event": "host.updated", "host": "h1", "data": { "id": "h1", "online": true } });
        assert_eq!(
            fleet.on_event(events::HOST_UPDATED, "h1".into(), &online.to_string()),
            Some(Effect::Snapshot(Some("h1".into())))
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode_query("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode_query("x+y/z="), "x%2By%2Fz%3D");
    }
}
