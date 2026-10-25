//! The Workflows tab, mined once on the home daemon over the raw material every host sends
//! ([`WorkflowPartial`]: recorded and reconstructed events, agent facts, card lifecycles).
//!
//! Pure computation. The vocabulary — sequence steps, features, friction signals, session shapes,
//! journey and Kanban stages — is shared with the UI labels and the "How these numbers are
//! computed" texts, so every rule here is one the user can read there.

use std::collections::{BTreeMap, HashMap, HashSet};

use coocoo_proto::insights::{Actor, InsightEvent};
use coocoo_proto::{Board, Card, CardColumn};
use serde_json::Value;

use crate::report::*;
use crate::time::day_index;

const MINUTE_MS: i64 = 60_000;
/// An interrupted question asked again within this counts as re-asked.
const REASK_MS: i64 = 10 * MINUTE_MS;
const LONG_WAIT_MS: i64 = 15 * MINUTE_MS;
/// An abort followed by a prompt this soon was a correction, not giving up.
const RECOVER_MS: i64 = 2 * MINUTE_MS;
const QUICK_TURN_MS: i64 = 2 * MINUTE_MS;
/// A client's events that belong to no agent split into sessions at longer gaps.
const SESSION_GAP_MS: i64 = 30 * MINUTE_MS;
/// A commit this soon after a sequence counts as its outcome.
const COMMIT_AFTER_MS: i64 = 30 * MINUTE_MS;
const MAX_EXAMPLES: usize = 5;
const MAX_STUCK: usize = 20;
const MAX_SEQUENCES: usize = 12;
const MIN_GRAM: usize = 3;
const MAX_GRAM: usize = 5;

const INBOX: u8 = 0;
const DOING: u8 = 2;
const DONE: u8 = 3;

/// Sequence step ids; a step is stored as its index here.
const STEPS: [&str; 28] = [
    "agent.new",
    "agent.spawn",
    "prompt",
    "steer",
    "abort",
    "turn.error",
    "needs.you",
    "answer",
    "card.add",
    "card.suggest",
    "card.accept",
    "card.refine",
    "card.start",
    "card.done",
    "commit",
    "panel.changes",
    "panel.browser",
    "panel.terminal",
    "panel.board",
    "panel.plan",
    "panel.info",
    "palette",
    "model.change",
    "leave",
    "dialog.cancel",
    "crash",
    "compaction",
    "api.limit",
];
const FRICTION_STEPS: [&str; 5] = ["abort", "turn.error", "crash", "api.limit", "dialog.cancel"];
const PANELS: [&str; 6] = ["changes", "browser", "terminal", "board", "plan", "info"];

const JOURNEY: [(&str, &str); 6] = [
    ("created", "Created"),
    ("prompted", "Prompted"),
    ("changedFiles", "Changed files"),
    ("reviewed", "Reviewed"),
    ("committed", "Committed"),
    ("closed", "Closed"),
];

const KANBAN: [(&str, &str); 5] =
    [("created", "Created"), ("accepted", "Accepted"), ("refined", "Refined"), ("doing", "Doing"), ("done", "Done")];

/// `(id, label, rule)`; the first matching rule wins, in this order.
const SHAPES: [(&str, &str, &str); 6] = [
    ("orchestrator", "Orchestrator", "Spawned at least one other agent."),
    ("kanbanRun", "Kanban run", "Started from a Kanban card, at most 3 prompts."),
    ("quickQuestion", "Quick question", "One prompt, no file changes, every turn under 2 minutes."),
    ("autonomous", "Autonomous", "At most 3 prompts and at least 20 tool calls per prompt."),
    ("pingPong", "Ping-pong", "At least 6 prompts and fewer than 10 tool calls per prompt."),
    ("other", "Other", "Everything else."),
];

const FEATURES: [(&str, &str); 19] = [
    ("board", "Kanban board"),
    ("changes", "Changes panel"),
    ("browser", "Browser panel"),
    ("terminal", "Terminal"),
    ("plan", "Plan panel"),
    ("info", "Info panel"),
    ("palette", "Command palette"),
    ("shortcuts", "Keyboard shortcuts"),
    ("slash", "Slash commands"),
    ("modelPicker", "Model picker"),
    ("steer", "Steering"),
    ("worktree", "Worktrees"),
    ("git", "Git from the UI"),
    ("actions", "Project actions"),
    ("images", "Images in prompts"),
    ("stats", "Stats & Workflows"),
    ("analyze", "Analyze patterns"),
    ("schedules", "Scheduled runs"),
    ("secrets", "Agent secrets"),
];

const FRICTION: [(&str, &str); 10] = [
    ("askInterrupted", "Re-asked questions (ask interrupted)"),
    ("longWait", "Long needs-you wait (> 15 min)"),
    ("aborts", "Aborted turns"),
    ("apiLimit", "API limit / retries exhausted"),
    ("crash", "Crash / restart"),
    ("compaction", "Compactions"),
    ("modelSwitch", "Model switched mid-session"),
    ("refusedSlash", "Refused slash command"),
    ("dialogCancel", "Cancelled dialogs"),
    ("flagged", "Flagged prompts (frustration)"),
];

// ─── Cards ──────────────────────────────────────────────────────────────────────────────────

/// One [`CardFacts`] per card of `boards` (`(project id, board)`). Lifecycle times come from the
/// recorded `card.*` events where there are any, else from the card itself; when work on a card
/// started is only known from events.
pub fn card_facts(host_id: &str, boards: &[(String, Board)], events: &[InsightEvent]) -> Vec<CardFacts> {
    let mut by_card: HashMap<&str, Vec<&InsightEvent>> = HashMap::new();
    for e in events.iter().filter(|e| matches!(e.kind.as_str(), "card.add" | "card.move" | "card.refined")) {
        if let Some(card) = e.card_id.as_deref() {
            by_card.entry(card).or_default().push(e);
        }
    }
    let mut out = Vec::new();
    for (project, board) in boards {
        for card in &board.cards {
            let events = by_card.get(card.id.as_str()).map_or(&[][..], Vec::as_slice);
            out.push(card_fact(host_id, project, card, events));
        }
    }
    out
}

