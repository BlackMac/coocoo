//! Host tools registered with every omp process: they let an agent see the fleet and
//! spawn/steer/await other agents on any host, routed through the [`Orchestrator`].
//!
//! Handlers run on omp-rpc's handler threads (never on the async runtime), so blocking on the
//! runtime handle is safe there.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use coocoo_proto::insights::TOOL_AGENT;
use coocoo_proto::util::now_ms;
use coocoo_proto::{ApiError, DoneAction, Emitter, Orchestrator, events};
use omp_rpc::{HostTool, HostToolContext, HostToolError, HostToolOutput, ToolLoadMode};
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;
use tokio::sync::broadcast::error::RecvError;

use crate::Agents;

/// Upper bound for `coocoo_wait.timeoutSec`.
const MAX_WAIT_SECS: u64 = 3600;
/// Re-check interval while waiting (covers peers whose events are not relayed to this host).
const WAIT_RECHECK: Duration = Duration::from_secs(5);
const DONE_STATUSES: &[&str] = &["idle", "error", "hibernated", "stopped"];
const DAY_MS: i64 = 86_400_000;
/// Longest period `coocoo_insights` accepts, in days.
const MAX_INSIGHT_DAYS: u64 = 3660;
/// Agents `coocoo_insights` lists in the usage section, most expensive first.
const TOP_AGENTS: usize = 20;
const FRUSTRATION_EXAMPLES: usize = 10;
const DEFAULT_EVENTS: u64 = 200;
const MAX_EVENTS: u64 = 2000;

struct ToolEnv {
    agents: Weak<Agents>,
    orchestrator: Arc<dyn Orchestrator>,
    emitter: Emitter,
    handle: Handle,
    /// The agent whose process owns these tools.
    agent_id: String,
}

type ToolResult = Result<HostToolOutput, HostToolError>;

fn err(message: impl Into<String>) -> HostToolError {
    Box::new(ApiError::invalid(message.into()))
}

