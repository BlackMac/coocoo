//! Usage log contract shared by the subsystems and `coocoo-core` (`insights.*`).
//!
//! The log records *what happened*, never content: no prompt texts, arguments, file contents or
//! paths — only kinds, ids, durations and small metadata such as a prompt's length.
//!
//! - [`InsightEvent`]: one log line (`<data_dir>/events/YYYY-MM.jsonl`).
//! - [`InsightsSink`]: where subsystems report domain events and agent identity changes; the
//!   daemon writes them to its event log and agent ledger.
//! - [`ORIGIN`]: who caused the work running in the current task. `coocoo-core` scopes it around
//!   a request from a client, a peer or an agent tool, so domain events emitted while handling it
//!   (a card that moves because an agent was started for it) carry the same actor.
//! - [`TOOL_AGENT`]: set by an agent's host tools around their routed calls, so the daemon can
//!   tell an agent's action from its own housekeeping.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::now_ms;

/// Who caused an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    /// A person, through a client (desktop, phone, browser).
    User,
    /// An agent, through its host tools.
    Agent,
    /// The daemon on its own (a turn that ends, a crash, hibernation).
    Daemon,
}

/// The cause of a request: carried with the call, forwarded to peers with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub actor: Actor,
    /// `desktop`, `phone` or `browser` (users only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Random id of the client installation (users only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// The acting agent (agents only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl Origin {
    pub fn agent(agent_id: impl Into<String>) -> Self {
        Self { actor: Actor::Agent, device: None, client_id: None, agent_id: Some(agent_id.into()) }
    }

    pub fn daemon() -> Self {
        Self { actor: Actor::Daemon, device: None, client_id: None, agent_id: None }
    }
}

tokio::task_local! {
    /// Cause of the request being handled in this task (see the module docs).
    pub static ORIGIN: Origin;
    /// Agent whose host tool issued the routed call running in this task.
    pub static TOOL_AGENT: String;
}

/// The origin of the work in the current task, if a request scoped one.
pub fn current_origin() -> Option<Origin> {
    ORIGIN.try_with(Clone::clone).ok()
}

/// One line of the usage log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightEvent {
    /// Unix ms.
    pub ts: i64,
    /// Method name for actions (`agents.prompt`), `ui.*` for client events, else a domain event
    /// (`turn.end`, `card.move`, `request.answer`, …). See `docs/PROTOCOL.md`.
    pub kind: String,
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Host the event was recorded on; filled in when events of several hosts are merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    /// The action succeeded (always `true` for domain and client events).
    #[serde(default = "yes")]
    pub ok: bool,
    /// Duration in ms (request handling time, turn length, wait time…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<i64>,
    /// Kind-specific metadata; never content.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
    /// Derived from older data (timeline, cards, omp stats) instead of recorded live; only in
    /// query results, never in the log.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reconstructed: bool,
}

fn yes() -> bool {
    true
}

impl InsightEvent {
    /// A domain event now, attributed to the current [`ORIGIN`] (else the daemon).
    pub fn new(kind: impl Into<String>) -> Self {
        let origin = current_origin().unwrap_or_else(Origin::daemon);
        Self {
            ts: now_ms(),
            kind: kind.into(),
            actor: origin.actor,
            device: origin.device,
            client_id: origin.client_id,
            host: None,
            agent_id: None,
            project_id: None,
            card_id: None,
            ok: true,
            ms: None,
            meta: Map::new(),
            reconstructed: false,
        }
    }

    pub fn agent(mut self, id: impl Into<String>) -> Self {
        self.agent_id = Some(id.into());
        self
    }

    pub fn project(mut self, id: Option<impl Into<String>>) -> Self {
        self.project_id = id.map(Into::into);
        self
    }

    pub fn card(mut self, id: impl Into<String>) -> Self {
        self.card_id = Some(id.into());
        self
    }

    pub fn ms(mut self, ms: i64) -> Self {
        self.ms = Some(ms.max(0));
        self
    }

    pub fn meta(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.meta.insert(key.to_owned(), value.into());
        self
    }
}

/// Identity of an agent at one point in time, for the agent ledger (agents stay in the usage
/// statistics after they are deleted).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub ts: i64,
    pub agent_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub cwd: String,
    /// Worktree path, when the agent works in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
}

/// Receiver of domain events and agent identity changes (implemented by `coocoo-core`).
/// Both calls are cheap and never block on I/O.
pub trait InsightsSink: Send + Sync {
    fn event(&self, event: InsightEvent);
    /// An agent was created, renamed, deleted, or its session file changed.
    fn agent(&self, entry: LedgerEntry);
}

/// Content-free name of a slash command (`/review foo` → `review`); `None` for anything else.
pub fn slash_name(message: &str) -> Option<String> {
    let rest = message.trim_start().strip_prefix('/')?;
    let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    let valid = !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    valid.then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_names_carry_no_arguments() {
        assert_eq!(slash_name("  /review src/secret.rs please").as_deref(), Some("review"));
        assert_eq!(slash_name("/plan:new"), Some("plan:new".to_owned()));
        assert_eq!(slash_name("/etc/passwd is a file"), None);
        assert_eq!(slash_name("hello /review"), None);
    }

    #[tokio::test]
    async fn events_take_the_scoped_origin() {
        let origin = Origin { actor: Actor::User, device: Some("phone".into()), client_id: Some("c1".into()), agent_id: None };
        let event = ORIGIN.scope(origin, async { InsightEvent::new("card.move") }).await;
        assert_eq!((event.actor, event.device.as_deref()), (Actor::User, Some("phone")));
        assert_eq!(InsightEvent::new("turn.end").actor, Actor::Daemon);
    }
}
