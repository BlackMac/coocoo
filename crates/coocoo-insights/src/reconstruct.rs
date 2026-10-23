//! Workflow events from before a host's usage log starts (or all of them while it is off),
//! rebuilt from what Coocoo keeps anyway: the agent facts (omp statistics, ledger), the agents'
//! timelines and the Kanban cards. Coarser than the recorded log (no devices, no UI events, no
//! modes), so every event carries `reconstructed: true` and the UI says so.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::ops::Range;
use std::path::Path;

use coocoo_proto::insights::{Actor, InsightEvent, slash_name};
use coocoo_proto::{Board, CardColumn, ItemKind, TimelineItem, ToolStatus};
use serde_json::Map;

use crate::report::AgentFacts;

pub struct Input<'a> {
    pub host_id: &'a str,
    /// `<data_dir>/agents`: `<id>/timeline.jsonl` per agent.
    pub agents_dir: &'a Path,
    pub agents: &'a [AgentFacts],
    /// `(project id, board)` of this host's projects.
    pub boards: &'a [(String, Board)],
    /// Start of the recorded log: only older events are rebuilt.
    pub before: i64,
    pub from: i64,
    pub to: i64,
}

/// Rebuilt events in `[from, min(to, before))`, unsorted.
pub fn events(input: &Input) -> Vec<InsightEvent> {
    let span = input.from..input.to.min(input.before);
    if span.is_empty() {
        return Vec::new();
    }
    let mut sink = Sink { host: input.host_id, span, events: Vec::new() };
    for a in input.agents {
        agent_events(&mut sink, a);
        let active = sink.span.contains(&a.created_at)
            || a.deleted_at.is_some_and(|t| sink.span.contains(&t))
            || a.prompt_ts.iter().any(|t| sink.span.contains(t));
        // Timelines can be large; an agent without activity in the window has nothing to add.
        if active {
            for item in timeline(&input.agents_dir.join(&a.id).join("timeline.jsonl")) {
                timeline_events(&mut sink, a, item);
            }
        }
    }
    for (project, board) in input.boards {
        for card in &board.cards {
            card_events(&mut sink, project, card);
        }
    }
    sink.events
}

struct Sink<'a> {
    host: &'a str,
    span: Range<i64>,
    events: Vec<InsightEvent>,
}

impl Sink<'_> {
    fn event(&self, ts: i64, kind: &str, actor: Actor) -> InsightEvent {
        InsightEvent {
            ts,
            kind: kind.to_owned(),
            actor,
            device: None,
            client_id: None,
            host: Some(self.host.to_owned()),
            agent_id: None,
            project_id: None,
            card_id: None,
            ok: true,
            ms: None,
            meta: Map::new(),
            reconstructed: true,
        }
    }

    fn agent_event(&self, a: &AgentFacts, ts: i64, kind: &str, actor: Actor) -> InsightEvent {
        self.event(ts, kind, actor).agent(&a.id).project(a.project_id.as_deref())
    }

    fn card_event(&self, project: &str, card: &coocoo_proto::Card, ts: i64, kind: &str, actor: Actor) -> InsightEvent {
        self.event(ts, kind, actor).project(Some(project)).card(&card.id)
    }

    fn push(&mut self, event: InsightEvent) {
        if self.span.contains(&event.ts) {
            self.events.push(event);
        }
    }
}

fn agent_events(sink: &mut Sink, a: &AgentFacts) {
    let (source, actor) = if a.parent_id.is_some() {
        ("agent", Actor::Agent)
    } else if a.from_card {
        ("card", Actor::User)
    } else {
        ("user", Actor::User)
    };
    let mut created = sink.agent_event(a, a.created_at, "agent.created", actor).meta("source", source);
    if let Some(parent) = &a.parent_id {
        created = created.meta("parentId", parent.as_str());
    }
    sink.push(created);
    if let Some(ts) = a.deleted_at {
        sink.push(sink.agent_event(a, ts, "agent.deleted", Actor::User));
    }
    for &ts in &a.prompt_ts {
        sink.push(sink.agent_event(a, ts, "agents.prompt", Actor::User));
    }
    for (stamps, stop) in [(&a.aborted_ts, "aborted"), (&a.error_ts, "error")] {
        for &ts in stamps {
            sink.push(sink.agent_event(a, ts, "turn.end", Actor::Daemon).meta("stop", stop));
        }
    }
}

/// The items of a timeline file that can stand for an event, last version per id, in `seq`
/// order. Missing or unreadable files and lines are skipped.
fn timeline(path: &Path) -> Vec<TimelineItem> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut items: HashMap<String, TimelineItem> = HashMap::new();
    for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
        if !wanted(&line) {
            continue;
        }
        if let Ok(item) = serde_json::from_slice::<TimelineItem>(&line) {
            items.insert(item.id.clone(), item);
        }
    }
    let mut items: Vec<TimelineItem> = items.into_values().collect();
    items.sort_unstable_by_key(|i| i.seq);
    items
}