fn schema(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn text(value: &Value) -> HostToolOutput {
    HostToolOutput::Text(serde_json::to_string_pretty(value).unwrap_or_default())
}

fn str_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn required(args: &Map<String, Value>, key: &str) -> Result<String, HostToolError> {
    str_arg(args, key).ok_or_else(|| err(format!("`{key}` is required")))
}

impl ToolEnv {
    fn call(&self, host: Option<String>, method: &str, params: Value) -> Result<Value, HostToolError> {
        let orchestrator = self.orchestrator.clone();
        let method = method.to_owned();
        // The daemon logs the call as this agent's action.
        let call = TOOL_AGENT.scope(self.agent_id.clone(), async move { orchestrator.call(host, method, params).await });
        self.handle.block_on(call).map_err(|e| Box::new(e) as HostToolError)
    }

    /// `None` = this daemon.
    fn host(&self, args: &Map<String, Value>) -> Option<String> {
        str_arg(args, "host").filter(|h| *h != self.orchestrator.local_host_id())
    }

    fn hosts(&self) -> ToolResult {
        let hosts = self.call(None, "hosts.list", Value::Null)?;
        let local = self.orchestrator.local_host_id();
        let list: Vec<Value> = hosts
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| {
                json!({
                    "id": h["id"],
                    "name": h["name"],
                    "online": h["online"],
                    "kind": h["kind"],
                    "os": h.get("os"),
                    "thisHost": h["id"].as_str() == Some(local.as_str()),
                })
            })
            .collect();
        Ok(text(&Value::Array(list)))
    }

    fn projects(&self, args: Map<String, Value>) -> ToolResult {
        let host = self.host(&args);
        let projects = self.call(host.clone(), "projects.list", Value::Null)?;
        let list: Vec<Value> = projects
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                json!({
                    "id": p["id"],
                    "host": p["hostId"],
                    "name": p["name"],
                    "path": p["path"],
                    "branch": p.pointer("/git/branch"),
                    "changedFiles": p.pointer("/git/changed"),
                })
            })
            .collect();
        Ok(text(&Value::Array(list)))
    }

    fn agents(&self, args: Map<String, Value>) -> ToolResult {
        let host = self.host(&args);
        let mut params = json!({});
        if let Some(project) = str_arg(&args, "projectId") {
            params["projectId"] = Value::String(project);
        }
        let agents = self.call(host, "agents.list", params)?;
        let list: Vec<Value> = agents
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                json!({
                    "id": a["id"],
                    "host": a["hostId"],
                    "title": a["title"],
                    "status": a["status"],
                    "projectId": a.get("projectId"),
                    "cwd": a["cwd"],
                    "branch": a.pointer("/worktree/branch"),
                    "parentId": a.get("parentId"),
                    "preview": a.get("preview"),
                    "self": a["id"].as_str() == Some(self.agent_id.as_str()),
                })
            })
            .collect();
        Ok(text(&Value::Array(list)))
    }

    fn spawn(&self, args: Map<String, Value>) -> ToolResult {
        let host = self.host(&args);
        let prompt = required(&args, "prompt")?;
        let mut params = json!({ "prompt": prompt, "parentId": self.agent_id });
        let project = str_arg(&args, "projectId");
        let cwd = str_arg(&args, "cwd");
        match (project, cwd) {
            (None, None) => {
                // Default to the caller's own project/directory on this host.
                if host.is_some() {
                    return Err(err("`projectId` or `cwd` is required when spawning on another host"));
                }
                let agents = self.agents.upgrade().ok_or_else(|| err("daemon is shutting down"))?;
                let me = agents.get(&self.agent_id).map_err(|e| Box::new(e) as HostToolError)?.summary();
                match me.project_id {
                    Some(project) => params["projectId"] = Value::String(project),
                    None => params["cwd"] = Value::String(me.cwd),
                }
            }
            (project, cwd) => {
                if let Some(project) = project {
                    params["projectId"] = Value::String(project);
                }
                if let Some(cwd) = cwd {
                    params["cwd"] = Value::String(cwd);
                }
            }
        }
        for key in ["title", "model"] {
            if let Some(value) = str_arg(&args, key) {
                params[key] = Value::String(value);
            }
        }
        if let Some(worktree) = args.get("worktree").and_then(Value::as_bool) {
            params["worktree"] = Value::Bool(worktree);
        }
        let summary = self.call(host, "agents.create", params)?;
        Ok(text(&json!({
            "agentId": summary["id"],
            "host": summary["hostId"],
            "title": summary["title"],
            "status": summary["status"],
            "cwd": summary["cwd"],
            "branch": summary.pointer("/worktree/branch"),
            "next": "The agent is working on the prompt. Call coocoo_wait with this agentId (and host) to get its answer.",
        })))
    }

    fn send(&self, args: Map<String, Value>) -> ToolResult {
        let host = self.host(&args);
        let id = required(&args, "agentId")?;
        let message = required(&args, "message")?;
        if host.is_none() && id == self.agent_id {
            return Err(err("an agent cannot message itself"));
        }
        self.call(host, "agents.prompt", json!({ "id": id, "message": message, "mode": "auto" }))?;
        Ok(HostToolOutput::Text(format!(
            "Message delivered to agent {id}. Call coocoo_wait to get its answer."
        )))
    }

    fn output(&self, args: Map<String, Value>) -> ToolResult {
        let host = self.host(&args);
        let id = required(&args, "agentId")?;
        let output = self.call(host, "agents.output", json!({ "id": id }))?;
        Ok(text(&json!({ "agentId": id, "status": output["status"], "text": output.get("text") })))
    }

    fn wait(&self, args: Map<String, Value>, ctx: &HostToolContext) -> ToolResult {
        let host = self.host(&args);
        let id = required(&args, "agentId")?;
        if host.is_none() && id == self.agent_id {
            return Err(err("an agent cannot wait for itself"));
        }
        let timeout = args.get("timeoutSec").and_then(Value::as_u64).unwrap_or(600).clamp(1, MAX_WAIT_SECS);
        let deadline = Instant::now() + Duration::from_secs(timeout);
        // Subscribe before the first check so no update in between is missed.
        let mut updates = self.emitter.subscribe();
        let needle = format!("\"id\":\"{id}\"");
        let orchestrator = self.orchestrator.clone();
        let result: Result<(Value, bool), ApiError> = self.handle.block_on(async {
            loop {
                let output = orchestrator.call(host.clone(), "agents.output".to_owned(), json!({ "id": id })).await?;
                let status = output["status"].as_str().unwrap_or_default();
                if DONE_STATUSES.contains(&status) {
                    return Ok((output, false));
                }
                let now = Instant::now();
                if now >= deadline {
                    return Ok((output, true));
                }
                if ctx.is_cancelled() {
                    return Err(ApiError::conflict("cancelled"));
                }
                let pause = (deadline - now).min(WAIT_RECHECK);
                let _ = tokio::time::timeout(pause, async {
                    loop {
                        match updates.recv().await {
                            Ok(event) if event.name == events::AGENT_UPDATED && event.json.contains(&needle) => return,
                            Ok(_) => {}
                            Err(RecvError::Lagged(_)) => return,
                            Err(RecvError::Closed) => std::future::pending::<()>().await,
                        }
                    }
                })
                .await;
            }
        });
        let (output, timed_out) = result.map_err(|e| Box::new(e) as HostToolError)?;
        let mut report = json!({ "agentId": id, "status": output["status"], "text": output.get("text") });
        if timed_out {
            report["timedOut"] = Value::Bool(true);
            report["note"] = Value::String(format!(
                "Still {} after {timeout}s; call coocoo_wait again or check with coocoo_output.",
                output["status"].as_str().unwrap_or("busy")
            ));
        }
        Ok(text(&report))
    }

    // ─── Kanban board (this agent's project) ──────────────────────────────────────────────

    /// The project whose board these tools work on.
    fn project_id(&self) -> Result<String, HostToolError> {
        let agent = self.call(None, "agents.get", json!({ "id": self.agent_id }))?;
        agent
            .pointer("/summary/projectId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| err("you are not working in a Coocoo project, so there is no Kanban board"))
    }

    fn kanban_list(&self) -> ToolResult {
        let board = self.call(None, "boards.get", json!({ "projectId": self.project_id()? }))?;
        let agents = self.agents.upgrade();
        let mut columns = Map::new();
        for column in COLUMNS {
            let cards: Vec<Value> = board["cards"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["column"] == *column)
                .map(|c| {
                    let mut card = json!({ "id": c["id"], "title": c["title"], "body": c.get("body"), "url": c.get("url") });
                    if let Some(agent) = c.get("agentId").and_then(Value::as_str) {
                        card["agent"] = if agent == self.agent_id { json!("you") } else { c.get("agentTitle").cloned().unwrap_or(json!(agent)) };
                    }
                    if let Some(by) = c.pointer("/addedBy/id").and_then(Value::as_str) {
                        card["addedBy"] = if by == self.agent_id { json!("you") } else { c.pointer("/addedBy/title").cloned().unwrap_or(json!(by)) };
                    }
                    if c.get("refining") == Some(&Value::Bool(true)) {
                        card["refining"] = json!(true);
                    }
                    if c.get("refinement").is_some_and(|r| !r.is_null()) {
                        card["refined"] = json!(true);
                    }
                    let images: Vec<String> = c["images"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|i| i["name"].as_str())
                        .filter_map(|name| Some(agents.as_ref()?.card_image_path(c["id"].as_str()?, name).display().to_string()))
                        .collect();
                    if !images.is_empty() {
                        card["images"] = json!(images);
                    }
                    card
                })
                .collect();
            columns.insert((*column).to_owned(), Value::Array(cards));
        }
        Ok(text(&Value::Object(columns)))
    }

    fn kanban_add(&self, args: Map<String, Value>) -> ToolResult {
        let mut params = json!({ "projectId": self.project_id()?, "title": required(&args, "title")?, "agentId": self.agent_id });
        for key in ["body", "column"] {
            if let Some(value) = str_arg(&args, key) {
                params[key] = Value::String(value);
            }
        }
        // The inbox lists the newest suggestion first, like importer entries.
        if params["column"] == "inbox" {
            params["top"] = json!(true);
        }
        let card = self.call(None, "boards.cardAdd", params)?;
        Ok(text(&json!({ "id": card["id"], "column": card["column"] })))
    }

    fn kanban_move(&self, args: Map<String, Value>) -> ToolResult {
        let mut params = json!({ "projectId": self.project_id()?, "cardId": required(&args, "cardId")?, "column": required(&args, "column")? });
        if let Some(index) = args.get("index").and_then(Value::as_u64) {
            params["index"] = json!(index);
        }
        let card = self.call(None, "boards.cardUpdate", params)?;
        Ok(text(&json!({ "id": card["id"], "column": card["column"] })))
    }

    fn kanban_done(&self, args: Map<String, Value>) -> ToolResult {
        let mut params = json!({ "projectId": self.project_id()?, "cardId": required(&args, "cardId")?, "agentId": self.agent_id });
        for key in ["commitMessage", "note", "prTitle", "prBody"] {
            if let Some(value) = str_arg(&args, key) {
                params[key] = Value::String(value);
            }
        }
        let done = self.call(None, "boards.cardDone", params)?;
        let mut report = json!({ "column": "done", "commit": done.get("commit"), "commitSkipped": done.get("commitSkipped") });
        for key in ["pushed", "pullRequest", "gitError"] {
            if let Some(value) = done.get(key).filter(|v| !v.is_null()) {
                report[key] = value.clone();
            }
        }
        Ok(text(&report))
    }

    fn kanban_refine(&self, args: Map<String, Value>) -> ToolResult {
        let attachments: Vec<Value> = args.get("attachments").and_then(Value::as_array).cloned().unwrap_or_default();
        let params = json!({
            "projectId": self.project_id()?,
            "cardId": required(&args, "cardId")?,
            "agentId": self.agent_id,
            "text": required(&args, "refinement")?,
            "attachments": attachments,
        });
        let card = self.call(None, "boards.refinementSave", params)?;
        let names: Vec<&Value> = card.pointer("/refinement/attachments").and_then(Value::as_array).into_iter().flatten().map(|a| &a["name"]).collect();
        Ok(text(&json!({ "cardId": card["id"], "column": card["column"], "attachments": names })))
    }

    /// An importer as the agent sees it; `script` only on request (it can be long).
    fn importer_view(&self, s: &Value, with_script: bool) -> Value {
        let mut view = json!({
            "id": s["id"],
            "kind": s["kind"],
            "name": s.get("name"),
            "intervalMins": s["intervalMins"],
            "lastRun": s.get("checkedAt"),
            "error": s.get("error"),
        });
        if s["kind"] != "script" {
            view["target"] = s["target"].clone();
        } else {
            view["sandboxed"] = s.get("sandboxed").cloned().unwrap_or(Value::Null);
        }
        if let Some(agent) = s.get("agentId").and_then(Value::as_str) {
            view["by"] = if agent == self.agent_id { json!("you") } else { s.get("agentTitle").cloned().unwrap_or(json!(agent)) };
        }
        if with_script {
            view["script"] = s.get("script").cloned().unwrap_or(Value::Null);
        }
        view
    }

    fn importers(&self, args: Map<String, Value>) -> ToolResult {
        let board = self.call(None, "boards.get", json!({ "projectId": self.project_id()? }))?;
        let sources = board["sources"].as_array().cloned().unwrap_or_default();
        match str_arg(&args, "id") {
            Some(id) => {
                let source = sources.iter().find(|s| s["id"] == id.as_str()).ok_or_else(|| err(format!("no importer `{id}`")))?;
                Ok(text(&self.importer_view(source, true)))
            }
            None => Ok(text(&Value::Array(sources.iter().map(|s| self.importer_view(s, false)).collect()))),
        }
    }

    fn importer_save(&self, args: Map<String, Value>) -> ToolResult {
        let project_id = self.project_id()?;
        let interval = args.get("intervalMins").and_then(Value::as_u64);
        let (script, feed) = (args.get("script").and_then(Value::as_str).filter(|s| !s.trim().is_empty()), str_arg(&args, "feedUrl"));
        if script.is_some() && feed.is_some() {
            return Err(err("give either `script` or `feedUrl`, not both"));
        }
        let run = match str_arg(&args, "id") {
            None => {
                let mut params = json!({ "projectId": project_id, "agentId": self.agent_id, "name": str_arg(&args, "name") });
                match (script, feed) {
                    (Some(script), _) => (params["kind"], params["script"]) = (json!("script"), json!(script)),
                    (_, Some(url)) => (params["kind"], params["target"]) = (json!("feed"), json!(url)),
                    _ => return Err(err("a new importer needs `script` (or `feedUrl` for an RSS/Atom feed)")),
                }
                if let Some(mins) = interval {
                    params["intervalMins"] = json!(mins);
                }
                self.call(None, "boards.sourceAdd", params)?
            }
            Some(id) => {
                let board = self.call(None, "boards.get", json!({ "projectId": project_id }))?;
                let kind = board["sources"].as_array().into_iter().flatten().find(|s| s["id"] == id.as_str()).map(|s| s["kind"].clone());
                let kind = kind.ok_or_else(|| err(format!("no importer `{id}`")))?;
                if kind == "command" && (script.is_some() || feed.is_some()) {
                    return Err(err("command importers belong to the user: you may only rename, reschedule or delete them"));
                }
                if (kind == "script" && feed.is_some()) || (kind == "feed" && script.is_some()) {
                    return Err(err(format!("importer `{id}` is a {} importer", kind.as_str().unwrap_or_default())));
                }
                let mut params = json!({ "projectId": project_id, "sourceId": id, "agentId": self.agent_id });
                let changed = [("name", str_arg(&args, "name").map(Value::from)), ("script", script.map(Value::from)), ("target", feed.clone().map(Value::from)), ("intervalMins", interval.map(Value::from))];
                let mut any = false;
                for (key, value) in changed {
                    if let Some(value) = value {
                        params[key] = value;
                        any = true;
                    }
                }
                let run = if any { Some(self.call(None, "boards.sourceUpdate", params)?) } else { None };
                match run {
                    // A new script or URL already ran.
                    Some(run) if script.is_some() || feed.is_some() => run,
                    _ => self.call(None, "boards.sourceRefresh", json!({ "projectId": project_id, "sourceId": id }))?,
                }
            }
        };
        let mut report = self.importer_view(&run["source"], false);
        report["imported"] = run["imported"].clone();
        Ok(text(&report))
    }

    fn importer_delete(&self, args: Map<String, Value>) -> ToolResult {
        let id = required(&args, "id")?;
        let project_id = self.project_id()?;
        let board = self.call(None, "boards.get", json!({ "projectId": project_id }))?;
        if !board["sources"].as_array().into_iter().flatten().any(|s| s["id"] == id.as_str()) {
            return Err(err(format!("no importer `{id}`")));
        }
        self.call(None, "boards.sourceRemove", json!({ "projectId": project_id, "sourceId": id }))?;
        Ok(text(&json!({ "deleted": id })))
    }

    // ─── Project actions (read-only) ──────────────────────────────────────────────────────

    fn actions(&self, args: Map<String, Value>) -> ToolResult {
        let agent = self.call(None, "agents.get", json!({ "id": self.agent_id }))?;
        let own = agent.pointer("/summary/projectId").and_then(Value::as_str).map(str::to_owned);
        let id = str_arg(&args, "projectId")
            .or(own)
            .ok_or_else(|| err("you are not working in a Coocoo project; pass `projectId` (see coocoo_projects)"))?;
        let project = self.call(self.host(&args), "projects.get", json!({ "id": id }))?;
        let path = project["path"].as_str().unwrap_or_default();
        let actions: Vec<Value> = project["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let mut view = json!({ "id": a["id"], "name": a["name"], "kind": a["kind"], "scope": a["scope"] });
                for key in ["command", "cwd", "prompt", "worktree", "url", "target", "icon", "default", "confirm"] {
                    if let Some(value) = a.get(key) {
                        view[key] = value.clone();
                    }
                }
                view
            })
            .collect();
        let mut report = json!({
            "file": format!("{path}/.coocoo/actions.toml"),
            "actions": actions,
            "errors": project.get("actionErrors").cloned().unwrap_or(json!([])),
        });
        let cwd = agent.pointer("/summary/cwd").and_then(Value::as_str).unwrap_or_default();
        if !cwd.is_empty() && cwd != path {
            report["note"] = json!(format!(
                "You work in {cwd}. Only the file in the project's main checkout ({path}) counts: edits to .coocoo/actions.toml in your directory show up once they reach that checkout (e.g. after the merge)."
            ));
        }
        Ok(text(&report))
    }

    // ─── Usage statistics (read-only) ─────────────────────────────────────────────────────

    /// `coocoo_insights`: the Stats/Workflows numbers of every host, asked of this daemon (it
    /// gathers the peers' parts itself), boiled down to what an agent can read.
    fn insights(&self, args: Map<String, Value>) -> ToolResult {
        let section = required(&args, "section")?;
        let (from, to) = insights_period(&args)?;
        let mut query = json!({ "from": from, "to": to, "tzOffset": 0 });
        if let Some(project) = str_arg(&args, "project") {
            query["project"] = Value::String(project);
        }
        if let Some(host) = str_arg(&args, "host") {
            query["hosts"] = json!([host]);
        }
        let mut report = match section.as_str() {
            "usage" => usage_digest(&self.call(None, "insights.usage", query)?),
            "workflows" | "friction" | "features" | "sequences" => workflow_digest(&self.call(None, "insights.workflows", query)?, &section),
            "events" => {
                query["limit"] = json!(args.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_EVENTS).clamp(1, MAX_EVENTS));
                self.call(None, "insights.events", query)?
            }
            other => return Err(err(format!("unknown section `{other}` (usage, workflows, friction, features, sequences, events)"))),
        };
        if let Some(map) = report.as_object_mut() {
            map.insert("period".to_owned(), json!({ "from": from, "to": to, "unit": "Unix ms, UTC, `to` exclusive" }));
        }
        Ok(text(&report))
    }
}