fn card_fact(host_id: &str, project: &str, card: &Card, events: &[&InsightEvent]) -> CardFacts {
    let moves = || events.iter().copied().filter(|e| e.kind == "card.move");
    let moved = |e: &InsightEvent| (meta_str(e, "from").and_then(column_rank), meta_str(e, "to").and_then(column_rank));
    let was_inbox = card.column == CardColumn::Inbox
        || card.source.is_some()
        || card.added_by.is_some()
        || events.iter().any(|e| {
            (e.kind == "card.add" && meta_str(e, "column") == Some("inbox"))
                || (e.kind == "card.move" && meta_str(e, "from") == Some("inbox"))
        });
    let accepted_at = if was_inbox {
        moves().find(|e| matches!(moved(e), (Some(INBOX), Some(to)) if to > INBOX)).map(|e| e.ts)
    } else {
        None
    };
    let doing: Vec<i64> = moves().filter(|e| moved(e).1 == Some(DOING)).map(|e| e.ts).collect();
    // A card in doing or done with an agent was started at least once, even before the log knew.
    let starts = match doing.len() {
        0 if card.agent_id.is_some() && matches!(card.column, CardColumn::Doing | CardColumn::Done) => 1,
        n => n as u64,
    };
    let back_moves = moves().filter(|e| matches!(moved(e), (Some(from), Some(to)) if from >= DOING && to < from)).count() as u64;
    let (done_at, done_approx) = if card.column == CardColumn::Done {
        match moves().filter(|e| moved(e).1 == Some(DONE)).map(|e| e.ts).max().or(card.done_at) {
            Some(ts) => (Some(ts), false),
            None => (Some(card.updated_at), true),
        }
    } else {
        (None, false)
    };
    CardFacts {
        host_id: host_id.to_owned(),
        project_id: project.to_owned(),
        id: card.id.clone(),
        title: card.title.clone(),
        column: column_name(card.column).to_owned(),
        created_at: card.created_at,
        accepted_at,
        refined_at: events
            .iter()
            .find(|e| e.kind == "card.refined")
            .map(|e| e.ts)
            .or(card.refinement.as_ref().map(|r| r.at)),
        doing_at: doing.first().copied(),
        done_at,
        done_approx,
        back_moves,
        starts,
        agent_id: card.agent_id.clone(),
        added_by: card.added_by.as_ref().map(|a| a.id.clone()),
        was_inbox,
    }
}

fn column_rank(name: &str) -> Option<u8> {
    match name {
        "inbox" => Some(INBOX),
        "todo" => Some(1),
        "doing" => Some(DOING),
        "done" => Some(DONE),
        _ => None,
    }
}

fn column_name(column: CardColumn) -> &'static str {
    match column {
        CardColumn::Inbox => "inbox",
        CardColumn::Todo => "todo",
        CardColumn::Doing => "doing",
        CardColumn::Done => "done",
    }
}

// ─── Mining ─────────────────────────────────────────────────────────────────────────────────

/// The Workflows tab over `[q.from, q.to)`, previous-period values over `[q.prev_from(), q.from)`
/// when `q.compare`. `q.model` is ignored: the hosts already narrowed the agent facts by it.
pub fn mine(q: &Query, partials: Vec<WorkflowPartial>) -> WorkflowReport {
    let mut hosts = Vec::with_capacity(partials.len());
    let (mut events, mut agents, mut cards) = (Vec::new(), Vec::new(), Vec::new());
    let mut last_analyze: Option<AnalyzeRun> = None;
    for p in partials {
        let host = p.host.host_id.clone();
        events.extend(p.events.into_iter().map(|mut e| {
            if e.host.is_none() {
                e.host = Some(host.clone());
            }
            e
        }));
        agents.extend(p.agents);
        cards.extend(p.cards);
        if let Some(run) = p.analyze
            && last_analyze.as_ref().is_none_or(|last| run.at > last.at) {
                last_analyze = Some(run);
            }
        hosts.push(p.host);
    }
    if let Some(project) = q.project.as_deref() {
        agents.retain(|a| a.project_id.as_deref() == Some(project));
        cards.retain(|c| c.project_id == project);
        let agent_ids: HashSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        let card_ids: HashSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
        events.retain(|e| match e.project_id.as_deref() {
            Some(p) => p == project,
            None => {
                e.agent_id.as_deref().is_some_and(|a| agent_ids.contains(a))
                    || e.card_id.as_deref().is_some_and(|c| card_ids.contains(c))
            }
        });
    }
    // Hosts answer in any order; sorting makes the result independent of it.
    events.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.host.cmp(&b.host)));
    agents.sort_by(|a, b| (&a.host_id, &a.id).cmp(&(&b.host_id, &b.id)));
    cards.sort_by(|a, b| (&a.host_id, &a.id).cmp(&(&b.host_id, &b.id)));

    let cur = Span { from: q.from, to: q.to };
    let prev = q.compare.then(|| Span { from: q.prev_from(), to: q.from });
    let ctx = Ctx::new(q, &events, &agents, &cards, &hosts);
    let (in_period, reconstructed) = events
        .iter()
        .filter(|e| cur.has(e.ts))
        .fold((0, 0), |(n, r), e| (n + 1, r + u64::from(e.reconstructed)));
    WorkflowReport {
        query: Some(q.clone()),
        log_since: hosts.iter().filter_map(|h| h.log_since).min(),
        events: in_period,
        reconstructed,
        kpis: ctx.kpis(cur, prev),
        journey: ctx.journey(cur),
        kanban: ctx.kanban(cur),
        shapes: ctx.shapes(cur),
        sequences: ctx.sequences(cur),
        features: ctx.features(cur),
        friction: ctx.friction(cur, prev),
        last_analyze,
        hosts,
    }
}

#[derive(Clone, Copy)]
struct Span {
    from: i64,
    to: i64,
}

impl Span {
    fn has(self, ts: i64) -> bool {
        (self.from..self.to).contains(&ts)
    }
}

struct Ctx<'a> {
    events: &'a [InsightEvent],
    agents: &'a [AgentFacts],
    cards: &'a [CardFacts],
    /// Agents by id alone: ids are random (48 bits), and UI events that name an agent are stored
    /// on the home host, not on the agent's.
    by_id: HashMap<&'a str, &'a AgentFacts>,
    card_agent: HashMap<&'a str, &'a str>,
    /// Host → start of its usage log (turns before it come from omp's statistics).
    log_since: HashMap<String, i64>,
    /// Per agent, oldest first.
    prompts: HashMap<&'a str, Vec<i64>>,
    asks: HashMap<&'a str, Vec<i64>>,
    /// Its changes panel opened, or it was marked read.
    reviews: HashMap<&'a str, Vec<i64>>,
    /// First archive or deletion.
    closed: HashMap<&'a str, i64>,
    tz: i32,
}

