//! Result types of `insights.usage` and `insights.workflows` (camelCase JSON, mirrored in
//! `ui/src/lib/protocol.ts`).
//!
//! Every host computes a partial report over its own data; the home daemon merges the partials
//! ([`UsageReport::merge`]) and derives ratios, medians and streaks once ([`UsageReport::finish`]),
//! so the UI and the `coocoo_insights` tool always see the same numbers.

use std::collections::BTreeMap;

use coocoo_proto::insights::InsightEvent;
use serde::{Deserialize, Serialize};

/// The filter every query takes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    /// Unix ms, inclusive.
    pub from: i64,
    /// Unix ms, exclusive.
    pub to: i64,
    /// The client's UTC offset in minutes (east positive, `-new Date().getTimezoneOffset()`);
    /// days and the heatmap use it.
    #[serde(default)]
    pub tz_offset: i32,
    /// Project id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Model name as omp records it (`claude-opus-5-5`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Host ids to include; absent = every host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    /// Answer from this daemon's own data only (used by the fan-out to peers).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_only: bool,
    /// Compare with the period of equal length right before `from`.
    #[serde(default = "yes")]
    pub compare: bool,
}

fn yes() -> bool {
    true
}

impl Query {
    pub fn prev_from(&self) -> i64 {
        self.from - (self.to - self.from)
    }
}

/// Where one host's numbers come from, and why they may be missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSource {
    pub host_id: String,
    pub name: String,
    /// The host answered (it may still lack omp statistics, see `statsError`).
    pub ok: bool,
    /// Why the host could not answer at all (offline, timeout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// omp's statistics database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_db: Option<String>,
    /// When `omp stats` last synced it (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<i64>,
    /// `meta.rollup_version` of the database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Why token/cost numbers are missing (no database, unknown schema, sync failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_error: Option<String>,
    /// Oldest entry of the usage log (Unix ms); older workflow data is reconstructed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_since: Option<i64>,
    /// The usage log is switched on.
    #[serde(default)]
    pub log_enabled: bool,
}

// ─── Usage ──────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub total: f64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Cost {
    pub fn add(&mut self, o: &Cost) {
        self.total += o.total;
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

/// KPI totals of a period.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub cost: Cost,
    pub tokens: Tokens,
    /// Assistant responses (one per model call), main agents, subagents and advisors.
    pub requests: u64,
    /// Responses that ended with `stop_reason = error`.
    pub failed: u64,
    /// Responses that ended with `stop_reason = aborted`.
    pub aborted: u64,
    /// Messages sent to main agents.
    pub prompts: u64,
    /// Commits attributed to an agent (exact or during one of its turns), each counted once.
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    /// Coocoo agents with at least one request.
    pub agents: u64,
    /// Subagent sessions with at least one request.
    pub subagents: u64,
    /// Sum of the agents' active time (see `docs/PROTOCOL.md`).
    pub active_ms: i64,
    /// Local days with at least one prompt or request (indices, see [`DayRow::index`]).
    pub active_day_set: Vec<i64>,
    /// Derived by [`UsageReport::finish`].
    #[serde(default)]
    pub active_days: u64,
    /// Longest run of consecutive active days in the period; derived.
    #[serde(default)]
    pub streak: u64,
    /// Cards that entered done.
    pub cards_done: u64,
}

impl Totals {
    pub fn merge(&mut self, o: &Totals) {
        self.cost.add(&o.cost);
        self.tokens.add(&o.tokens);
        self.requests += o.requests;
        self.failed += o.failed;
        self.aborted += o.aborted;
        self.prompts += o.prompts;
        self.commits += o.commits;
        self.lines_added += o.lines_added;
        self.lines_deleted += o.lines_deleted;
        self.agents += o.agents;
        self.subagents += o.subagents;
        self.active_ms += o.active_ms;
        self.active_day_set.extend(&o.active_day_set);
        self.cards_done += o.cards_done;
    }