/// `[from, to)` in Unix ms from `coocoo_insights`' `from`/`to` (UTC days, `to` inclusive) and
/// `days` (counted back from `to`, default today).
fn insights_period(args: &Map<String, Value>) -> Result<(i64, i64), HostToolError> {
    let day = |key: &str| -> Result<Option<i64>, HostToolError> {
        str_arg(args, key).map(|d| parse_date(&d).ok_or_else(|| err(format!("`{key}` must be a date like 2026-01-31")))).transpose()
    };
    let to = match day("to")? {
        Some(start) => start + DAY_MS,
        None => now_ms(),
    };
    let from = match day("from")? {
        Some(start) => start,
        None => {
            let days = args.get("days").and_then(Value::as_u64).unwrap_or(30).clamp(1, MAX_INSIGHT_DAYS) as i64;
            (to - 1).div_euclid(DAY_MS) * DAY_MS - (days - 1) * DAY_MS
        }
    };
    if from >= to {
        return Err(err("`from` must not be after `to`"));
    }
    Ok((from, to))
}

/// Unix ms of UTC midnight starting `YYYY-MM-DD` (Howard Hinnant's days-from-civil).
fn parse_date(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * DAY_MS)
}

/// Where each host's numbers come from, without the database paths.
fn hosts_digest(report: &Value) -> Value {
    let hosts = report["hosts"].as_array().into_iter().flatten().map(|h| {
        let mut host = Map::new();
        for key in ["hostId", "name", "ok", "error", "syncedAt", "statsError", "logSince", "logEnabled"] {
            if let Some(value) = h.get(key) {
                host.insert(key.to_owned(), value.clone());
            }
        }
        Value::Object(host)
    });
    Value::Array(hosts.collect())
}

