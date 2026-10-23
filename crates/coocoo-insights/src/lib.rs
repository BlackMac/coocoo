//! Usage statistics and workflow insights (`insights.*`), used only by `coocoo-core`.
//!
//! - [`log`]: the usage log, `<data_dir>/events/YYYY-MM.jsonl` (content-free events).
//! - [`ledger`]: `<data_dir>/insights/ledger.jsonl`, so deleted agents stay in the statistics.
//! - [`omp`]: omp's `~/.omp/stats.db` (tokens, cost, tool calls, prompts), read-only.
//! - [`git`]: commits per project, attributed to agents by [`usage`].
//! - [`usage`]: one host's part of the Stats tab ([`UsageReport`]).
//! - [`reconstruct`], [`workflows`]: the Workflows tab, mined once over every host's raw facts.
//!
//! Every host answers for its own data; the home daemon merges ([`UsageReport::merge`],
//! [`workflows::mine`]) so the UI and the agent tool get the same numbers.

pub mod git;
pub mod ledger;
pub mod log;
pub mod omp;
pub mod reconstruct;
pub mod report;
pub mod time;
pub mod usage;
pub mod workflows;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coocoo_proto::AgentSummary;
use coocoo_proto::insights::{InsightEvent, LedgerEntry};
use coocoo_proto::util::{read_json, write_json_atomic};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

pub use report::*;

use crate::ledger::Ledger;
use crate::log::EventLog;
use crate::omp::OmpStats;

/// `[insights]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InsightsConfig {
    /// Record the usage log (local only).
    pub enabled: bool,
    /// Delete log months older than this many days (`0` = keep everything).
    pub retention_days: u32,
}

impl Default for InsightsConfig {
    fn default() -> Self {
        Self { enabled: true, retention_days: 365 }
    }
}

/// `insights.settings` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub enabled: bool,
    pub retention_days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_since: Option<i64>,
    pub bytes: u64,
}

/// A project as the queries need it.
#[derive(Debug, Clone)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// What the daemon knows that the queries need: this host and its live state.
#[derive(Debug, Clone)]
pub struct LocalContext {
    pub host_id: String,
    pub host_name: String,
    pub projects: Vec<ProjectRef>,
    /// Agents that exist now (deleted ones come from the ledger).
    pub agents: Vec<AgentSummary>,
}

/// The last pattern-analysis agent started on this host (`insights/analyze.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeRecord {
    pub project_id: String,
    pub agent_id: String,
    pub at: i64,
}

pub struct Insights {
    data_dir: PathBuf,
    log: Arc<EventLog>,
    ledger: Ledger,
    stats: OmpStats,
    analyze: Mutex<Option<AnalyzeRecord>>,
    handle: Handle,
}

impl Insights {
    /// Must be called inside the tokio runtime. `omp` is the omp executable (for `omp stats`).
    pub fn new(data_dir: &Path, omp: PathBuf, config: &InsightsConfig) -> Arc<Self> {
        let handle = Handle::current();
        let analyze = read_json::<AnalyzeRecord>(&data_dir.join("insights").join("analyze.json")).ok().flatten();
        Arc::new(Self {
            data_dir: data_dir.to_owned(),
            log: EventLog::new(data_dir.join("events"), config.enabled, config.retention_days, handle.clone()),
            ledger: Ledger::open(data_dir.join("insights").join("ledger.jsonl")),
            stats: OmpStats::new(omp),
            analyze: Mutex::new(analyze),
            handle,
        })
    }

    pub fn log_enabled(&self) -> bool {
        self.log.enabled()
    }

    /// Queue an event for the usage log (dropped while it is off).
    pub fn record(&self, event: InsightEvent) {
        self.log.record(event);
    }

    /// Fold an agent identity change into the ledger; unchanged entries (most calls) cost a
    /// lookup, changes are written off the async runtime.
    pub fn ledger(self: &Arc<Self>, entry: LedgerEntry) {
        if !self.ledger.changes(&entry) {
            return;
        }
        let me = self.clone();
        self.handle.spawn_blocking(move || me.ledger.record(entry));
    }

    /// Record every existing agent the ledger does not know yet (or knows differently). Blocking.
    pub fn backfill(&self, agents: &[AgentSummary]) {
        for a in agents {
            self.ledger.record(entry_of(a, a.last_activity_at, None));
        }
    }

    /// Write buffered events (shutdown). Blocking.
    pub fn flush(&self) {
        self.log.flush();
    }

    pub fn settings(&self, retention_days: u32) -> SettingsInfo {
        SettingsInfo { enabled: self.log.enabled(), retention_days, log_since: self.log.since(), bytes: self.log.bytes() }
    }