impl<'a> Ctx<'a> {
    fn new(
        q: &Query,
        events: &'a [InsightEvent],
        agents: &'a [AgentFacts],
        cards: &'a [CardFacts],
        hosts: &[HostSource],
    ) -> Self {
        let mut ctx = Ctx {
            events,
            agents,
            cards,
            by_id: agents.iter().map(|a| (a.id.as_str(), a)).collect(),
            card_agent: cards.iter().filter_map(|c| Some((c.id.as_str(), c.agent_id.as_deref()?))).collect(),
            log_since: hosts.iter().filter_map(|h| Some((h.host_id.clone(), h.log_since?))).collect(),
            prompts: HashMap::new(),
            asks: HashMap::new(),
            reviews: HashMap::new(),
            closed: HashMap::new(),
            tz: q.tz_offset,
        };
        for e in events.iter().filter(|e| e.ok) {
            let Some(agent) = e.agent_id.as_deref() else {
                continue;
            };
            let times = match e.kind.as_str() {
                "agents.prompt" => &mut ctx.prompts,
                "request.ask" => &mut ctx.asks,
                "agents.markRead" => &mut ctx.reviews,
                "ui.panel" if meta_str(e, "panel") == Some("changes") => &mut ctx.reviews,
                "agents.archive" | "agent.deleted" => {
                    ctx.closed.entry(agent).or_insert(e.ts);
                    continue;
                }
                _ => continue,
            };
            times.entry(agent).or_default().push(e.ts);
        }
        ctx
    }

    fn agent_example(&self, host: &str, agent: &str, ts: i64) -> ExampleRef {
        match self.by_id.get(agent) {
            Some(a) => agent_ref(a, ts),
            None => ExampleRef { host_id: host.to_owned(), agent_id: Some(agent.to_owned()), ts, ..Default::default() },
        }
    }

    fn event_example(&self, e: &InsightEvent) -> ExampleRef {
        let mut example = match e.agent_id.as_deref() {
            Some(agent) => self.agent_example(host_of(e), agent, e.ts),
            None => ExampleRef { host_id: host_of(e).to_owned(), ts: e.ts, ..Default::default() },
        };
        example.project_id = example.project_id.take().or_else(|| e.project_id.clone());
        example.card_id = e.card_id.clone();
        example
    }

    // ── KPIs ──

    fn numbers(&self, span: Span, previous: bool) -> Numbers {
        let mut n = Numbers::default();
        for e in self.events.iter().filter(|e| span.has(e.ts)) {
            match e.kind.as_str() {
                "agent.created" => {
                    n.started += 1;
                    match meta_str(e, "source") {
                        Some("card") => n.from_cards += 1,
                        Some("agent") => n.by_agents += 1,
                        _ => {}
                    }
                }
                "request.answer" => n.waits.extend(e.ms),
                "turn.end" => {
                    // Reconstructed turn ends repeat omp's turns, counted below.
                    n.turns += u64::from(!e.reconstructed);
                    if meta_str(e, "stop") == Some("aborted") {
                        n.aborted += 1;
                        n.recovered += u64::from(self.recovered(e));
                    }
                }
                _ => {}
            }
        }
        for a in self.agents {
            let log_start = self.log_since.get(&a.host_id).copied().unwrap_or(i64::MAX);
            n.turns += a.turns.iter().filter(|&&(start, _)| span.has(start) && start < log_start).count() as u64;
            n.tool_calls += if previous { a.tool_calls_prev } else { a.tool_calls };
            let prompts = count_in(&a.prompt_ts, span);
            if prompts > 0 {
                n.prompts.push(prompts as i64);
                n.prompt_total += prompts;
                n.prompted += 1;
                n.committed += u64::from(a.commit_ts.iter().any(|&t| span.has(t)));
            }
        }
        // Every aborted turn is a turn, whichever source missed it.
        n.turns = n.turns.max(n.aborted);
        n
    }

    /// A prompt (or steer) of the same agent followed the aborted turn `e` within 2 min.
    fn recovered(&self, e: &InsightEvent) -> bool {
        e.agent_id.as_deref().and_then(|a| self.prompts.get(a)).is_some_and(|times| next_within(times, e.ts, RECOVER_MS))
    }

    fn kpis(&self, cur: Span, prev: Option<Span>) -> WorkflowKpis {
        let c = self.numbers(cur, false);
        let p = prev.map(|span| self.numbers(span, true));
        let p = p.as_ref();
        let median = |values: &[i64]| quantile(values, 0.5).map(|v| v as f64);
        let (prompts_median, prompts_p90) = median_p90(&c.prompts);
        let (wait_median, wait_p90) = median_p90(&c.waits);
        WorkflowKpis {
            agents_started: Kpi {
                value: c.started as f64,
                prev: p.map(|p| p.started as f64),
                extra: Some(c.from_cards as f64),
                n: c.started,
            },
            agents_by_agents: c.by_agents,
            prompts_per_agent: Kpi {
                value: prompts_median.unwrap_or(0) as f64,
                prev: p.and_then(|p| median(&p.prompts)),
                extra: prompts_p90.map(|v| v as f64),
                n: c.prompts.len() as u64,
            },
            autonomy: Kpi {
                value: ratio(c.tool_calls, c.prompt_total).unwrap_or(0.0),
                prev: p.and_then(|p| ratio(p.tool_calls, p.prompt_total)),
                extra: None,
                n: c.prompt_total,
            },
            needs_you_wait: Kpi {
                value: wait_median.unwrap_or(0) as f64,
                prev: p.and_then(|p| median(&p.waits)),
                extra: wait_p90.map(|v| v as f64),
                n: c.waits.len() as u64,
            },
            abort_rate: Kpi {
                value: ratio(c.aborted, c.turns).unwrap_or(0.0),
                prev: p.and_then(|p| ratio(p.aborted, p.turns)),
                extra: Some(c.recovered as f64),
                n: c.turns,
            },
            agent_commit: Kpi {
                value: ratio(c.committed, c.prompted).unwrap_or(0.0),
                prev: p.and_then(|p| ratio(p.committed, p.prompted)),
                extra: None,
                n: c.prompted,
            },
        }
    }

    // ── Funnels ──

    fn journey(&self, span: Span) -> Vec<FunnelStage> {
        let items: Vec<Progress> = self
            .agents
            .iter()
            .filter(|a| span.has(a.created_at))
            .map(|a| {
                let id = a.id.as_str();
                // Opening an agent marks it read, so only a look after its first edit is a review.
                let reviewed = a.first_edit_at.and_then(|edit| {
                    let times = self.reviews.get(id)?;
                    times.get(times.partition_point(|&t| t < edit)).copied()
                });
                let closed = earliest(self.closed.get(id).copied(), a.deleted_at);
                let at = vec![
                    Some(a.created_at),
                    a.prompt_ts.iter().min().copied(),
                    a.first_edit_at,
                    reviewed,
                    a.commit_ts.iter().min().copied(),
                    closed,
                ];
                let mut reached: Vec<bool> = at.iter().map(Option::is_some).collect();
                reached[5] |= a.archived;
                Progress { reached, at, example: agent_ref(a, a.created_at) }
            })
            .collect();
        funnel(&JOURNEY, &items)
    }