/// `insights.usage` for an agent: totals, the breakdowns, the most expensive agents and the
/// frustration counts with a few examples.
fn usage_digest(report: &Value) -> Value {
    let totals = |totals: &Value| {
        let mut totals = totals.clone();
        if let Some(map) = totals.as_object_mut() {
            map.remove("activeDaySet");
        }
        totals
    };
    let cost = |agent: &Value| agent.pointer("/cost/total").and_then(Value::as_f64).unwrap_or(0.0);
    let mut agents: Vec<&Value> = report["agents"].as_array().into_iter().flatten().collect();
    agents.sort_by(|a, b| cost(b).total_cmp(&cost(a)));
    let agents: Vec<Value> = agents
        .into_iter()
        .take(TOP_AGENTS)
        .map(|a| {
            json!({
                "id": a["id"],
                "host": a["hostId"],
                "title": a["title"],
                "project": a.get("projectId"),
                "cost": cost(a),
                "prompts": a["prompts"],
                "requests": a["requests"],
                "commits": a["commits"].as_array().map_or(0, Vec::len),
                "cardsDone": a["cardsDone"],
                "deleted": a.get("deleted").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .collect();
    let models: Vec<Value> = report["models"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            json!({
                "model": m["model"],
                "provider": m["provider"],
                "requests": m["requests"],
                "failed": m["failed"],
                "cost": m.pointer("/cost/total"),
                "tokens": m["tokens"],
                "cacheHit": m.get("cacheHit"),
                "ttftMs": m.get("ttft"),
                "tokensPerSec": m.get("tps"),
            })
        })
        .collect();
    let frustration = &report["frustration"];
    let examples: Vec<&Value> = frustration["examples"].as_array().into_iter().flatten().take(FRUSTRATION_EXAMPLES).collect();
    json!({
        "cost": "API-equivalent USD (what the tokens would cost at API prices; subscriptions are billed differently)",
        "totals": totals(&report["totals"]),
        "prev": report.get("prev").map(totals),
        "models": models,
        "projects": report["projects"],
        "agents": agents,
        "agentCount": report["agents"].as_array().map_or(0, Vec::len),
        "tools": report["tools"],
        "frustration": {
            "prompts": frustration["prompts"],
            "flagged": frustration["flagged"],
            "signals": frustration["signals"],
            "examples": examples,
        },
        "hosts": hosts_digest(report),
    })
}

/// `insights.workflows` for an agent: everything (`workflows`) or one part of it.
fn workflow_digest(report: &Value, section: &str) -> Value {
    let keys: &[&str] = match section {
        "workflows" => &["kpis", "journey", "kanban", "shapes", "sequences", "features", "friction", "logSince"],
        "friction" => &["friction", "logSince"],
        "features" => &["features", "logSince"],
        _ => &["sequences", "logSince"],
    };
    let mut digest: Map<String, Value> = keys.iter().map(|key| ((*key).to_owned(), report.get(*key).cloned().unwrap_or(Value::Null))).collect();
    if section == "workflows" {
        digest.insert("hosts".to_owned(), hosts_digest(report));
    }
    Value::Object(digest)
}

const COLUMNS: &[&str] = &["inbox", "todo", "doing", "done"];

const HOST_PROP: &str = "Host id from coocoo_hosts; omit for this machine.";

/// The orchestration tools for agent `agent_id`; `done_action` is its board's current setting.
pub(crate) fn host_tools(agents: &Arc<Agents>, agent_id: &str, orchestrator: Arc<dyn Orchestrator>, done_action: DoneAction) -> Vec<HostTool> {
    let env = Arc::new(ToolEnv {
        agents: Arc::downgrade(agents),
        orchestrator,
        emitter: agents.emitter.clone(),
        handle: agents.handle(),
        agent_id: agent_id.to_owned(),
    });
    let tool = |name: &str, label: &str, description: &str, parameters: Value, run: fn(&ToolEnv, Map<String, Value>, &HostToolContext) -> ToolResult| {
        let env = env.clone();
        HostTool::new(name, description, schema(parameters), move |args, ctx| run(&env, args, ctx))
            .label(label)
            .load_mode(ToolLoadMode::Essential)
    };
    vec![
        tool(
            "coocoo_hosts",
            "Coocoo hosts",
            "List the machines in this Coocoo fleet (this one and linked peers, desktops and headless servers) with their online state. Use a host id with the other coocoo_* tools to work on that machine.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            |env, _, _| env.hosts(),
        ),
        tool(
            "coocoo_projects",
            "Coocoo projects",
            "List the projects (repositories) registered on a host, with id, path and current branch. Pass a project id to coocoo_spawn to start an agent in it.",
            json!({
                "type": "object",
                "properties": { "host": { "type": "string", "description": HOST_PROP } },
                "additionalProperties": false
            }),
            |env, args, _| env.projects(args),
        ),
        tool(
            "coocoo_agents",
            "Coocoo agents",
            "List the coding agents on a host with their status (starting, running, idle, waiting = blocked on the user, error, hibernated, stopped), title, working directory and a preview of their last answer. Your own entry has self=true.",
            json!({
                "type": "object",
                "properties": {
                    "host": { "type": "string", "description": HOST_PROP },
                    "projectId": { "type": "string", "description": "Only agents of this project." }
                },
                "additionalProperties": false
            }),
            |env, args, _| env.agents(args),
        ),
        tool(
            "coocoo_spawn",
            "Spawn agent",
            "Start a new, independent coding agent (a full omp session the user can watch and talk to) on this or another host, and give it a task. Returns immediately with its agentId; use coocoo_wait to collect the result. Use worktree=true for changes that must not collide with other agents (own git branch and directory). Without projectId/cwd the agent starts in your own project.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Complete, self-contained task for the new agent; it does not see your conversation." },
                    "host": { "type": "string", "description": HOST_PROP },
                    "projectId": { "type": "string", "description": "Project id from coocoo_projects." },
                    "cwd": { "type": "string", "description": "Absolute working directory on the target host (alternative to projectId)." },
                    "title": { "type": "string", "description": "Short title shown in the UI." },
                    "worktree": { "type": "boolean", "description": "Run in a fresh git worktree on its own branch (requires a project)." },
                    "model": { "type": "string", "description": "provider/modelId; default is the host's default model." }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            |env, args, _| env.spawn(args),
        ),
        tool(
            "coocoo_send",
            "Message agent",
            "Send a message to another agent: starts a new turn when it is idle (hibernated agents resume automatically) or is queued as a follow-up while it works. Use coocoo_wait afterwards for the answer.",
            json!({
                "type": "object",
                "properties": {
                    "agentId": { "type": "string" },
                    "host": { "type": "string", "description": HOST_PROP },
                    "message": { "type": "string" }
                },
                "required": ["agentId", "message"],
                "additionalProperties": false
            }),
            |env, args, _| env.send(args),
        ),
        tool(
            "coocoo_wait",
            "Wait for agent",
            "Block until another agent finished its current work (status idle, error, hibernated or stopped) or the timeout passes, then return its status and full final answer. An agent in status `waiting` needs the user and keeps this call waiting.",
            json!({
                "type": "object",
                "properties": {
                    "agentId": { "type": "string" },
                    "host": { "type": "string", "description": HOST_PROP },
                    "timeoutSec": { "type": "integer", "minimum": 1, "maximum": MAX_WAIT_SECS, "description": "Default 600." }
                },
                "required": ["agentId"],
                "additionalProperties": false
            }),
            |env, args, ctx| env.wait(args, ctx),
        ),
        tool(
            "coocoo_output",
            "Agent output",
            "Return another agent's current status and the full text of its last answer without waiting.",
            json!({
                "type": "object",
                "properties": {
                    "agentId": { "type": "string" },
                    "host": { "type": "string", "description": HOST_PROP }
                },
                "required": ["agentId"],
                "additionalProperties": false
            }),
            |env, args, _| env.output(args),
        ),
        tool(
            "kanban_list",
            "Kanban board",
            "List the cards on your project's Kanban board by column with the agent working on each (inbox: suggestions from external sources the user has not accepted yet; todo: the user's queue, top first; doing; done).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            |env, _, _| env.kanban_list(),
        ),
        tool(
            "kanban_add",
            "Add card",
            "Add a card to your project's Kanban board. Suggestions the user did not ask for (follow-ups, ideas, problems you noticed outside your task) go to the inbox (column: inbox, newest on top), where the user moves them to todo or deletes them. Without a column the card goes to the bottom of todo: use that only for work the user asked you to queue.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string", "description": "Details." },
                    "column": { "type": "string", "enum": COLUMNS }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
            |env, args, _| env.kanban_add(args),
        ),
        tool(
            "kanban_move",
            "Move card",
            "Move a card on your project's Kanban board to a column (and position, 0 = top). Use kanban_done, not this, to finish a card.",
            json!({
                "type": "object",
                "properties": {
                    "cardId": { "type": "string" },
                    "column": { "type": "string", "enum": COLUMNS },
                    "index": { "type": "integer", "minimum": 0 }
                },
                "required": ["cardId", "column"],
                "additionalProperties": false
            }),
            |env, args, _| env.kanban_move(args),
        ),
        tool(
            "kanban_done",
            "Finish card",
            &kanban_done_doc(done_action),
            json!({
                "type": "object",
                "properties": {
                    "cardId": { "type": "string" },
                    "commitMessage": { "type": "string", "description": "Describes the change; defaults to the card title." },
                    "note": { "type": "string", "description": "Short summary of what was done, shown on the card." },
                    "prTitle": { "type": "string", "description": "Pull request title, when the board opens pull requests; defaults to the card title." },
                    "prBody": { "type": "string", "description": "Pull request description (Markdown: what changed and why), when the board opens pull requests; defaults to the card, its goal and the commits." }
                },
                "required": ["cardId"],
                "additionalProperties": false
            }),
            |env, args, _| env.kanban_done(args),
        ),
        tool(
            "kanban_refine",
            "Refine card",
            "Attach your refinement to a Kanban card you were asked to refine: the plan agreed before the work starts (Markdown: goal, behaviour, approach, open questions and risks), shown in the chat and on the card, and handed to whoever implements it. Optionally attach UI mocks: paths of image (png, jpg, gif, webp, svg) or self-contained HTML files, relative to your working directory or absolute; they are copied. Calling it again replaces the refinement and its attachments.",
            json!({
                "type": "object",
                "properties": {
                    "cardId": { "type": "string" },
                    "refinement": { "type": "string", "description": "Markdown." },
                    "attachments": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "description": "File paths of mocks (images, HTML), at most 5 MiB each." }
                },
                "required": ["cardId", "refinement"],
                "additionalProperties": false
            }),
            |env, args, _| env.kanban_refine(args),
        ),
        tool(
            "kanban_importers",
            "Inbox importers",
            "List the importers that fill your project's Kanban inbox (RSS/Atom feeds, the user's commands, and script importers agents built) with schedule, last run and error; with `id`, one importer including its script.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Show this importer with its script." } },
                "additionalProperties": false
            }),
            |env, args, _| env.importers(args),
        ),
        tool(
            "kanban_importer_save",
            "Save importer",
            IMPORTER_SAVE_DOC,
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Importer to change; omit to create one." },
                    "name": { "type": "string", "description": "Label shown to the user, e.g. \"Svelte changelog\" (required for a new script)." },
                    "script": { "type": "string", "description": "Full source of a script importer." },
                    "feedUrl": { "type": "string", "description": "RSS/Atom feed URL, instead of a script, when the source already has a feed." },
                    "intervalMins": { "type": "integer", "enum": crate::board::INTERVALS, "description": "How often it runs: 15, 60 (default), 360 or 1440 minutes." }
                },
                "additionalProperties": false
            }),
            |env, args, _| env.importer_save(args),
        ),
        tool(
            "kanban_importer_delete",
            "Delete importer",
            "Delete an importer of your project's Kanban inbox (cards it imported stay; a script's state directory is removed).",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
            |env, args, _| env.importer_delete(args),
        ),
        tool(
            "coocoo_actions",
            "Project actions",
            ACTIONS_DOC,
            json!({
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "Project id from coocoo_projects; omit for your own project." },
                    "host": { "type": "string", "description": HOST_PROP }
                },
                "additionalProperties": false
            }),
            |env, args, _| env.actions(args),
        ),
        tool(
            "coocoo_insights",
            "Usage insights",
            INSIGHTS_DOC,
            json!({
                "type": "object",
                "properties": {
                    "section": { "type": "string", "enum": ["usage", "workflows", "friction", "features", "sequences", "events"] },
                    "days": { "type": "integer", "minimum": 1, "maximum": MAX_INSIGHT_DAYS, "description": "Period length in days, ending with `to` (default 30); ignored with `from`." },
                    "from": { "type": "string", "description": "First day, YYYY-MM-DD (UTC)." },
                    "to": { "type": "string", "description": "Last day, inclusive, YYYY-MM-DD (UTC); default today." },
                    "project": { "type": "string", "description": "Only this project (id from coocoo_projects)." },
                    "host": { "type": "string", "description": "Only this host (id from coocoo_hosts); default all hosts." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_EVENTS, "description": "events: at most this many (default 200)." }
                },
                "required": ["section"],
                "additionalProperties": false
            }),
            |env, args, _| env.insights(args),
        ),
    ]
}