/// Whether a timeline line is a notice, error or tool item. Lines are compact serde output in
/// field order (`seq`, `id`, `ts`, `kind`, …) and quotes inside strings are escaped, so the first
/// `"kind":"` is the item's own: assistant and thinking text — the bulk of a timeline — is
/// skipped without parsing.
fn wanted(line: &[u8]) -> bool {
    const KEY: &[u8] = b"\"kind\":\"";
    const KINDS: [&[u8]; 3] = [b"notice\"", b"error\"", b"tool\""];
    line.windows(KEY.len()).position(|w| w == KEY).is_some_and(|at| {
        let rest = &line[at + KEY.len()..];
        KINDS.iter().any(|kind| rest.starts_with(kind))
    })
}

fn timeline_events(sink: &mut Sink, a: &AgentFacts, item: TimelineItem) {
    match item.kind {
        ItemKind::Notice | ItemKind::Error => {
            let base = sink.agent_event(a, item.ts, "", Actor::Daemon);
            if let Some(event) = text_event(base, item.kind == ItemKind::Error, item.text.as_deref().unwrap_or("")) {
                sink.push(event);
            }
        }
        ItemKind::Tool => {
            let Some(tool) = item.tool.filter(|t| t.name == "ask") else {
                return;
            };
            sink.push(sink.agent_event(a, item.ts, "request.ask", Actor::Daemon).meta("method", "ask"));
            let (ToolStatus::Ok | ToolStatus::Error, Some(ms)) = (tool.status, tool.duration_ms) else {
                return;
            };
            let ms = i64::try_from(ms).unwrap_or(i64::MAX);
            let interrupted = tool.status == ToolStatus::Error
                || tool.output.as_deref().is_some_and(|o| o.trim_start().get(..11).is_some_and(|p| p.eq_ignore_ascii_case("interrupted")));
            let end = item.ts.saturating_add(ms);
            let event = if interrupted {
                sink.agent_event(a, end, "request.cancel", Actor::Daemon).meta("reason", "process")
            } else {
                sink.agent_event(a, end, "request.answer", Actor::User)
            };
            sink.push(event.ms(ms).meta("method", "ask"));
        }
        _ => {}
    }
}

/// The event a notice or error item stands for, recognised by the texts `coocoo-agents`
/// writes. "Compaction skipped" is not a compaction; an aborted or failed one is, with `ok` false.
fn text_event(mut event: InsightEvent, error: bool, text: &str) -> Option<InsightEvent> {
    let kind = if text.starts_with("Context compacted") || (text.starts_with("Compaction ") && !text.starts_with("Compaction skipped")) {
        let ok = !error && !text.starts_with("Compaction aborted");
        event = event.meta("ok", ok);
        "compaction"
    } else if error && text.starts_with("omp exited unexpectedly") {
        "agent.crash"
    } else if error && text.starts_with("Retries exhausted") {
        event = event.meta("ok", false);
        "retry.end"
    } else if error && text.contains("is not available in Coocoo") {
        if let Some(command) = slash_name(text) {
            event = event.meta("command", command);
        }
        "command.refused"
    } else if let (false, Some(model)) = (error, text.strip_prefix("Model: ")) {
        event = event.meta("to", model.trim()).meta("cause", "unknown");
        "model.change"
    } else if !error && text.starts_with("Hibernated") {
        "agent.hibernate"
    } else {
        return None;
    };
    event.kind = kind.to_owned();
    Some(event)
}

fn card_events(sink: &mut Sink, project: &str, card: &coocoo_proto::Card) {
    let suggested_by = card.added_by.as_ref().map(|a| a.id.as_str());
    let actor = match (suggested_by, &card.source) {
        (Some(_), _) => Actor::Agent,
        (None, Some(_)) => Actor::Daemon,
        (None, None) => Actor::User,
    };
    let mut add = sink.card_event(project, card, card.created_at, "card.add", actor).meta("byAgent", suggested_by.is_some());
    if let Some(agent) = suggested_by {
        add = add.agent(agent);
    }
    // Suggestions and source entries land in the inbox; for anything else the column is unknown.
    if suggested_by.is_some() || card.source.is_some() {
        add = add.meta("column", "inbox");
    }
    sink.push(add);
    if let Some(r) = &card.refinement {
        let mut refined = sink.card_event(project, card, r.at, "card.refined", Actor::Agent);
        if let Some(agent) = &r.agent_id {
            refined = refined.agent(agent);
        }
        sink.push(refined);
    }
    let done = match (card.done_at, card.column) {
        (Some(ts), _) => Some((ts, false)),
        (None, CardColumn::Done) => Some((card.updated_at, true)),
        (None, _) => None,
    };
    if let Some((ts, approx)) = done {
        let mut moved = sink.card_event(project, card, ts, "card.move", Actor::Agent).meta("to", "done");
        if approx {
            moved = moved.meta("approx", true);
        }
        if let Some(agent) = &card.agent_id {
            moved = moved.agent(agent);
        }
        sink.push(moved);
    }
}