    fn kanban(&self, span: Span) -> KanbanFlow {
        let cards: Vec<&CardFacts> = self.cards.iter().filter(|c| span.has(c.created_at)).collect();
        let items: Vec<Progress> = cards
            .iter()
            .map(|c| {
                let at = vec![
                    Some(c.created_at),
                    if c.was_inbox { c.accepted_at } else { Some(c.created_at) },
                    c.refined_at,
                    c.doing_at,
                    c.done_at,
                ];
                let reached = vec![
                    true,
                    !c.was_inbox || c.accepted_at.is_some() || c.column != "inbox",
                    c.refined_at.is_some(),
                    c.doing_at.is_some() || c.starts > 0 || matches!(c.column.as_str(), "doing" | "done"),
                    c.done_at.is_some() || c.column == "done",
                ];
                Progress { reached, at, example: card_ref(c, c.created_at) }
            })
            .collect();
        let mut flow = KanbanFlow {
            stages: funnel(&KANBAN, &items),
            back_moves: cards.iter().map(|c| c.back_moves).sum(),
            ..Default::default()
        };
        for c in cards.iter().filter(|c| c.column == "done") {
            let tally = if c.refined_at.is_some() { &mut flow.refined_one_run } else { &mut flow.unrefined_one_run };
            tally.0 += u64::from(c.starts == 1 && c.back_moves == 0);
            tally.1 += 1;
        }
        flow
    }

    // ── Shapes ──

    fn shapes(&self, span: Span) -> Vec<Shape> {
        let mut acc: Vec<ShapeAcc> = SHAPES.iter().map(|_| ShapeAcc::default()).collect();
        for a in self.agents {
            let prompts: Vec<i64> = a.prompt_ts.iter().copied().filter(|&t| span.has(t)).collect();
            let Some(&first) = prompts.iter().min() else {
                continue;
            };
            let turns: Vec<i64> = a.turns.iter().filter(|&&(start, _)| span.has(start)).map(|&(_, ms)| ms).collect();
            let s = &mut acc[shape_of(a, prompts.len() as u64, &turns)];
            s.prompts.push(prompts.len() as i64);
            s.turns.extend(turns);
            s.cost += a.cost;
            if a.first_edit_at.is_some() {
                s.changed += 1;
                s.committed += u64::from(a.commit_ts.iter().any(|&t| span.has(t)));
            }
            s.examples.push(agent_ref(a, first));
        }
        SHAPES
            .iter()
            .zip(acc)
            .map(|(&(id, label, rule), s)| Shape {
                id: id.to_owned(),
                label: label.to_owned(),
                rule: rule.to_owned(),
                agents: s.prompts.len() as u64,
                prompts_median: quantile(&s.prompts, 0.5).unwrap_or(0) as f64,
                turn_median_ms: quantile(&s.turns, 0.5).unwrap_or(0),
                cost_avg: if s.prompts.is_empty() { 0.0 } else { s.cost / s.prompts.len() as f64 },
                commit_rate: ratio(s.committed, s.changed),
                examples: newest(s.examples, MAX_EXAMPLES),
            })
            .collect()
    }

    // ── Sequences ──