const INSIGHTS_DOC: &str = "Usage statistics of the Coocoo agents on all hosts: the same numbers as the Stats and Workflows tabs of the Coocoo UI. Sections: \
`usage`: cost (API equivalent), tokens, requests, prompts, commits and finished cards, per model, project, agent (top 20 by cost) and tool, plus frustration signals with a few flagged prompts; \
`workflows`: KPIs, the agent journey (created → prompted → changed files → reviewed → committed → closed), Kanban flow, session shapes, frequent action sequences, feature usage and friction; \
`friction`, `features`, `sequences`: only that part of `workflows`; \
`events`: the raw usage log (kinds, ids, durations and small metadata; no prompt texts, arguments, paths or file contents). \
Examples carry hostId, agentId and ts (Unix ms), so you can cite an agent at a moment in its history. Periods are UTC days; data before a host's `logSince` is reconstructed from older records.";

/// `kanban_done` description, naming what the board does after the commit.
fn kanban_done_doc(done_action: DoneAction) -> String {
    let mut doc = "Finish a Kanban card once its work is complete: commits all changes in your working directory with the commit message when it is a git repository (skipped when there is nothing to commit), then moves the card to done. Fails without moving the card if the commit fails.".to_owned();
    if let Some(hint) = crate::board::done_hint(done_action) {
        doc.push(' ');
        doc.push_str(hint);
        doc.push_str(" If the push or pull request fails, the card is done anyway and the result reports `gitError`.");
    }
    doc
}