    fn finish(&mut self) {
        self.active_day_set.sort_unstable();
        self.active_day_set.dedup();
        self.active_days = self.active_day_set.len() as u64;
        let mut best = 0;
        let mut run = 0;
        let mut last = None;
        for &d in &self.active_day_set {
            run = if last == Some(d - 1) { run + 1 } else { 1 };
            best = best.max(run);
            last = Some(d);
        }
        self.streak = best;
    }
}

/// One model's share of a day (or of an agent's day).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayModel {
    pub cost: f64,
    pub tokens: u64,
    pub requests: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayRow {
    /// Days since 1970-01-01 in the query's time zone.
    pub index: i64,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Unix ms of the local midnight that starts the day.
    pub start: i64,
    pub models: BTreeMap<String, DayModel>,
    pub commits: u64,
    pub prompts: u64,
    /// Prompts with at least one frustration signal.
    pub flagged: u64,
    pub cards_done: u64,
    /// Of `cardsDone`: cards whose done time is only known approximately (`updatedAt`).
    pub cards_done_approx: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub model: String,
    pub provider: String,
    pub requests: u64,
    pub failed: u64,
    pub cost: Cost,
    pub tokens: Tokens,
    pub ttft_sum: f64,
    pub ttft_n: u64,
    /// Output tokens per second of generation (`duration − ttft`), summed per response.
    pub tps_sum: f64,
    pub tps_n: u64,
    /// Derived: `cacheRead / (input + cacheRead + cacheWrite)`.
    #[serde(default)]
    pub cache_hit: f64,
    /// Derived, ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft: Option<f64>,
    /// Derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tps: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    pub host_id: String,
    pub project_id: String,
    pub name: String,
    pub agents: u64,
    pub cost: f64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub cards_done: u64,
}