    /// Step streams: one per agent (its events, its cards' events and its commits), and per
    /// client the events that belong to no agent, split into sessions. Consecutive repeats
    /// collapse.
    fn streams(&self, span: Span) -> Vec<Stream<'a>> {
        let mut agents: BTreeMap<&'a str, (&'a str, Vec<Point>)> = BTreeMap::new();
        let mut clients: BTreeMap<(&'a str, &'a str), Vec<Point>> = BTreeMap::new();
        for e in self.events.iter().filter(|e| span.has(e.ts)) {
            let Some(step) = step(e) else {
                continue;
            };
            let point = Point { ts: e.ts, step, phone: e.device.as_deref() == Some("phone") };
            let agent = e.agent_id.as_deref().or_else(|| e.card_id.as_deref().and_then(|c| self.card_agent.get(c).copied()));
            match agent {
                Some(agent) => agents.entry(agent).or_insert_with(|| (host_of(e), Vec::new())).1.push(point),
                None => {
                    let client = e.client_id.as_deref().unwrap_or(actor_name(e.actor));
                    clients.entry((host_of(e), client)).or_default().push(point);
                }
            }
        }
        if let Some(commit) = step_index("commit") {
            for a in self.agents {
                for &ts in a.commit_ts.iter().filter(|&&t| span.has(t)) {
                    let stream = agents.entry(a.id.as_str()).or_insert_with(|| (a.host_id.as_str(), Vec::new()));
                    stream.1.push(Point { ts, step: commit, phone: false });
                }
            }
        }
        let mut out = Vec::new();
        for (agent, (host, mut points)) in agents {
            points.sort_by_key(|p| p.ts);
            out.push(Stream { host, agent: Some(agent), points: collapse(points) });
        }
        for ((host, _), points) in clients {
            let mut session: Vec<Point> = Vec::new();
            for p in points {
                if session.last().is_some_and(|last| p.ts - last.ts > SESSION_GAP_MS) {
                    out.push(Stream { host, agent: None, points: collapse(std::mem::take(&mut session)) });
                }
                session.push(p);
            }
            if !session.is_empty() {
                out.push(Stream { host, agent: None, points: collapse(session) });
            }
        }
        out
    }

    fn sequences(&self, span: Span) -> Vec<Sequence> {
        let streams = self.streams(span);
        let commit = step_index("commit");
        let mut friction = [false; STEPS.len()];
        for step in FRICTION_STEPS.iter().filter_map(|s| step_index(s)) {
            friction[usize::from(step)] = true;
        }
        let mut grams: HashMap<Gram, GramStats> = HashMap::new();
        for (index, stream) in streams.iter().enumerate() {
            let points = &stream.points;
            let commits: Vec<i64> = points.iter().filter(|p| Some(p.step) == commit).map(|p| p.ts).collect();
            for start in 0..points.len() {
                for len in MIN_GRAM..=MAX_GRAM {
                    let Some(window) = points.get(start..start + len) else {
                        break;
                    };
                    let mut key = [u8::MAX; MAX_GRAM];
                    for (k, p) in key.iter_mut().zip(window) {
                        *k = p.step;
                    }
                    let (first, last) = (window[0], window[len - 1]);
                    let g = grams.entry((len, key)).or_default();
                    g.count += 1;
                    g.spans.push(last.ts - first.ts);
                    let committed = window.iter().any(|p| Some(p.step) == commit)
                        || commits.get(commits.partition_point(|&t| t < last.ts)).is_some_and(|&t| t - last.ts <= COMMIT_AFTER_MS);
                    g.commits += u64::from(committed);
                    g.friction += u64::from(window.iter().any(|p| friction[usize::from(p.step)]));
                    g.phone += u64::from(first.phone);
                    // One example per stream (its newest occurrence), the newest streams first.
                    match g.examples.iter_mut().find(|(_, s)| *s == index) {
                        Some(example) => example.0 = example.0.max(first.ts),
                        None => {
                            g.examples.push((first.ts, index));
                            if g.examples.len() > MAX_EXAMPLES {
                                sort_examples(&mut g.examples);
                                g.examples.truncate(MAX_EXAMPLES);
                            }
                        }
                    }
                }
            }
        }
        let repeated: Vec<(Gram, GramStats)> = grams.into_iter().filter(|(_, g)| g.count >= 2).collect();
        // A shorter n-gram that only ever occurs inside a longer one adds nothing.
        let covered: HashSet<(Gram, u64)> = repeated
            .iter()
            .filter(|((len, _), _)| *len > MIN_GRAM)
            .flat_map(|&((len, key), ref g)| {
                let count = g.count;
                (MIN_GRAM..len).flat_map(move |sub| {
                    (0..=len - sub).map(move |start| {
                        let mut part = [u8::MAX; MAX_GRAM];
                        part[..sub].copy_from_slice(&key[start..start + sub]);
                        ((sub, part), count)
                    })
                })
            })
            .collect();
        let mut kept: Vec<(Gram, GramStats)> =
            repeated.into_iter().filter(|(gram, g)| !covered.contains(&(*gram, g.count))).collect();
        kept.sort_by(|(a, ga), (b, gb)| {
            (gb.count * b.0 as u64)
                .cmp(&(ga.count * a.0 as u64))
                .then(gb.count.cmp(&ga.count))
                .then(a.cmp(b))
        });
        kept.truncate(MAX_SEQUENCES);
        kept.into_iter()
            .map(|((len, key), mut g)| {
                sort_examples(&mut g.examples);
                Sequence {
                    steps: key[..len].iter().map(|&s| STEPS[usize::from(s)].to_owned()).collect(),
                    count: g.count,
                    median_ms: quantile(&g.spans, 0.5).unwrap_or(0),
                    commits: g.commits,
                    friction: g.friction,
                    phone: g.phone,
                    examples: g
                        .examples
                        .iter()
                        .map(|&(ts, index)| {
                            let stream = &streams[index];
                            match stream.agent {
                                Some(agent) => self.agent_example(stream.host, agent, ts),
                                None => ExampleRef { host_id: stream.host.to_owned(), ts, ..Default::default() },
                            }
                        })
                        .collect(),
                }
            })
            .collect()
    }

    // ── Features ──

    fn features(&self, span: Span) -> Vec<Feature> {
        let mut uses: HashMap<&'static str, FeatureAcc> = HashMap::new();
        for e in self.events {
            features_of(e, |id| {
                let f = uses.entry(id).or_default();
                f.last = f.last.max(Some(e.ts));
                if span.has(e.ts) {
                    f.uses += 1;
                    f.days.insert(day_index(e.ts, self.tz));
                    match e.device.as_deref() {
                        Some("desktop") => f.desktop += 1,
                        Some("phone") => f.phone += 1,
                        Some("browser") => f.browser += 1,
                        _ => {}
                    }
                }
            });
        }
        FEATURES
            .iter()
            .map(|&(id, label)| {
                let f = uses.remove(id).unwrap_or_default();
                Feature {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    uses: f.uses,
                    days: f.days.len() as u64,
                    last_used: f.last,
                    desktop: f.desktop,
                    phone: f.phone,
                    browser: f.browser,
                }
            })
            .collect()
    }

    // ── Friction ──

    fn friction(&self, cur: Span, prev: Option<Span>) -> Vec<Friction> {
        let mut acc: HashMap<&'static str, FrictionAcc> = HashMap::new();
        let mut hit = |id: &'static str, ts: i64, example: &dyn Fn() -> ExampleRef| {
            let f = acc.entry(id).or_default();
            if cur.has(ts) {
                f.count += 1;
                f.examples.push(example());
            } else if prev.is_some_and(|p| p.has(ts)) {
                f.prev += 1;
            }
        };
        for e in self.events {
            if let Some(id) = self.friction_of(e) {
                hit(id, e.ts, &|| {
                    let mut example = self.event_example(e);
                    if id == "dialogCancel" {
                        example.label = meta_str(e, "dialog").map(str::to_owned);
                    }
                    example
                });
            }
        }
        for a in self.agents {
            for &ts in &a.flagged_ts {
                hit("flagged", ts, &|| agent_ref(a, ts));
            }
        }
        FRICTION
            .iter()
            .map(|&(id, label)| {
                let f = acc.remove(id).unwrap_or_default();
                Friction {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    count: f.count,
                    prev: prev.map(|_| f.prev),
                    examples: newest(f.examples, MAX_EXAMPLES),
                }
            })
            .collect()
    }

    fn friction_of(&self, e: &InsightEvent) -> Option<&'static str> {
        if !e.ok {
            return None;
        }
        Some(match e.kind.as_str() {
            // Rebuilt cancellations are interrupted `ask` calls, which omp always asks again.
            "request.cancel" if matches!(meta_str(e, "reason"), Some("process" | "omp")) && (e.reconstructed || self.reasked(e)) => {
                "askInterrupted"
            }
            "request.answer" if e.ms.is_some_and(|ms| ms > LONG_WAIT_MS) => "longWait",
            "turn.end" => match meta_str(e, "stop") {
                Some("aborted") => "aborts",
                Some("error") => "apiLimit",
                _ => return None,
            },
            "retry.end" if meta_bool(e, "ok") == Some(false) => "apiLimit",
            "agent.crash" => "crash",
            "compaction" => "compaction",
            "model.change" if meta_str(e, "cause") != Some("respawn") && self.after_first_prompt(e) => "modelSwitch",
            "command.refused" => "refusedSlash",
            "ui.dialog.cancel" => "dialogCancel",
            _ => return None,
        })
    }

    fn reasked(&self, e: &InsightEvent) -> bool {
        e.agent_id.as_deref().and_then(|a| self.asks.get(a)).is_some_and(|times| next_within(times, e.ts, REASK_MS))
    }

    fn after_first_prompt(&self, e: &InsightEvent) -> bool {
        let Some(agent) = e.agent_id.as_deref() else {
            return false;
        };
        let recorded = self.prompts.get(agent).and_then(|t| t.first().copied());
        let facts = self.by_id.get(agent).and_then(|a| a.prompt_ts.iter().min().copied());
        earliest(recorded, facts).is_some_and(|first| first < e.ts)
    }
}

#[derive(Default)]
struct Numbers {
    started: u64,
    from_cards: u64,
    by_agents: u64,
    /// Prompts per agent with at least one.
    prompts: Vec<i64>,
    prompt_total: u64,
    tool_calls: u64,
    waits: Vec<i64>,
    turns: u64,
    aborted: u64,
    recovered: u64,
    prompted: u64,
    committed: u64,
}

/// One agent's or card's way through a funnel: per stage whether it got there and, if known, when.
struct Progress {
    reached: Vec<bool>,
    at: Vec<Option<i64>>,
    example: ExampleRef,
}