const IMPORTER_SAVE_DOC: &str = "Create or change an importer that fills your project's Kanban inbox on a schedule, e.g. \"take every new entry of X's changelog\". \
Usually a script: a small program whose first line picks the interpreter (`#!/usr/bin/env python3`; `sh` without one) and that prints one item per line to stdout, newest first: \
plain text (the card title) or a JSON object {\"title\", \"body\"?, \"url\"?, \"id\"?}. Each item becomes an inbox card once (deduped by id, else url, else title; the first run imports at most 20). \
It runs with a 60 s timeout in its own state directory (cwd, $HOME and $COOCOO_STATE_DIR; files there persist between runs), may read the project ($COOCOO_PROJECT_DIR) and use the network, but does not inherit your environment; \
on Linux it is sandboxed: it can read the system (/usr, /etc, …) but not the rest of the home directory, and can write only its state directory, so use system interpreters and tools (python3, curl, …). \
Saving runs it once right away and reports how many cards it imported or its error (stderr tail), so fix and save again until it works. With `id` and nothing else it just runs again.";

const ACTIONS_DOC: &str = "Show the project's UI actions: the buttons Coocoo offers under ▶ Run in the project and agent views and in ⌘K, read from `.coocoo/actions.toml` in the project's main checkout. \
Returns the file path, the parsed actions and the problems found in the file (with line). It runs nothing; to add or change actions, edit that file, then call this again to check that it parsed. \
Format (TOML; the order of the entries is the order in the menu):\n\
[[action]]\n\
name = \"Dev server\"   # required\n\
id = \"dev\"            # optional, default: slug of name; unique\n\
icon = \"server\"       # optional: play, server, test-tube, terminal, rocket, globe, bug, package, database, wrench, refresh, message, external-link\n\
scope = \"agent\"       # project | agent | both (default)\n\
default = true        # the Run button's one-click action (else the first in scope)\n\
confirm = \"Sure?\"     # optional confirmation dialog\n\
# exactly one of:\n\
run = \"npm run dev\"   # shell command in its own terminal ($SHELL -lc); env COOCOO_PROJECT_DIR, COOCOO_WORKDIR, COOCOO_ACTION\n\
cwd = \"ui\"            # with run: relative to the project or the agent's worktree, never outside\n\
prompt = \"Review the changes on {branch}.\"  # agent scope: sent to that agent; project scope: first prompt of a new agent; placeholders {branch} {project} {cwd}\n\
worktree = true       # with prompt: the new agent gets its own worktree\n\
url = \"http://localhost:5173\"  # opens in the project's browser for loopback hosts, else the system browser\n\
target = \"external\"   # with url: browser | external\n\
Scope `project` runs in the project directory, `agent` in the agent's directory (its worktree). Ports are fixed by the command, so a dev server in two worktrees collides; use foreground processes (Stop sends SIGHUP to the process group).";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insight_periods_are_utc_days() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2000-03-01"), Some(11_017 * DAY_MS));
        assert_eq!(parse_date("2024-13-01"), None);
        assert_eq!(parse_date("yesterday"), None);
        let day = parse_date("2026-01-31").unwrap();
        assert_eq!(insights_period(&schema(json!({ "from": "2026-01-31", "to": "2026-01-31" }))).unwrap(), (day, day + DAY_MS));
        assert_eq!(insights_period(&schema(json!({ "to": "2026-01-31", "days": 7 }))).unwrap(), (day - 6 * DAY_MS, day + DAY_MS));
        let (from, to) = insights_period(&schema(json!({}))).unwrap();
        assert_eq!(from % DAY_MS, 0);
        assert!(to - from >= 29 * DAY_MS && to - from <= 30 * DAY_MS, "today and the 29 days before");
        assert!(insights_period(&schema(json!({ "from": "2026-02-01", "to": "2026-01-31" }))).is_err());
    }
}