/// How a commit was attributed to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommitMatch {
    /// Recorded by the Kanban board (`kanban_done`).
    Exact,
    /// Made in the agent's repository while one of its turns ran (±60 s).
    Turn,
    /// Inside the turns of several agents of the same repository.
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRef {
    pub sha: String,
    pub ts: i64,
    pub summary: String,
    pub lines_added: u64,
    pub lines_deleted: u64,
    #[serde(rename = "match")]
    pub matched: CommitMatch,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCost {
    pub main: f64,
    pub subagents: f64,
    pub advisor: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRow {
    pub host_id: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub created_at: i64,
    /// Deleted in Coocoo; kept from the agent ledger.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// Models used in the period, most requests first.
    pub models: Vec<String>,
    pub prompts: u64,
    pub requests: u64,
    pub failed: u64,
    pub tool_calls: u64,
    pub active_ms: i64,
    pub cost: Cost,
    pub cost_by: AgentCost,
    pub requests_by: BTreeMap<String, u64>,
    /// Subagent sessions with requests in the period.
    pub subagent_sessions: u64,
    /// omp session files (main sessions; subagents live under `<file without .jsonl>/`).
    pub session_files: Vec<String>,
    pub commits: Vec<CommitRef>,
    pub lines_added: u64,
    pub lines_deleted: u64,
    /// Cards of this agent that entered done in the period.
    pub cards_done: u64,
    /// Cost per day, aligned with [`UsageReport::days`] (filled by `finish`).
    pub day_cost: Vec<f64>,
    /// Cost per day index before alignment.
    #[serde(default, skip_serializing)]
    pub day_cost_by_index: BTreeMap<i64, f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    pub name: String,
    pub calls: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signals {
    pub yelling: u64,
    pub profanity: u64,
    pub anguish: u64,
    pub negation: u64,
    pub repetition: u64,
    pub blame: u64,
}

impl Signals {
    pub fn add(&mut self, o: &Signals) {
        self.yelling += o.yelling;
        self.profanity += o.profanity;
        self.anguish += o.anguish;
        self.negation += o.negation;
        self.repetition += o.repetition;
        self.blame += o.blame;
    }

    pub fn any(&self) -> bool {
        self.yelling + self.profanity + self.anguish + self.negation + self.repetition + self.blame > 0
    }

    pub fn names(&self) -> Vec<String> {
        [
            ("yelling", self.yelling),
            ("profanity", self.profanity),
            ("anguish", self.anguish),
            ("negation", self.negation),
            ("repetition", self.repetition),
            ("blame", self.blame),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, _)| name.to_owned())
        .collect()
    }
}

/// A prompt omp flagged as frustrated (text from omp's statistics, shortened).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlaggedPrompt {
    pub ts: i64,
    pub host_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub text: String,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frustration {
    pub prompts: u64,
    pub flagged: u64,
    pub signals: Signals,
    /// Newest first, at most [`MAX_FLAGGED`].
    pub examples: Vec<FlaggedPrompt>,
}

pub const MAX_FLAGGED: usize = 200;
const MAX_AGENT_COMMITS: usize = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub query: Option<Query>,
    pub hosts: Vec<HostSource>,
    pub totals: Totals,
    /// The period of equal length before `from` (absent when `compare` is off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<Totals>,
    /// Every day of the period, oldest first.
    pub days: Vec<DayRow>,
    pub models: Vec<ModelRow>,
    pub projects: Vec<ProjectRow>,
    pub agents: Vec<AgentRow>,
    pub tools: Vec<ToolRow>,
    /// Prompts by local weekday (0 = Monday) × hour.
    pub heatmap: Vec<Vec<u64>>,
    pub frustration: Frustration,
    /// Model names with requests in the period, for the filter (ignores the model filter).
    pub all_models: Vec<String>,
}

impl UsageReport {
    /// Fold another host's partial report into this one.
    pub fn merge(&mut self, o: UsageReport) {
        self.hosts.extend(o.hosts);
        self.totals.merge(&o.totals);
        match (&mut self.prev, o.prev) {
            (Some(a), Some(b)) => a.merge(&b),
            (a @ None, Some(b)) => *a = Some(b),
            _ => {}
        }
        for day in o.days {
            match self.days.iter_mut().find(|d| d.index == day.index) {
                Some(d) => {
                    for (model, m) in day.models {
                        let e = d.models.entry(model).or_default();
                        e.cost += m.cost;
                        e.tokens += m.tokens;
                        e.requests += m.requests;
                    }
                    d.commits += day.commits;
                    d.prompts += day.prompts;
                    d.flagged += day.flagged;
                    d.cards_done += day.cards_done;
                    d.cards_done_approx += day.cards_done_approx;
                }
                None => self.days.push(day),
            }
        }
        for m in o.models {
            match self.models.iter_mut().find(|x| x.model == m.model && x.provider == m.provider) {
                Some(x) => {
                    x.requests += m.requests;
                    x.failed += m.failed;
                    x.cost.add(&m.cost);
                    x.tokens.add(&m.tokens);
                    x.ttft_sum += m.ttft_sum;
                    x.ttft_n += m.ttft_n;
                    x.tps_sum += m.tps_sum;
                    x.tps_n += m.tps_n;
                }
                None => self.models.push(m),
            }
        }
        self.projects.extend(o.projects);
        self.agents.extend(o.agents);
        for t in o.tools {
            match self.tools.iter_mut().find(|x| x.name == t.name) {
                Some(x) => {
                    x.calls += t.calls;
                    x.errors += t.errors;
                }
                None => self.tools.push(t),
            }
        }
        if self.heatmap.is_empty() {
            self.heatmap = o.heatmap;
        } else {
            for (row, other) in self.heatmap.iter_mut().zip(o.heatmap) {
                for (cell, n) in row.iter_mut().zip(other) {
                    *cell += n;
                }
            }
        }
        let f = &mut self.frustration;
        f.prompts += o.frustration.prompts;
        f.flagged += o.frustration.flagged;
        f.signals.add(&o.frustration.signals);
        f.examples.extend(o.frustration.examples);
        self.all_models.extend(o.all_models);
    }

    /// Sort, align and derive once all partials are merged.
    pub fn finish(&mut self) {
        self.totals.finish();
        if let Some(prev) = &mut self.prev {
            prev.finish();
        }
        self.days.sort_by_key(|d| d.index);
        for m in &mut self.models {
            let prompt = m.tokens.input + m.tokens.cache_read + m.tokens.cache_write;
            m.cache_hit = if prompt > 0 { m.tokens.cache_read as f64 / prompt as f64 } else { 0.0 };
            m.ttft = (m.ttft_n > 0).then(|| m.ttft_sum / m.ttft_n as f64);
            m.tps = (m.tps_n > 0).then(|| m.tps_sum / m.tps_n as f64);
        }
        self.models.sort_by(|a, b| b.cost.total.total_cmp(&a.cost.total).then(b.requests.cmp(&a.requests)));
        self.projects.sort_by(|a, b| b.cost.total_cmp(&a.cost));
        let index: Vec<i64> = self.days.iter().map(|d| d.index).collect();
        for a in &mut self.agents {
            a.day_cost = index.iter().map(|i| a.day_cost_by_index.get(i).copied().unwrap_or(0.0)).collect();
            a.commits.sort_by_key(|c| std::cmp::Reverse(c.ts));
            a.commits.truncate(MAX_AGENT_COMMITS);
        }
        self.agents.sort_by(|a, b| b.cost.total.total_cmp(&a.cost.total).then(b.prompts.cmp(&a.prompts)));
        self.tools.sort_by_key(|t| std::cmp::Reverse(t.calls));
        self.frustration.examples.sort_by_key(|e| std::cmp::Reverse(e.ts));
        self.frustration.examples.truncate(MAX_FLAGGED);
        self.all_models.sort();
        self.all_models.dedup();
        self.hosts.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

// ─── Workflows ──────────────────────────────────────────────────────────────────────────────

/// A concrete example behind a number: an agent (and where in its history), or a card.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExampleRef {
    pub host_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    /// Agent title or card title at the time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Unix ms: where in the agent's timeline the example happened.
    pub ts: i64,
}

/// Facts about one agent that the workflow analysis needs from its host, over `[prevFrom, to)`
/// (time-stamped where the analysis splits current and previous period).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFacts {
    pub host_id: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// A Kanban card names it as its agent.
    pub from_card: bool,
    /// Prompts to its main session (omp statistics), oldest first.
    pub prompt_ts: Vec<i64>,
    /// Of those: prompts omp flagged as frustrated.
    pub flagged_ts: Vec<i64>,
    /// Main-session responses that ended aborted / with an error.
    pub aborted_ts: Vec<i64>,
    pub error_ts: Vec<i64>,
    /// Tool calls (all sessions) in `[from, to)` and in `[prevFrom, from)`.
    pub tool_calls: u64,
    pub tool_calls_prev: u64,
    /// First `edit`/`write`/`ast_edit`/`notebook` call (files changed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_edit_at: Option<i64>,
    /// Times of the commits attributed to it.
    pub commit_ts: Vec<i64>,
    /// API-equivalent cost in `[from, to)`.
    pub cost: f64,
    /// Turns as `(start, length ms)`, derived from omp's statistics.
    pub turns: Vec<(i64, i64)>,
    /// Agents it spawned (all time).
    pub children: u64,
}

/// Lifecycle of one card as far as it is known.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardFacts {
    pub host_id: String,
    pub project_id: String,
    pub id: String,
    pub title: String,
    pub column: String,
    pub created_at: i64,
    /// Entered todo (inbox cards only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refined_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<i64>,
    /// `doneAt` is the card's `updatedAt` (finished before Coocoo recorded the time).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done_approx: bool,
    /// Moves out of doing/done back to an earlier column.
    pub back_moves: u64,
    /// Times work was started on it.
    pub starts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Added by this agent (`kanban_add`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<String>,
    pub was_inbox: bool,
}

/// One host's raw material for the workflow analysis (`insights.workflows` with `localOnly`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPartial {
    pub host: HostSource,
    /// Recorded and reconstructed events in `[prevFrom, to)`, oldest first.
    pub events: Vec<InsightEvent>,
    pub agents: Vec<AgentFacts>,
    pub cards: Vec<CardFacts>,
    /// The last pattern analysis started on this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyze: Option<AnalyzeRun>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kpi {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<f64>,
    /// Second number (p90, count, …) as the KPI defines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<f64>,
    /// Sample size.
    pub n: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowKpis {
    /// Agents created; `extra` = from cards, `byAgents` below.
    pub agents_started: Kpi,
    pub agents_by_agents: u64,
    /// Median prompts per agent; `extra` = p90.
    pub prompts_per_agent: Kpi,
    /// Tool calls per prompt.
    pub autonomy: Kpi,
    /// Median ms a question waited for an answer; `extra` = p90.
    pub needs_you_wait: Kpi,
    /// Share of turns aborted; `extra` = aborts followed by a steer within 2 min.
    pub abort_rate: Kpi,
    /// Share of agents with at least one commit.
    pub agent_commit: Kpi,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunnelStage {
    pub id: String,
    pub label: String,
    pub count: u64,
    /// Median ms from this stage to the next, over those that reached it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_to_next_ms: Option<i64>,
    /// Agents (or cards) that reached this stage but not the next, newest first.
    pub stuck: Vec<ExampleRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KanbanFlow {
    pub stages: Vec<FunnelStage>,
    pub back_moves: u64,
    /// Refined cards finished in one run (one start, no move back): done / finished.
    pub refined_one_run: (u64, u64),
    pub unrefined_one_run: (u64, u64),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    pub id: String,
    pub label: String,
    /// The rule, in words (tooltip).
    pub rule: String,
    pub agents: u64,
    pub prompts_median: f64,
    pub turn_median_ms: i64,
    pub cost_avg: f64,
    /// Share of its agents with a commit; `None` when none could commit (no changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_rate: Option<f64>,
    pub examples: Vec<ExampleRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sequence {
    /// Normalized step ids (`card.refine`, `request.answer`, …); labels in the UI.
    pub steps: Vec<String>,
    pub count: u64,
    pub median_ms: i64,
    /// Occurrences followed by a commit within the window.
    pub commits: u64,
    /// Occurrences containing a friction signal.
    pub friction: u64,
    /// Occurrences that started on a phone.
    pub phone: u64,
    pub examples: Vec<ExampleRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub id: String,
    pub label: String,
    pub uses: u64,
    pub days: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
    pub desktop: u64,
    pub phone: u64,
    pub browser: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Friction {
    pub id: String,
    pub label: String,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<u64>,
    pub examples: Vec<ExampleRef>,
}

/// The last `insights.analyze` run and what became of its cards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeRun {
    pub host_id: String,
    pub project_id: String,
    pub agent_id: String,
    pub at: i64,
    /// Inbox cards the agent added.
    pub cards: u64,
    /// Of those: moved on from the inbox.
    pub accepted: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReport {
    pub query: Option<Query>,
    pub hosts: Vec<HostSource>,
    /// Oldest usage-log entry over all hosts; workflow data before it is reconstructed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_since: Option<i64>,
    /// Events in the period (recorded and reconstructed).
    pub events: u64,
    pub reconstructed: u64,
    pub kpis: WorkflowKpis,
    pub journey: Vec<FunnelStage>,
    pub kanban: KanbanFlow,
    pub shapes: Vec<Shape>,
    pub sequences: Vec<Sequence>,
    pub features: Vec<Feature>,
    pub friction: Vec<Friction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_analyze: Option<AnalyzeRun>,
}

/// Median and p90 of a sample (nearest rank); `None` when empty.
pub fn median_p90(values: &[i64]) -> (Option<i64>, Option<i64>) {
    (quantile(values, 0.5), quantile(values, 0.9))
}

pub fn quantile(values: &[i64], q: f64) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}