/// Stages are not strictly sequential (a card can skip refinement), so "stuck" means reached the
/// stage and none after it.
fn funnel(stages: &[(&str, &str)], items: &[Progress]) -> Vec<FunnelStage> {
    stages
        .iter()
        .enumerate()
        .map(|(i, &(id, label))| {
            let has_next = i + 1 < stages.len();
            let (mut count, mut gaps, mut stuck) = (0, Vec::new(), Vec::new());
            for p in items.iter().filter(|p| p.reached[i]) {
                count += 1;
                if !has_next {
                    continue;
                }
                if let (Some(from), Some(to)) = (p.at[i], p.at[i + 1])
                    && to >= from {
                        gaps.push(to - from);
                    }
                if !p.reached[i + 1..].contains(&true) {
                    stuck.push(ExampleRef { ts: p.at[i].unwrap_or(p.example.ts), ..p.example.clone() });
                }
            }
            FunnelStage {
                id: id.to_owned(),
                label: label.to_owned(),
                count,
                median_to_next_ms: quantile(&gaps, 0.5),
                stuck: newest(stuck, MAX_STUCK),
            }
        })
        .collect()
}

#[derive(Default)]
struct ShapeAcc {
    prompts: Vec<i64>,
    turns: Vec<i64>,
    cost: f64,
    changed: u64,
    committed: u64,
    examples: Vec<ExampleRef>,
}

/// Index into [`SHAPES`]. `turns` are the lengths of its turns in the period.
fn shape_of(a: &AgentFacts, prompts: u64, turns: &[i64]) -> usize {
    let per_prompt = a.tool_calls as f64 / prompts as f64;
    if a.children >= 1 {
        0
    } else if a.from_card && prompts <= 3 {
        1
    } else if prompts == 1 && a.first_edit_at.is_none() && turns.iter().all(|&ms| ms < QUICK_TURN_MS) {
        2
    } else if prompts <= 3 && per_prompt >= 20.0 {
        3
    } else if prompts >= 6 && per_prompt < 10.0 {
        4
    } else {
        5
    }
}

#[derive(Clone, Copy)]
struct Point {
    ts: i64,
    step: u8,
    phone: bool,
}

struct Stream<'a> {
    host: &'a str,
    agent: Option<&'a str>,
    points: Vec<Point>,
}

/// `(length, steps)`, unused slots `u8::MAX`.
type Gram = (usize, [u8; MAX_GRAM]);

#[derive(Default)]
struct GramStats {
    count: u64,
    spans: Vec<i64>,
    commits: u64,
    friction: u64,
    phone: u64,
    /// `(ts of the first step, stream index)`.
    examples: Vec<(i64, usize)>,
}

fn sort_examples(examples: &mut [(i64, usize)]) {
    examples.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
}

fn collapse(mut points: Vec<Point>) -> Vec<Point> {
    points.dedup_by_key(|p| p.step);
    points
}

fn step_index(id: &str) -> Option<u8> {
    STEPS.iter().position(|s| *s == id).and_then(|i| u8::try_from(i).ok())
}

/// The sequence step an event stands for, if any.
fn step(e: &InsightEvent) -> Option<u8> {
    if !e.ok {
        return None;
    }
    let id = match e.kind.as_str() {
        "agent.created" => match meta_str(e, "source") {
            Some("user") => "agent.new",
            Some("agent") => "agent.spawn",
            _ => return None,
        },
        "agents.prompt" if meta_str(e, "mode") == Some("steer") => "steer",
        "agents.prompt" => "prompt",
        // The turn end that follows an abort collapses into it.
        "agents.abort" => "abort",
        "turn.end" => match meta_str(e, "stop") {
            Some("aborted") => "abort",
            Some("error") => "turn.error",
            _ => return None,
        },
        "request.ask" => "needs.you",
        "request.answer" => "answer",
        "boards.cardAdd" if e.actor == Actor::User => "card.add",
        "card.add" if meta_bool(e, "byAgent") == Some(true) => "card.suggest",
        // Recorded user adds are the `boards.cardAdd` action; rebuilt ones have only this.
        "card.add" if e.reconstructed => "card.add",
        "card.move" => match (meta_str(e, "from"), meta_str(e, "to")) {
            (Some("inbox"), Some("todo")) => "card.accept",
            (_, Some("done")) => "card.done",
            _ => return None,
        },
        "boards.cardRefine" => "card.refine",
        "card.refined" if e.reconstructed => "card.refine",
        "boards.cardStart" => "card.start",
        "git.commit" => "commit",
        "boards.cardDone" if meta_bool(e, "commit") == Some(true) => "commit",
        "ui.panel" => match meta_str(e, "panel") {
            Some("changes") => "panel.changes",
            Some("browser") => "panel.browser",
            Some("terminal") => "panel.terminal",
            Some("board") => "panel.board",
            Some("plan") => "panel.plan",
            Some("info") => "panel.info",
            _ => return None,
        },
        "ui.palette.run" => "palette",
        "agents.setModel" | "ui.model.pick" => "model.change",
        "ui.leave" => "leave",
        "ui.dialog.cancel" => "dialog.cancel",
        "agent.crash" => "crash",
        "compaction" => "compaction",
        "retry.end" if meta_bool(e, "ok") == Some(false) => "api.limit",
        _ => return None,
    };
    step_index(id)
}

#[derive(Default)]
struct FeatureAcc {
    uses: u64,
    days: HashSet<i64>,
    last: Option<i64>,
    desktop: u64,
    phone: u64,
    browser: u64,
}

/// Calls `hit` with every feature a user's event is a use of.
fn features_of(e: &InsightEvent, mut hit: impl FnMut(&'static str)) {
    if e.actor != Actor::User || !e.ok {
        return;
    }
    match e.kind.as_str() {
        "ui.panel" => {
            if let Some(panel) = meta_str(e, "panel").and_then(|p| PANELS.into_iter().find(|f| *f == p)) {
                hit(panel);
            }
        }
        "ui.palette.open" => hit("palette"),
        "ui.shortcut" => hit("shortcuts"),
        "ui.slash" => hit("slash"),
        "ui.model.open" => hit("modelPicker"),
        "ui.route" => {
            if matches!(meta_str(e, "name"), Some("stats" | "workflows")) {
                hit("stats");
            }
        }
        "agents.prompt" => {
            if meta_str(e, "mode") == Some("steer") {
                hit("steer");
            }
            if e.meta.contains_key("slash") {
                hit("slash");
            }
            if meta_u64(e, "images").is_some_and(|n| n > 0) {
                hit("images");
            }
        }
        "agents.create" => {
            if meta_bool(e, "worktree") == Some(true) {
                hit("worktree");
            }
        }
        "git.worktreeCreate" => hit("worktree"),
        "git.commit" | "git.merge" | "git.push" | "git.prCreate" | "git.prMerge" => hit("git"),
        "actions.run" => hit("actions"),
        "terminal.open" => hit("terminal"),
        "insights.analyze" => hit("analyze"),
        kind if kind.starts_with("boards.") => hit("board"),
        kind if kind.starts_with("schedules.") => hit("schedules"),
        kind if kind.starts_with("secrets.") => hit("secrets"),
        kind if kind.starts_with("browser.") => hit("browser"),
        _ => {}
    }
}

#[derive(Default)]
struct FrictionAcc {
    count: u64,
    prev: u64,
    examples: Vec<ExampleRef>,
}

// ─── Helpers ────────────────────────────────────────────────────────────────────────────────

fn meta_str<'e>(e: &'e InsightEvent, key: &str) -> Option<&'e str> {
    e.meta.get(key).and_then(Value::as_str)
}

