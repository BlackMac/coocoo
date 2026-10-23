//! WebSocket envelopes (text frames, one JSON object each).
//!
//! ```text
//! client → daemon   {"id": 7, "method": "agents.create", "params": {...}, "host": "<host id>"?}
//! daemon → client   {"id": 7, "result": ...}  |  {"id": 7, "error": {"code": "...", "message": "..."}}
//! daemon → client   {"event": "agent.updated", "host": "<origin host id>", "data": {...}}
//! ```
//!
//! `host` on a request selects the target daemon; absent or equal to the home daemon's id means
//! "handle locally", any other value is forwarded over the peer link to that host.
//! Every event names the host it originated on, so a client connected to one daemon sees the
//! whole fleet without re-tagging.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ApiError;
use crate::insights::Origin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Who caused a request a peer link forwards (`Origin`); ignored from clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    pub fn from_result(id: u64, result: Result<Value, ApiError>) -> Self {
        match result {
            Ok(value) => Self { id, result: Some(value), error: None },
            Err(err) => Self { id, result: None, error: Some(err) },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    pub host: String,
    #[serde(default)]
    pub data: Value,
}

/// Any frame the daemon sends; used by peer links and clients to classify incoming text.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServerFrame {
    Response(Response),
    Event(EventEnvelope),
}

/// Event names. Payload types are documented next to each constant.
pub mod events {
    /// `HostInfo` — a host was added, came online/offline, or changed.
    pub const HOST_UPDATED: &str = "host.updated";
    /// `{ "id": String }` — a peer host was removed from the fleet.
    pub const HOST_REMOVED: &str = "host.removed";
    /// `HostStats` — emitted every ~2 s while at least one client runs `stats.watch`.
    pub const HOST_STATS: &str = "host.stats";
    /// `Project`
    pub const PROJECT_UPDATED: &str = "project.updated";
    /// `{ "id": String }`
    pub const PROJECT_REMOVED: &str = "project.removed";
    /// `AgentSummary`
    pub const AGENT_UPDATED: &str = "agent.updated";
    /// `{ "id": String }`
    pub const AGENT_REMOVED: &str = "agent.removed";
    /// `TimelineEvent` — upsert of one timeline item (same `item.id` replaces the earlier one).
    pub const AGENT_TIMELINE: &str = "agent.timeline";
    /// `TimelineDelta` — streaming text appended to an in-progress assistant/thinking item.
    pub const AGENT_DELTA: &str = "agent.delta";
    /// `UiRequest` — the agent is blocked on user input.
    pub const AGENT_REQUEST: &str = "agent.request";
    /// `{ "agentId": String, "requestId": String }` — request answered, cancelled, or expired.
    pub const AGENT_REQUEST_RESOLVED: &str = "agent.requestResolved";
    /// `BoardEvent` — a project's Kanban board changed.
    pub const PROJECT_BOARD: &str = "project.board";
    /// `CommandsEvent` — an agent's slash-command catalog changed.
    pub const AGENT_COMMANDS: &str = "agent.commands";
    /// `BrowserProfileInfo` — profile started/stopped or its tab list changed.
    pub const BROWSER_UPDATED: &str = "browser.updated";
    /// `BrowserFrame` — screencast frame; sent only to connections watching that tab.
    pub const BROWSER_FRAME: &str = "browser.frame";
    /// `BrowserConsoleEntry` — console/log/exception line; sent only to watching connections.
    pub const BROWSER_CONSOLE: &str = "browser.console";
    /// `{ "id": String, "data": String }` — terminal output; sent only to attached connections.
    pub const TERMINAL_OUTPUT: &str = "terminal.output";
    /// `{ "id": String, "code": i32? }` — the terminal's process exited; sent only to attached
    /// connections.
    pub const TERMINAL_EXIT: &str = "terminal.exit";
    /// `TerminalInfo` — a terminal was opened or its process ended (broadcast, so every client
    /// sees which actions run without attaching).
    pub const TERMINAL_UPDATED: &str = "terminal.updated";
    /// `{ "id": String }` — a terminal is gone (shell exited, action run closed or replaced).
    pub const TERMINAL_REMOVED: &str = "terminal.removed";
    /// `{}` — the connection lagged behind the event stream; the client must refetch state.
    pub const SYSTEM_RESYNC: &str = "system.resync";
}
