//! `insights.*`: the usage log's recording side (actions with user intent, client events) and
//! the Stats/Workflows queries, fanned out to every online host and merged here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use coocoo_insights::{Insights, LocalContext, ProjectRef, Query, UsageReport, WorkflowPartial, WorkflowReport, workflows};
use coocoo_proto::error::{ok, params};
use coocoo_proto::insights::{Actor, InsightEvent, InsightsSink, LedgerEntry, Origin, slash_name};
use coocoo_proto::util::now_ms;
use coocoo_proto::{AgentSummary, ApiError, ApiResult};
use futures_util::future::join_all;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::app::{App, Caller};

/// Methods recorded in the usage log when a user or an agent calls them (not the daemon's own
/// housekeeping, which goes through the same path).
const ACTIONS: &[&str] = &[
    "agents.create",
    "agents.prompt",
    "agents.abort",
    "agents.respond",
    "agents.setModel",
    "agents.setThinking",
    "agents.markRead",
    "agents.rename",
    "agents.hibernate",
    "agents.archive",
    "agents.delete",
    "boards.cardAdd",
    "boards.cardUpdate",
    "boards.cardStart",
    "boards.cardRefine",
    "boards.cardDone",
    "boards.cardDelete",
    "boards.cardPublish",
    "boards.update",
    "boards.sourceAdd",
    "boards.sourceUpdate",
    "boards.sourceRemove",
    "boards.sourceRefresh",
    "schedules.add",
    "schedules.update",
    "schedules.remove",
    "schedules.run",
    // Only ids are recorded (never `value`, never the name): see `Action::begin`.
    "secrets.set",
    "secrets.remove",
    "secrets.provide",
    "git.commit",
    "git.merge",
    "git.push",
    "git.fetch",
    "git.fastForward",
    "git.resetToBase",
    "git.prCreate",
    "git.prMerge",
    "git.worktreeCreate",
    "git.worktreeRemove",
    "browser.start",
    "browser.open",
    "browser.navigate",
    "browser.watch",
    "browser.input",
    "terminal.open",
    "actions.run",
    "projects.add",
    "projects.remove",
    "hosts.add",
    "hosts.remove",
    "insights.analyze",
];

/// `browser.input` (every click and key) is recorded at most this often per client and profile.
const INPUT_EVERY_MS: i64 = 10 * 60_000;
/// Client events accepted per `insights.track` call.
const MAX_TRACK: usize = 500;
/// How long a peer may take for its part of a query (it may sync omp's statistics first).
const PEER_QUERY: Duration = Duration::from_secs(80);

/// Normalized device name from a client.
pub(crate) fn device(raw: &str) -> &'static str {
    match raw {
        "desktop" => "desktop",
        "phone" => "phone",
        _ => "browser",
    }
}

/// An action being handled; recorded with its outcome by [`Action::finish`].
pub(crate) struct Action {
    event: InsightEvent,
    started: Instant,
}

impl Action {
    /// `None` when `method` is not recorded (or the log is off).
    pub fn begin(app: &App, method: &str, p: &Value, origin: Origin) -> Option<Action> {
        if !ACTIONS.contains(&method) || !app.insights.log_enabled() {
            return None;
        }
        let s = |key: &str| p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
        let mut event = InsightEvent {
            actor: origin.actor,
            device: origin.device,
            client_id: origin.client_id,
            ..InsightEvent::new(method)
        };
        if let Some(by) = origin.agent_id {
            event.meta.insert("by".into(), by.into());
        }
        event.agent_id = if method.starts_with("agents.") || method == "secrets.provide" { s("id") } else { s("agentId") };
        event.project_id = s("projectId");
        event.card_id = s("cardId");
        let meta = &mut event.meta;
        match method {
            "agents.create" => {
                let worktree = p.get("worktree").is_some_and(|w| w.as_bool() != Some(false) && !w.is_null());
                meta.insert("worktree".into(), worktree.into());
                if let Some(model) = s("model") {
                    meta.insert("model".into(), model.into());
                }
            }
            "agents.prompt" => {
                let message = p.get("message").and_then(Value::as_str).unwrap_or("");
                meta.insert("mode".into(), s("mode").unwrap_or_else(|| "auto".into()).into());
                meta.insert("chars".into(), message.chars().count().into());
                meta.insert("images".into(), p.get("images").and_then(Value::as_array).map_or(0, Vec::len).into());
                if let Some(name) = slash_name(message) {
                    meta.insert("slash".into(), name.into());
                }
            }
            "agents.setModel" => {
                if let Some(model) = s("model") {
                    meta.insert("model".into(), model.into());
                }
            }
            "agents.setThinking" => {
                if let Some(level) = s("level") {
                    meta.insert("level".into(), level.into());
                }
            }
            "boards.cardAdd" | "boards.cardUpdate" => {
                if let Some(column) = s("column") {
                    meta.insert("column".into(), column.into());
                }
            }
            "boards.cardStart" | "boards.cardRefine" => {
                meta.insert("worktree".into(), p.get("worktree").and_then(Value::as_bool).unwrap_or(false).into());
            }
            "browser.start" | "browser.open" | "browser.navigate" | "browser.watch" | "browser.input" => {
                // A browser profile is named after its project.
                event.project_id = s("profile").filter(|p| p != "default");
                if method == "browser.input" {
                    let key = format!("{}:{}", event.client_id.as_deref().unwrap_or("-"), s("profile").unwrap_or_default());
                    let now = now_ms();
                    let mut seen = app.input_logged.lock();
                    if seen.get(&key).is_some_and(|at| now - at < INPUT_EVERY_MS) {
                        return None;
                    }
                    seen.insert(key, now);
                }
            }
            "insights.analyze" | "projects.remove" => {
                event.project_id = s("projectId").or_else(|| s("id"));
            }
            _ => {}
        }
        if event.project_id.is_none()
            && let Some(id) = &event.agent_id
        {
            event.project_id = app.agents.summary(id).and_then(|a| a.project_id);
        }
        Some(Action { event, started: Instant::now() })
    }