fn meta_bool(e: &InsightEvent, key: &str) -> Option<bool> {
    e.meta.get(key).and_then(Value::as_bool)
}

fn meta_u64(e: &InsightEvent, key: &str) -> Option<u64> {
    e.meta.get(key).and_then(Value::as_u64)
}

fn host_of(e: &InsightEvent) -> &str {
    e.host.as_deref().unwrap_or_default()
}

fn actor_name(actor: Actor) -> &'static str {
    match actor {
        Actor::User => "user",
        Actor::Agent => "agent",
        Actor::Daemon => "daemon",
    }
}

fn count_in(times: &[i64], span: Span) -> u64 {
    times.iter().filter(|&&t| span.has(t)).count() as u64
}

/// `times` (sorted) has an entry after `ts` and at most `within` later.
fn next_within(times: &[i64], ts: i64, within: i64) -> bool {
    times.get(times.partition_point(|&t| t <= ts)).is_some_and(|&t| t - ts <= within)
}

fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn ratio(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

fn agent_ref(a: &AgentFacts, ts: i64) -> ExampleRef {
    ExampleRef {
        host_id: a.host_id.clone(),
        agent_id: Some(a.id.clone()),
        project_id: a.project_id.clone(),
        card_id: None,
        label: Some(a.title.clone()),
        ts,
    }
}

fn card_ref(c: &CardFacts, ts: i64) -> ExampleRef {
    ExampleRef {
        host_id: c.host_id.clone(),
        agent_id: c.agent_id.clone(),
        project_id: Some(c.project_id.clone()),
        card_id: Some(c.id.clone()),
        label: Some(c.title.clone()),
        ts,
    }
}

/// The `max` newest, ties by agent, card and host id.
fn newest(mut refs: Vec<ExampleRef>, max: usize) -> Vec<ExampleRef> {
    refs.sort_by(|a, b| {
        b.ts.cmp(&a.ts)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
            .then_with(|| a.card_id.cmp(&b.card_id))
            .then_with(|| a.host_id.cmp(&b.host_id))
    });
    refs.truncate(max);
    refs
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;
    const HOUR: i64 = 3_600_000;
    const FROM: i64 = 1_790_000_000_000;

    fn query(compare: bool) -> Query {
        Query {
            from: FROM,
            to: FROM + 7 * DAY,
            tz_offset: 0,
            project: None,
            model: None,
            hosts: None,
            local_only: false,
            compare,
        }
    }

    fn ev(ts: i64, kind: &str, agent: &str) -> InsightEvent {
        let mut e = InsightEvent::new(kind).agent(agent).project(Some("p1"));
        e.ts = ts;
        e
    }

    fn agent(id: &str, created_at: i64) -> AgentFacts {
        AgentFacts {
            host_id: "h1".into(),
            id: id.into(),
            title: format!("Agent {id}"),
            project_id: Some("p1".into()),
            created_at,
            ..Default::default()
        }
    }

    fn partial(events: Vec<InsightEvent>, agents: Vec<AgentFacts>) -> WorkflowPartial {
        WorkflowPartial {
            host: HostSource { host_id: "h1".into(), name: "Host".into(), ok: true, ..Default::default() },
            events,
            agents,
            cards: Vec::new(),
            analyze: None,
        }
    }

    fn ids(refs: &[ExampleRef]) -> Vec<&str> {
        refs.iter().filter_map(|r| r.agent_id.as_deref()).collect()
    }

    #[test]
    fn finds_the_kanban_refine_sequence_with_examples() {
        let mut events = Vec::new();
        for (i, id) in ["a1", "a2"].into_iter().enumerate() {
            let t = FROM + i as i64 * DAY;
            let card = format!("c{i}");
            events.push(ev(t, "boards.cardRefine", id).card(&card));
            events.push(ev(t + 60_000, "request.ask", id));
            events.push(ev(t + 120_000, "request.answer", id).ms(60_000));
            events.push(ev(t + 180_000, "boards.cardStart", id).card(&card));
            events.push(ev(t + 600_000, "card.move", id).card(&card).meta("from", "doing").meta("to", "done"));
        }
        let report = mine(&query(true), vec![partial(events, vec![agent("a1", FROM), agent("a2", FROM)])]);
        // Every shorter n-gram occurs exactly as often, inside this one, and is dropped.
        assert_eq!(report.sequences.len(), 1);
        let seq = &report.sequences[0];
        assert_eq!(seq.steps, ["card.refine", "needs.you", "answer", "card.start", "card.done"]);
        assert_eq!((seq.count, seq.median_ms, seq.commits, seq.friction), (2, 600_000, 0, 0));
        assert_eq!(ids(&seq.examples), ["a2", "a1"]);
        assert_eq!(seq.examples[0].ts, FROM + DAY);
        assert_eq!(seq.examples[0].label.as_deref(), Some("Agent a2"));
        assert_eq!(seq.examples[0].host_id, "h1");
    }

    #[test]
    fn needs_you_wait_is_the_median_answer_time() {
        let events = vec![
            ev(FROM - HOUR, "request.answer", "a1").ms(600_000),
            ev(FROM + HOUR, "request.answer", "a1").ms(180_000),
            ev(FROM + 2 * HOUR, "request.answer", "a1").ms(60_000),
            ev(FROM + 3 * HOUR, "request.answer", "a1").ms(120_000),
        ];
        let report = mine(&query(true), vec![partial(events, vec![agent("a1", FROM)])]);
        let wait = &report.kpis.needs_you_wait;
        assert_eq!((wait.value, wait.extra, wait.n, wait.prev), (120_000.0, Some(180_000.0), 3, Some(600_000.0)));
    }

    #[test]
    fn shapes_take_the_first_matching_rule() {
        let in_period = |n: i64| (0..n).map(|i| FROM + HOUR + i * 60_000).collect::<Vec<_>>();
        let shaped = |id: &str, prompts: i64, tool_calls: u64| AgentFacts {
            prompt_ts: in_period(prompts),
            tool_calls,
            ..agent(id, FROM)
        };
        let agents = vec![
            AgentFacts { children: 1, from_card: true, ..shaped("orch", 1, 0) },
            AgentFacts { from_card: true, ..shaped("card", 2, 100) },
            AgentFacts { turns: vec![(FROM + HOUR, 30_000)], ..shaped("quick", 1, 3) },
            AgentFacts { first_edit_at: Some(FROM + HOUR), ..shaped("auto", 1, 30) },
            shaped("pong", 6, 30),
            shaped("other", 6, 120),
            AgentFacts { prompt_ts: vec![FROM - DAY], ..agent("idle", FROM - DAY) },
        ];
        let report = mine(&query(true), vec![partial(Vec::new(), agents)]);
        let by_shape: Vec<(&str, Vec<&str>)> =
            report.shapes.iter().map(|s| (s.id.as_str(), ids(&s.examples))).collect();
        assert_eq!(
            by_shape,
            [
                ("orchestrator", vec!["orch"]),
                ("kanbanRun", vec!["card"]),
                ("quickQuestion", vec!["quick"]),
                ("autonomous", vec!["auto"]),
                ("pingPong", vec!["pong"]),
                ("other", vec!["other"]),
            ]
        );
        let shape = |id: &str| report.shapes.iter().find(|s| s.id == id).unwrap();
        assert_eq!(shape("autonomous").commit_rate, Some(0.0));
        assert_eq!(shape("quickQuestion").commit_rate, None);
        assert_eq!(shape("pingPong").prompts_median, 6.0);
    }

    #[test]
    fn friction_counts_the_previous_period() {
        let mut dialog = InsightEvent::new("ui.dialog.cancel").meta("dialog", "newAgent");
        dialog.ts = FROM + 2 * HOUR;
        dialog.actor = Actor::User;
        let events = vec![
            ev(FROM - 2 * DAY, "agent.crash", "a1"),
            ev(FROM - DAY, "agent.crash", "a1"),
            ev(FROM + DAY, "agent.crash", "a1"),
            ev(FROM + HOUR, "turn.end", "a1").meta("stop", "aborted"),
            dialog,
        ];
        let facts = AgentFacts { flagged_ts: vec![FROM - HOUR, FROM + HOUR], ..agent("a1", FROM - 3 * DAY) };
        let report = mine(&query(true), vec![partial(events.clone(), vec![facts.clone()])]);
        let get = |r: &WorkflowReport, id: &str| r.friction.iter().find(|f| f.id == id).cloned().unwrap();
        let crash = get(&report, "crash");
        assert_eq!((crash.count, crash.prev), (1, Some(2)));
        assert_eq!(ids(&crash.examples), ["a1"]);
        assert_eq!(crash.examples[0].ts, FROM + DAY);
        assert_eq!((get(&report, "aborts").count, get(&report, "aborts").prev), (1, Some(0)));
        assert_eq!((get(&report, "flagged").count, get(&report, "flagged").prev), (1, Some(1)));
        assert_eq!(get(&report, "dialogCancel").examples[0].label.as_deref(), Some("newAgent"));
        assert_eq!(report.friction.len(), FRICTION.len());

        let report = mine(&query(false), vec![partial(events, vec![facts])]);
        assert_eq!(get(&report, "crash").prev, None);
    }

    #[test]
    fn journey_lists_agents_stuck_at_each_stage() {
        let prompted = AgentFacts { prompt_ts: vec![FROM + 2 * HOUR], ..agent("p", FROM + HOUR) };
        let edited = AgentFacts {
            prompt_ts: vec![FROM + 3 * HOUR],
            first_edit_at: Some(FROM + 4 * HOUR),
            ..agent("e", FROM + 3 * HOUR)
        };
        let done = AgentFacts {
            prompt_ts: vec![FROM + 5 * HOUR],
            first_edit_at: Some(FROM + 6 * HOUR),
            commit_ts: vec![FROM + 8 * HOUR],
            deleted_at: Some(FROM + 9 * HOUR),
            ..agent("c", FROM + 4 * HOUR)
        };
        let old = AgentFacts { prompt_ts: vec![FROM + HOUR], ..agent("old", FROM - DAY) };
        let events = vec![
            // Read before its first edit: not a review.
            ev(FROM + 3 * HOUR + HOUR / 2, "agents.markRead", "e"),
            ev(FROM + 7 * HOUR, "agents.markRead", "c"),
        ];
        let report = mine(&query(true), vec![partial(events, vec![prompted, edited, done, old])]);
        let stages: Vec<(&str, u64, Vec<&str>)> =
            report.journey.iter().map(|s| (s.id.as_str(), s.count, ids(&s.stuck))).collect();
        assert_eq!(
            stages,
            [
                ("created", 3, vec![]),
                ("prompted", 3, vec!["p"]),
                ("changedFiles", 2, vec!["e"]),
                ("reviewed", 1, vec![]),
                ("committed", 1, vec![]),
                ("closed", 1, vec![]),
            ]
        );
        assert_eq!(report.journey[1].stuck[0].ts, FROM + 2 * HOUR);
        assert_eq!(report.journey[0].median_to_next_ms, Some(HOUR));
    }

    #[test]
    fn card_facts_prefer_events_and_fall_back_to_the_card() {
        let card = |id: &str, column: &str, extra: serde_json::Value| -> Card {
            let mut v = serde_json::json!({
                "id": id, "title": format!("Card {id}"), "column": column,
                "createdAt": FROM, "updatedAt": FROM + DAY,
            });
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            serde_json::from_value(v).unwrap()
        };
        let board = Board {
            cards: vec![
                card("moved", "done", serde_json::json!({"agentId": "a1"})),
                card("old", "done", serde_json::json!({"agentId": "a2"})),
                card("feed", "inbox", serde_json::json!({"source": "s1"})),
            ],
            ..Default::default()
        };
        let mv = |ts: i64, from: &str, to: &str| ev(ts, "card.move", "a1").card("moved").meta("from", from).meta("to", to);
        let events = vec![
            mv(FROM + 1, "inbox", "todo"),
            mv(FROM + 2, "todo", "doing"),
            mv(FROM + 3, "doing", "todo"),
            mv(FROM + 4, "todo", "doing"),
            mv(FROM + 5, "doing", "done"),
        ];
        let facts = card_facts("h1", &[("p1".to_owned(), board)], &events);
        let moved = &facts[0];
        assert!(moved.was_inbox);
        assert_eq!((moved.accepted_at, moved.doing_at, moved.done_at), (Some(FROM + 1), Some(FROM + 2), Some(FROM + 5)));
        assert_eq!((moved.starts, moved.back_moves, moved.done_approx), (2, 1, false));
        let old = &facts[1];
        assert_eq!((old.done_at, old.done_approx, old.starts, old.was_inbox), (Some(FROM + DAY), true, 1, false));
        let feed = &facts[2];
        assert_eq!((feed.was_inbox, feed.accepted_at, feed.column.as_str()), (true, None, "inbox"));
    }
}