    pub fn configure(&self, config: &InsightsConfig) {
        self.log.set_enabled(config.enabled);
        self.log.set_retention(config.retention_days);
    }

    /// Delete the usage log.
    pub fn clear(&self) -> std::io::Result<()> {
        self.log.clear()
    }

    /// Recorded events in `[from, to)` (this host), oldest first, at most `limit` (newest kept).
    pub fn events(&self, from: i64, to: i64, limit: usize) -> (Vec<InsightEvent>, bool) {
        let mut events = self.log.read(from, to);
        let truncated = events.len() > limit;
        if truncated {
            events.drain(..events.len() - limit);
        }
        (events, truncated)
    }

    pub fn set_analyze(&self, record: AnalyzeRecord) {
        let path = self.data_dir.join("insights").join("analyze.json");
        if let Err(err) = write_json_atomic(&path, &record) {
            tracing::warn!(%err, "cannot persist the analysis run");
        }
        *self.analyze.lock() = Some(record);
    }

    /// This host's part of the Stats tab (merge with the others, then [`UsageReport::finish`]).
    pub async fn usage(self: &Arc<Self>, q: Query, ctx: LocalContext, force_sync: bool) -> UsageReport {
        let synced = self.stats.sync(force_sync).await;
        let me = self.clone();
        let (report, _) = tokio::task::spawn_blocking(move || usage::collect(&me.inputs(&ctx), &q, synced))
            .await
            .unwrap_or_default();
        report
    }

    /// This host's raw material for the Workflows tab (mine with [`workflows::mine`]).
    pub async fn workflow_partial(self: &Arc<Self>, q: Query, ctx: LocalContext, force_sync: bool) -> WorkflowPartial {
        let synced = self.stats.sync(force_sync).await;
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let inputs = me.inputs(&ctx);
            let (report, agents) = usage::collect(&inputs, &q, synced);
            let log_since = me.log.since();
            let mut events = me.log.read(q.prev_from(), q.to);
            for e in &mut events {
                e.host = Some(ctx.host_id.clone());
            }
            let cards = workflows::card_facts(&ctx.host_id, &inputs.boards, &events);
            let before = log_since.unwrap_or(i64::MAX);
            events.extend(reconstruct::events(&reconstruct::Input {
                host_id: &ctx.host_id,
                agents_dir: &me.data_dir.join("agents"),
                agents: &agents,
                boards: &inputs.boards,
                before,
                from: q.prev_from(),
                to: q.to,
            }));
            events.sort_by_key(|e| e.ts);
            let analyze = me.analyze.lock().clone().map(|r| {
                let cards = inputs.boards.iter().filter(|(p, _)| *p == r.project_id).flat_map(|(_, b)| &b.cards);
                let (mut added, mut accepted) = (0, 0);
                for c in cards.filter(|c| c.added_by.as_ref().is_some_and(|a| a.id == r.agent_id)) {
                    added += 1;
                    accepted += u64::from(c.column != coocoo_proto::CardColumn::Inbox);
                }
                AnalyzeRun { host_id: ctx.host_id.clone(), project_id: r.project_id, agent_id: r.agent_id, at: r.at, cards: added, accepted }
            });
            let host = report.hosts.into_iter().next().unwrap_or_default();
            WorkflowPartial { host, events, agents, cards, analyze }
        })
        .await
        .unwrap_or_default()
    }

    /// Ledger agents merged with the live ones, plus the boards of this host's projects.
    fn inputs(&self, ctx: &LocalContext) -> usage::Inputs {
        let boards = ctx
            .projects
            .iter()
            .filter_map(|p| {
                let board = read_json::<coocoo_proto::Board>(&self.data_dir.join("boards").join(format!("{}.json", p.id))).ok()??;
                Some((p.id.clone(), board))
            })
            .collect();
        usage::Inputs {
            ctx: ctx.clone(),
            ledger: self.ledger.agents(),
            boards,
            stats: self.stats.path.clone(),
            db: self.stats.open(),
            log_since: self.log.since(),
            log_enabled: self.log.enabled(),
        }
    }
}

/// The ledger entry describing `a` now.
pub fn entry_of(a: &AgentSummary, ts: i64, deleted_at: Option<i64>) -> LedgerEntry {
    LedgerEntry {
        ts,
        agent_id: a.id.clone(),
        title: a.title.clone(),
        project_id: a.project_id.clone(),
        cwd: a.cwd.clone(),
        worktree: a.worktree.as_ref().map(|w| w.path.clone()),
        session_file: a.session_file.clone(),
        parent_id: a.parent_id.clone(),
        created_at: a.created_at,
        deleted_at,
    }
}