    pub fn finish(mut self, app: &App, result: &ApiResult<Value>) {
        self.event.ok = result.is_ok();
        self.event.ms = Some(self.started.elapsed().as_millis() as i64);
        if let Ok(value) = result {
            let s = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
            match self.event.kind.as_str() {
                "agents.create" => {
                    self.event.agent_id = s("id");
                    self.event.project_id = s("projectId").or(self.event.project_id.take());
                }
                "boards.cardStart" | "boards.cardRefine" => self.event.agent_id = s("agentId"),
                "boards.cardDone" => {
                    let committed = value.get("commit").is_some_and(|c| !c.is_null());
                    self.event.meta.insert("commit".into(), committed.into());
                }
                "projects.add" => self.event.project_id = s("id"),
                "boards.cardAdd" => self.event.card_id = s("id"),
                "insights.analyze" => self.event.agent_id = s("id"),
                _ => {}
            }
        }
        app.insights.record(self.event);
    }
}

/// Domain events and agent identity changes from the subsystems.
pub(crate) struct Sink(pub Arc<Insights>);

impl InsightsSink for Sink {
    fn event(&self, event: InsightEvent) {
        self.0.record(event);
    }

    fn agent(&self, entry: LedgerEntry) {
        self.0.ledger(entry);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackParams {
    client_id: String,
    device: String,
    events: Vec<TrackEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackEvent {
    ts: i64,
    kind: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    card_id: Option<String>,
    #[serde(default)]
    meta: Map<String, Value>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SettingsParams {
    enabled: Option<bool>,
    retention_days: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsParams {
    #[serde(flatten)]
    query: Query,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeParams {
    project_id: String,
    from: i64,
    to: i64,
    #[serde(default)]
    tz_offset: i32,
    focus: Option<String>,
}

/// Short `<value>` for client meta: ids and names only, never long text.
fn clean_meta(meta: Map<String, Value>) -> Map<String, Value> {
    meta.into_iter()
        .take(8)
        .filter_map(|(k, v)| {
            let v = match v {
                Value::String(s) => Value::String(s.chars().take(64).collect()),
                v @ (Value::Bool(_) | Value::Number(_)) => v,
                _ => return None,
            };
            Some((k.chars().take(32).collect(), v))
        })
        .collect()
}

impl App {
    pub(crate) async fn insights_method(self: &Arc<Self>, caller: &Caller, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "insights.usage" => {
                let q: Query = params(p)?;
                ok(self.usage(q).await)
            }
            "insights.workflows" => {
                let q: Query = params(p)?;
                if q.local_only {
                    return ok(self.insights.workflow_partial(q.clone(), self.local_context().await, false).await);
                }
                ok(self.workflows(q).await)
            }
            "insights.events" => {
                let EventsParams { query, limit } = params(p)?;
                ok(self.events(query, limit.unwrap_or(10_000).clamp(1, 200_000)).await)
            }
            "insights.track" => {
                let TrackParams { client_id, device: raw, events } = params(p)?;
                let origin = caller.origin.clone().unwrap_or_else(Origin::daemon);
                let actor = if origin.actor == Actor::Agent { Actor::Agent } else { Actor::User };
                for e in events.into_iter().take(MAX_TRACK).filter(|e| e.kind.starts_with("ui.") && e.kind.len() <= 48) {
                    self.insights.record(InsightEvent {
                        ts: e.ts,
                        kind: e.kind,
                        actor,
                        device: Some(device(&raw).to_owned()),
                        client_id: Some(client_id.chars().take(32).collect()),
                        host: None,
                        agent_id: e.agent_id,
                        project_id: e.project_id,
                        card_id: e.card_id,
                        ok: true,
                        ms: None,
                        meta: clean_meta(e.meta),
                        reconstructed: false,
                    });
                }
                ok(json!({}))
            }
            "insights.settings" => {
                let p: SettingsParams = if p.is_null() { SettingsParams::default() } else { params(p)? };
                let config = {
                    let mut config = self.config.lock();
                    if let Some(enabled) = p.enabled {
                        config.insights.enabled = enabled;
                    }
                    if let Some(days) = p.retention_days {
                        config.insights.retention_days = days;
                    }
                    config.insights.clone()
                };
                if p.enabled.is_some() || p.retention_days.is_some() {
                    self.insights.configure(&config);
                    self.save_config();
                }
                ok(self.insights.settings(config.retention_days))
            }
            "insights.clear" => {
                let insights = self.insights.clone();
                tokio::task::spawn_blocking(move || insights.clear()).await.map_err(ApiError::internal)??;
                ok(self.insights.settings(self.config.lock().insights.retention_days))
            }
            "insights.analyze" => ok(self.analyze(params(p)?).await?),
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    async fn local_context(&self) -> LocalContext {
        LocalContext {
            host_id: self.host_id.clone(),
            host_name: self.config.lock().name.clone(),
            projects: self.workspace.projects().into_iter().map(|p| ProjectRef { id: p.id, name: p.name, path: p.path }).collect(),
            agents: self.agents.summaries(),
        }
    }

    /// Peers a query goes to: the online ones among `hosts` (absent = all), and the offline ones
    /// (reported as missing).
    fn targets(&self, hosts: Option<&[String]>) -> (bool, Vec<Arc<crate::peer::PeerLink>>) {
        let wanted = |id: &str| hosts.is_none_or(|h| h.iter().any(|x| x == id));
        let peers = self.peers.read().iter().filter(|p| wanted(&p.id())).cloned().collect();
        (wanted(&self.host_id), peers)
    }

    /// Ask every wanted peer for its local part; failures become a host source with an error.
    async fn fan_out<T: DeserializeOwned>(&self, method: &str, q: &Query) -> Vec<Result<T, coocoo_insights::HostSource>> {
        let (_, peers) = self.targets(q.hosts.as_deref());
        let mut local_q = q.clone();
        local_q.local_only = true;
        let params = serde_json::to_value(&local_q).unwrap_or(Value::Null);
        join_all(peers.into_iter().map(|peer| {
            let params = params.clone();
            async move {
                let info = peer.info();
                let missing = |error: String| coocoo_insights::HostSource { host_id: info.id.clone(), name: info.name.clone(), ok: false, error: Some(error), ..Default::default() };
                if !info.online {
                    return Err(missing("offline".to_owned()));
                }
                match tokio::time::timeout(PEER_QUERY, peer.request(method, params)).await {
                    Ok(Ok(raw)) => serde_json::from_str::<T>(raw.get()).map_err(|e| missing(format!("unreadable answer: {e}"))),
                    Ok(Err(e)) if e.code == "unknown_method" => Err(missing("its Coocoo is too old for statistics; update it".to_owned())),
                    Ok(Err(e)) => Err(missing(e.message)),
                    Err(_) => Err(missing(format!("no answer within {} s", PEER_QUERY.as_secs()))),
                }
            }
        }))
        .await
    }

    async fn usage(self: &Arc<Self>, q: Query) -> UsageReport {
        let local = async {
            let (me, _) = self.targets(q.hosts.as_deref());
            if me || q.local_only { Some(self.insights.usage(q.clone(), self.local_context().await, false).await) } else { None }
        };
        if q.local_only {
            return local.await.unwrap_or_default();
        }
        let (local, remote) = tokio::join!(local, self.fan_out::<UsageReport>("insights.usage", &q));
        let mut report = UsageReport { query: Some(q.clone()), prev: q.compare.then(Default::default), ..UsageReport::default() };
        for part in local.into_iter().map(Ok).chain(remote) {
            match part {
                Ok(part) => report.merge(part),
                Err(source) => report.hosts.push(source),
            }
        }
        report.finish();
        report
    }

    async fn workflows(self: &Arc<Self>, q: Query) -> WorkflowReport {
        let local = async {
            let (me, _) = self.targets(q.hosts.as_deref());
            if me { Some(self.insights.workflow_partial(q.clone(), self.local_context().await, false).await) } else { None }
        };
        let (local, remote) = tokio::join!(local, self.fan_out::<WorkflowPartial>("insights.workflows", &q));
        let mut partials: Vec<WorkflowPartial> = local.into_iter().collect();
        let mut missing = Vec::new();
        for part in remote {
            match part {
                Ok(part) => partials.push(part),
                Err(source) => missing.push(source),
            }
        }
        let q2 = q.clone();
        let mut report = tokio::task::spawn_blocking(move || workflows::mine(&q2, partials)).await.unwrap_or_default();
        report.hosts.extend(missing);
        report.query = Some(q);
        report
    }

    async fn events(self: &Arc<Self>, q: Query, limit: usize) -> Value {
        #[derive(Deserialize, Default)]
        struct Part {
            events: Vec<InsightEvent>,
            truncated: bool,
        }
        let (me, _) = self.targets(q.hosts.as_deref());
        let local = if me || q.local_only {
            let (mut events, truncated) = self.insights.events(q.from, q.to, limit);
            for e in &mut events {
                e.host = Some(self.host_id.clone());
            }
            Some(Part { events, truncated })
        } else {
            None
        };
        if q.local_only {
            let part = local.unwrap_or_default();
            return json!({ "events": part.events, "truncated": part.truncated });
        }
        let mut hosts = Vec::new();
        if local.is_some() {
            hosts.push(coocoo_insights::HostSource {
                host_id: self.host_id.clone(),
                name: self.config.lock().name.clone(),
                ok: true,
                log_since: self.insights.settings(0).log_since,
                log_enabled: self.insights.log_enabled(),
                ..Default::default()
            });
        }
        let mut all: Vec<InsightEvent> = Vec::new();
        let mut truncated = false;
        for part in local.into_iter().map(Ok).chain(self.fan_out::<Part>("insights.events", &q).await) {
            match part {
                Ok(part) => {
                    truncated |= part.truncated;
                    all.extend(part.events);
                }
                Err(source) => hosts.push(source),
            }
        }
        all.sort_by_key(|e| e.ts);
        if all.len() > limit {
            truncated = true;
            all.drain(..all.len() - limit);
        }
        json!({ "events": all, "truncated": truncated, "hosts": hosts })
    }

    /// Start the pattern-analysis agent in a project of this host.
    async fn analyze(self: &Arc<Self>, p: AnalyzeParams) -> ApiResult<AgentSummary> {
        let project = self.workspace.projects().into_iter().find(|x| x.id == p.project_id).ok_or_else(|| ApiError::not_found(format_args!("project {}", p.project_id)))?;
        let prompt = analyze_prompt(&project.name, p.from, p.to, p.tz_offset, p.focus.as_deref());
        let summary = self
            .agents
            .dispatch("agents.create", json!({ "projectId": project.id, "title": "Analyze usage patterns", "prompt": prompt }))
            .await?;
        let summary: AgentSummary = serde_json::from_value(summary)?;
        self.insights.set_analyze(coocoo_insights::AnalyzeRecord { project_id: project.id, agent_id: summary.id.clone(), at: now_ms() });
        Ok(summary)
    }
}

/// The fixed prompt of "Analyze patterns".
fn analyze_prompt(project: &str, from: i64, to: i64, tz_offset: i32, focus: Option<&str>) -> String {
    let day = |ts: i64| coocoo_insights::time::date(coocoo_insights::time::day_index(ts, tz_offset));
    let (first, last) = (day(from.max(0)), day((to - 1).max(0)));
    let mut text = format!(
        "Analyze how Coocoo is used and turn the strongest patterns into improvement suggestions for Coocoo.\n\n\
         Period: {first} to {last} (UTC dates for the tool: from={first}, to={last}).\n\n\
         1. Read the usage insights with the `coocoo_insights` tool: sections `workflows`, `friction`, `sequences`, `features` and `usage` \
         (and `events` when you need to look at a concrete case). They are the numbers of the Stats and Workflows tabs; example references \
         name the host, agent and time.\n\
         2. Compare what you find with the code of this project ({project}) — read only; change nothing, run no builds, commit nothing.\n\
         3. List the open cards with `kanban_list` so you do not suggest what is already planned.\n\
         4. For the 3–7 strongest patterns or friction points, add one card each to the inbox with `kanban_add` (column `inbox`). Each card: \
         a short title naming the improvement; a body with the observation and its numbers (period, counts, rates, trend), 1–3 example \
         agents as `host`/`agent id`/time, your best guess at the cause (with the code you looked at) and a concrete proposal.\n\
         5. Do not implement anything. End with a short summary of the cards you added."
    );
    if let Some(focus) = focus.map(str::trim).filter(|f| !f.is_empty()) {
        text.push_str(&format!("\n\nFocus especially on: {focus}"));
    }
    text
}
