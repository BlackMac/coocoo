//! One host's part of the Stats tab, and the per-agent facts the Workflows tab builds on.
//!
//! Sources: the agents (live ones and the ledger's deleted ones), omp's statistics database
//! (responses, tool calls, prompts — matched to agents by session file), the Kanban boards and
//! `git log` of the projects the agents worked in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use coocoo_proto::{AgentStatus, Board, CardColumn};

use crate::LocalContext;
use crate::git;
use crate::ledger::LedgerAgent;
use crate::omp::{Db, Synced};
use crate::report::*;
use crate::time::{date, day_index, day_start, weekday_hour};

/// Gaps between an agent's responses longer than this do not count as active time (it waited
/// for the user, or the turn ended).
const IDLE_GAP_MS: i64 = 10 * 60_000;
/// A commit within this distance of a turn counts as made during it.
const COMMIT_SLACK_MS: i64 = 60_000;
/// Tools that change files (first use = "changed files" in the agent journey).
const EDIT_TOOLS: &[&str] = &["edit", "write", "ast_edit", "notebook", "apply_patch", "multi_edit"];

/// Everything one query reads.
pub struct Inputs {
    pub ctx: LocalContext,
    pub ledger: Vec<LedgerAgent>,
    /// `(project id, board)` of this host's projects.
    pub boards: Vec<(String, Board)>,
    pub stats: PathBuf,
    pub db: Result<Db, String>,
    pub log_since: Option<i64>,
    pub log_enabled: bool,
}

#[derive(Default)]
struct Agent {
    id: String,
    title: String,
    project_id: Option<String>,
    parent_id: Option<String>,
    created_at: i64,
    deleted_at: Option<i64>,
    archived: bool,
    session_files: Vec<String>,
    /// Current period.
    row: AgentRow,
    model_requests: HashMap<String, u64>,
    sub_sessions: HashSet<String>,
    /// Main-session prompts and responses (`[start, end]`) over the whole read window.
    prompts: Vec<i64>,
    responses: Vec<(i64, i64)>,
    facts: AgentFacts,
    /// Any filtered activity in the read window.
    active: bool,
}

/// Session file → agent, for main sessions and everything under `<main without .jsonl>/`.
struct Sessions {
    main: HashMap<String, usize>,
    stems: HashMap<String, usize>,
}

impl Sessions {
    /// `(agent index, is a main session)`.
    fn resolve(&self, path: &str) -> Option<(usize, bool)> {
        if let Some(&i) = self.main.get(path) {
            return Some((i, true));
        }
        let mut p = path;
        while let Some(pos) = p.rfind('/') {
            p = &p[..pos];
            if let Some(&i) = self.stems.get(p) {
                return Some((i, false));
            }
        }
        None
    }
}

/// This host's partial report (not yet [`UsageReport::finish`]ed) and its agents' facts.
pub fn collect(inp: &Inputs, q: &Query, synced: Synced) -> (UsageReport, Vec<AgentFacts>) {
    let host = &inp.ctx.host_id;
    let lo = if q.compare { q.prev_from() } else { q.from };
    let cur = |ts: i64| ts >= q.from && ts < q.to;
    let tz = q.tz_offset;

    // ── Agents: ledger (incl. deleted) overlaid with the live state ──
    let mut agents: Vec<Agent> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for l in &inp.ledger {
        by_id.insert(l.id.clone(), agents.len());
        agents.push(Agent {
            id: l.id.clone(),
            title: l.title.clone(),
            project_id: l.project_id.clone(),
            parent_id: l.parent_id.clone(),
            created_at: l.created_at,
            deleted_at: l.deleted_at,
            session_files: l.session_files.clone(),
            ..Agent::default()
        });
    }
    for s in &inp.ctx.agents {
        let i = *by_id.entry(s.id.clone()).or_insert_with(|| {
            agents.push(Agent { id: s.id.clone(), ..Agent::default() });
            agents.len() - 1
        });
        let a = &mut agents[i];
        a.title = s.title.clone();
        a.project_id = s.project_id.clone().or(a.project_id.take());
        a.parent_id = s.parent_id.clone();
        a.created_at = s.created_at;
        a.deleted_at = None;
        a.archived = s.status == AgentStatus::Stopped;
        if let Some(file) = &s.session_file
            && !a.session_files.contains(file)
        {
            a.session_files.push(file.clone());
        }
    }
    let in_scope = |a: &Agent| q.project.is_none() || a.project_id == q.project;
    let mut sessions = Sessions { main: HashMap::new(), stems: HashMap::new() };
    for (i, a) in agents.iter().enumerate().filter(|(_, a)| in_scope(a)) {
        for file in &a.session_files {
            sessions.main.insert(file.clone(), i);
            sessions.stems.insert(file.strip_suffix(".jsonl").unwrap_or(file).to_owned(), i);
        }
    }

    let mut totals = Totals::default();
    let mut prev = Totals::default();
    let mut days: BTreeMap<i64, DayRow> = BTreeMap::new();
    let mut models: HashMap<(String, String), ModelRow> = HashMap::new();
    let mut tools: HashMap<String, ToolRow> = HashMap::new();
    let mut heatmap = vec![vec![0u64; 24]; 7];
    let mut frustration = Frustration::default();
    let mut all_models: HashSet<String> = HashSet::new();
    let mut active_days: HashSet<i64> = HashSet::new();
    let mut prev_active_days: HashSet<i64> = HashSet::new();
    let mut prev_subs: HashSet<String> = HashSet::new();
    let mut prev_agents: HashSet<usize> = HashSet::new();
    let mut earliest = i64::MAX;
    let model_ok = |m: Option<&str>| q.model.is_none() || q.model.as_deref() == m;
    let day = |days: &mut BTreeMap<i64, DayRow>, ts: i64| -> i64 {
        let index = day_index(ts, tz);
        days.entry(index).or_insert_with(|| DayRow { index, date: date(index), start: day_start(index, tz), ..DayRow::default() });
        index
    };

    // ── omp statistics ──
    let mut stats_error = synced.error.clone().map(|e| format!("{e}; the numbers may be outdated"));
    let mut schema = None;
    match &inp.db {
        Err(err) => stats_error = Some(err.clone()),
        Ok(db) => {
            schema = db.schema.clone();
            let keep = |f: &str| sessions.resolve(f).is_some();
            match db.messages(lo, q.to, keep) {
                Ok(rows) => {
                    for m in rows {
                        let Some((ai, main)) = sessions.resolve(&m.session_file) else { continue };
                        let now = cur(m.ts);
                        if now {
                            all_models.insert(m.model.clone());
                        }
                        if !model_ok(Some(&m.model)) {
                            continue;
                        }
                        let a = &mut agents[ai];
                        a.active = true;
                        let duration = m.duration.unwrap_or(0.0).max(0.0) as i64;
                        let cost = Cost {
                            total: m.cost_total,
                            input: m.cost_input,
                            output: m.cost_output,
                            cache_read: m.cost_cache_read,
                            cache_write: m.cost_cache_write,
                        };
                        let tokens = Tokens { input: m.input, output: m.output, cache_read: m.cache_read, cache_write: m.cache_write };
                        let failed = m.stop_reason == "error";
                        let aborted = m.stop_reason == "aborted";
                        let subagent = !main || m.agent_type == "subagent";
                        if main && m.agent_type == "main" {
                            a.responses.push((m.ts, m.ts + duration));
                            if aborted {
                                a.facts.aborted_ts.push(m.ts);
                            } else if failed {
                                a.facts.error_ts.push(m.ts);
                            }
                        }
                        let t = if now { &mut totals } else { &mut prev };
                        t.cost.add(&cost);
                        t.tokens.add(&tokens);
                        t.requests += 1;
                        t.failed += u64::from(failed);
                        t.aborted += u64::from(aborted);
                        if !now {
                            prev_agents.insert(ai);
                            prev_active_days.insert(day_index(m.ts, tz));
                            if subagent {
                                prev_subs.insert(m.session_file);
                            }
                            continue;
                        }
                        earliest = earliest.min(m.ts);
                        a.facts.cost += cost.total;
                        let d = day(&mut days, m.ts);
                        active_days.insert(d);
                        let dm = days.get_mut(&d).expect("day exists").models.entry(m.model.clone()).or_default();
                        dm.cost += cost.total;
                        dm.tokens += tokens.total();
                        dm.requests += 1;
                        let row = models.entry((m.model.clone(), m.provider.clone())).or_insert_with(|| ModelRow {
                            model: m.model.clone(),
                            provider: m.provider.clone(),
                            ..ModelRow::default()
                        });
                        row.requests += 1;
                        row.failed += u64::from(failed);
                        row.cost.add(&cost);
                        row.tokens.add(&tokens);
                        if let Some(ttft) = m.ttft.filter(|t| *t > 0.0) {
                            row.ttft_sum += ttft;
                            row.ttft_n += 1;
                            let generating = m.duration.unwrap_or(0.0) - ttft;
                            if generating > 50.0 && m.output > 0 {
                                row.tps_sum += m.output as f64 / (generating / 1000.0);
                                row.tps_n += 1;
                            }
                        }
                        let r = &mut a.row;
                        r.requests += 1;
                        r.failed += u64::from(failed);
                        r.cost.add(&cost);
                        match m.agent_type.as_str() {
                            "advisor" => r.cost_by.advisor += cost.total,
                            _ if subagent => r.cost_by.subagents += cost.total,
                            _ => r.cost_by.main += cost.total,
                        }
                        *r.requests_by.entry(m.agent_type.clone()).or_default() += 1;
                        *r.day_cost_by_index.entry(d).or_default() += cost.total;
                        *a.model_requests.entry(m.model).or_default() += 1;
                        if subagent && m.agent_type != "advisor" {
                            a.sub_sessions.insert(m.session_file);
                        }
                    }
                }
                Err(err) => stats_error = Some(format!("reading responses: {err}")),
            }
            match db.tool_calls(lo, q.to, keep) {
                Ok(rows) => {
                    for t in rows {
                        let Some((ai, _)) = sessions.resolve(&t.session_file) else { continue };
                        if !model_ok(Some(&t.model)) {
                            continue;
                        }
                        let a = &mut agents[ai];
                        if EDIT_TOOLS.contains(&t.name.as_str()) && a.facts.first_edit_at.is_none_or(|f| t.ts < f) {
                            a.facts.first_edit_at = Some(t.ts);
                        }
                        if !cur(t.ts) {
                            a.facts.tool_calls_prev += 1;
                            continue;
                        }
                        a.facts.tool_calls += 1;
                        a.row.tool_calls += 1;
                        let row = tools.entry(t.name.clone()).or_insert_with(|| ToolRow { name: t.name, ..ToolRow::default() });
                        row.calls += 1;
                        row.errors += u64::from(t.error);
                    }
                }
                Err(err) => stats_error = Some(format!("reading tool calls: {err}")),
            }
            match db.user_messages(lo, q.to, |f| sessions.main.contains_key(f)) {
                Ok(rows) => {
                    for u in rows {
                        let Some(&ai) = sessions.main.get(&u.session_file) else { continue };
                        if !model_ok(u.model.as_deref()) {
                            continue;
                        }
                        let a = &mut agents[ai];
                        a.active = true;
                        a.prompts.push(u.ts);
                        a.facts.prompt_ts.push(u.ts);
                        let flagged = u.signals.any();
                        if flagged {
                            a.facts.flagged_ts.push(u.ts);
                        }
                        if !cur(u.ts) {
                            prev.prompts += 1;
                            prev_active_days.insert(day_index(u.ts, tz));
                            continue;
                        }
                        earliest = earliest.min(u.ts);
                        totals.prompts += 1;
                        a.row.prompts += 1;
                        let d = day(&mut days, u.ts);
                        active_days.insert(d);
                        let row = days.get_mut(&d).expect("day exists");
                        row.prompts += 1;
                        let (wd, hour) = weekday_hour(u.ts, tz);
                        heatmap[wd][hour] += 1;
                        frustration.prompts += 1;
                        if flagged {
                            row.flagged += 1;
                            frustration.flagged += 1;
                            frustration.signals.add(&u.signals);
                            if let Some(text) = u.text {
                                frustration.examples.push(FlaggedPrompt {
                                    ts: u.ts,
                                    host_id: host.clone(),
                                    agent_id: Some(a.id.clone()),
                                    text,
                                    signals: u.signals.names(),
                                });
                            }
                        }
                    }
                }
                Err(err) => stats_error = Some(format!("reading prompts: {err}")),
            }
        }
    }

    // ── Turns: from each prompt to the last response before the next one ──
    let mut windows: Vec<Vec<(i64, i64)>> = vec![Vec::new(); agents.len()];
    for (i, a) in agents.iter_mut().enumerate() {
        a.prompts.sort_unstable();
        a.responses.sort_unstable();
        for (start, end, active) in turns(&a.prompts, &a.responses) {
            windows[i].push((start - COMMIT_SLACK_MS, end + COMMIT_SLACK_MS));
            a.facts.turns.push((start, end - start));
            if cur(start) {
                a.row.active_ms += active;
            }
        }
    }

    // ── Cards: done times and exactly recorded commits ──
    let project_ok = |pid: &str| q.project.as_deref().is_none_or(|p| p == pid);
    let mut exact: Vec<(String, usize)> = Vec::new();
    let mut from_card: HashSet<String> = HashSet::new();
    let mut project_cards: HashMap<String, u64> = HashMap::new();
    for (pid, board) in inp.boards.iter().filter(|(p, _)| project_ok(p)) {
        for c in &board.cards {
            let agent = c.agent_id.as_ref().and_then(|id| by_id.get(id).copied());
            if let Some(id) = &c.agent_id {
                from_card.insert(id.clone());
            }
            if let Some(id) = c.refinement.as_ref().and_then(|r| r.agent_id.as_ref()) {
                from_card.insert(id.clone());
            }
            if let (Some(commit), Some(ai)) = (&c.commit, agent) {
                exact.push((commit.sha.clone(), ai));
            }
            if c.column != CardColumn::Done {
                continue;
            }
            let (ts, approx) = match c.done_at {
                Some(ts) => (ts, false),
                None => (c.updated_at, true),
            };
            if ts < lo || ts >= q.to || (q.model.is_some() && !agent.is_some_and(|ai| agents[ai].active)) {
                continue;
            }
            if !cur(ts) {
                prev.cards_done += 1;
                continue;
            }
            earliest = earliest.min(ts);
            totals.cards_done += 1;
            *project_cards.entry(pid.clone()).or_default() += 1;
            let d = day(&mut days, ts);
            let row = days.get_mut(&d).expect("day exists");
            row.cards_done += 1;
            row.cards_done_approx += u64::from(approx);
            if let Some(ai) = agent {
                agents[ai].row.cards_done += 1;
            }
        }
    }

    // ── Commits of the projects the agents worked in ──
    let mut project_commits: HashMap<String, (u64, u64, u64)> = HashMap::new();
    let mut scan: HashSet<String> = HashSet::new();
    for (i, a) in agents.iter().enumerate() {
        if in_scope(a)
            && (!windows[i].is_empty() || exact.iter().any(|(_, ai)| *ai == i))
            && let Some(pid) = &a.project_id
        {
            scan.insert(pid.clone());
        }
    }
    for p in inp.ctx.projects.iter().filter(|p| scan.contains(&p.id)) {
        let commits = match git::log(Path::new(&p.path), lo, q.to) {
            Ok(commits) => commits,
            Err(err) => {
                tracing::debug!(project = %p.id, %err, "no commits for insights");
                continue;
            }
        };
        let members: Vec<usize> = agents.iter().enumerate().filter(|(_, a)| a.project_id.as_deref() == Some(p.id.as_str())).map(|(i, _)| i).collect();
        for c in commits {
            let exact_agent = exact.iter().find(|(sha, _)| !sha.is_empty() && c.sha.starts_with(sha.as_str())).map(|(_, ai)| *ai);
            let matched: Vec<(usize, CommitMatch)> = match exact_agent {
                Some(ai) if q.model.is_none() || agents[ai].active => vec![(ai, CommitMatch::Exact)],
                Some(_) => Vec::new(),
                None => {
                    let hits: Vec<usize> = members.iter().copied().filter(|&ai| windows[ai].iter().any(|&(s, e)| c.ts >= s && c.ts <= e)).collect();
                    let kind = if hits.len() > 1 { CommitMatch::Shared } else { CommitMatch::Turn };
                    hits.into_iter().map(|ai| (ai, kind)).collect()
                }
            };
            if matched.is_empty() {
                continue;
            }
            for &(ai, _) in &matched {
                agents[ai].facts.commit_ts.push(c.ts);
            }
            if !cur(c.ts) {
                prev.commits += 1;
                prev.lines_added += c.added;
                prev.lines_deleted += c.deleted;
                continue;
            }
            earliest = earliest.min(c.ts);
            totals.commits += 1;
            totals.lines_added += c.added;
            totals.lines_deleted += c.deleted;
            let e = project_commits.entry(p.id.clone()).or_default();
            e.0 += 1;
            e.1 += c.added;
            e.2 += c.deleted;
            let d = day(&mut days, c.ts);
            days.get_mut(&d).expect("day exists").commits += 1;
            for (ai, kind) in matched {
                let r = &mut agents[ai].row;
                r.lines_added += c.added;
                r.lines_deleted += c.deleted;
                r.commits.push(CommitRef {
                    sha: c.sha.clone(),
                    ts: c.ts,
                    summary: c.summary.clone(),
                    lines_added: c.added,
                    lines_deleted: c.deleted,
                    matched: kind,
                });
            }
        }
    }

    // ── Rows ──
    let mut children: HashMap<String, u64> = HashMap::new();
    for a in &agents {
        if let Some(parent) = &a.parent_id {
            *children.entry(parent.clone()).or_default() += 1;
        }
    }
    let mut rows: Vec<AgentRow> = Vec::new();
    let mut facts: Vec<AgentFacts> = Vec::new();
    let mut projects: HashMap<String, ProjectRow> = HashMap::new();
    for a in agents.iter_mut().filter(|a| q.project.is_none() || a.project_id == q.project) {
        let created_now = cur(a.created_at);
        let r = &a.row;
        let worked = r.requests > 0 || r.prompts > 0;
        let shown = worked || !r.commits.is_empty() || r.cards_done > 0 || (created_now && q.model.is_none());
        let relevant = a.active || (a.created_at >= lo && a.created_at < q.to) || a.deleted_at.is_some_and(|d| d >= lo && d < q.to);
        if relevant {
            let f = &mut a.facts;
            f.host_id = host.clone();
            f.id = a.id.clone();
            f.title = a.title.clone();
            f.project_id = a.project_id.clone();
            f.parent_id = a.parent_id.clone();
            f.created_at = a.created_at;
            f.deleted_at = a.deleted_at;
            f.archived = a.archived;
            f.from_card = from_card.contains(&a.id);
            f.children = children.get(&a.id).copied().unwrap_or(0);
            f.prompt_ts.sort_unstable();
            f.commit_ts.sort_unstable();
            facts.push(std::mem::take(f));
        }
        if !shown {
            continue;
        }
        if r.requests > 0 {
            totals.agents += 1;
        }
        totals.subagents += a.sub_sessions.len() as u64;
        totals.active_ms += r.active_ms;
        let mut used: Vec<(&String, &u64)> = a.model_requests.iter().collect();
        used.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
        let mut row = std::mem::take(&mut a.row);
        row.host_id = host.clone();
        row.id = a.id.clone();
        row.title = a.title.clone();
        row.project_id = a.project_id.clone();
        row.parent_id = a.parent_id.clone();
        row.created_at = a.created_at;
        row.deleted = a.deleted_at.is_some();
        row.archived = a.archived;
        row.models = used.into_iter().map(|(m, _)| m.clone()).collect();
        row.subagent_sessions = a.sub_sessions.len() as u64;
        row.session_files = a.session_files.clone();
        if let Some(pid) = &a.project_id {
            let p = projects.entry(pid.clone()).or_insert_with(|| ProjectRow {
                host_id: host.clone(),
                project_id: pid.clone(),
                name: project_name(&inp.ctx, pid),
                ..ProjectRow::default()
            });
            p.agents += u64::from(worked);
            p.cost += row.cost.total;
        }
        rows.push(row);
    }
    for (pid, (n, added, deleted)) in project_commits {
        let p = projects.entry(pid.clone()).or_insert_with(|| ProjectRow {
            host_id: host.clone(),
            project_id: pid.clone(),
            name: project_name(&inp.ctx, &pid),
            ..ProjectRow::default()
        });
        (p.commits, p.lines_added, p.lines_deleted) = (n, added, deleted);
    }
    for (pid, n) in project_cards {
        let p = projects.entry(pid.clone()).or_insert_with(|| ProjectRow {
            host_id: host.clone(),
            project_id: pid.clone(),
            name: project_name(&inp.ctx, &pid),
            ..ProjectRow::default()
        });
        p.cards_done = n;
    }
    totals.active_day_set = active_days.into_iter().collect();
    prev.agents = prev_agents.len() as u64;
    prev.subagents = prev_subs.len() as u64;
    prev.active_day_set = prev_active_days.into_iter().collect();

    // Every day of the period (from the first activity when the period is open-ended).
    let first = if q.from > 0 { q.from } else { earliest };
    if first < q.to && first != i64::MAX {
        for index in day_index(first, tz)..=day_index(q.to - 1, tz) {
            days.entry(index).or_insert_with(|| DayRow { index, date: date(index), start: day_start(index, tz), ..DayRow::default() });
        }
    }

    let source = HostSource {
        host_id: host.clone(),
        name: inp.ctx.host_name.clone(),
        ok: true,
        error: None,
        stats_db: Some(inp.stats.display().to_string()),
        synced_at: synced.at,
        schema,
        stats_error,
        log_since: inp.log_since,
        log_enabled: inp.log_enabled,
    };
    let report = UsageReport {
        query: Some(q.clone()),
        hosts: vec![source],
        totals,
        prev: q.compare.then_some(prev),
        days: days.into_values().collect(),
        models: models.into_values().collect(),
        projects: projects.into_values().collect(),
        agents: rows,
        tools: tools.into_values().collect(),
        heatmap,
        frustration,
        all_models: all_models.into_iter().collect(),
    };
    (report, facts)
}

fn project_name(ctx: &LocalContext, id: &str) -> String {
    ctx.projects.iter().find(|p| p.id == id).map_or_else(|| "(removed project)".to_owned(), |p| p.name.clone())
}

/// Turns as `(start, end, active ms)`: each prompt up to the last response before the next
/// prompt; responses before the first prompt (a turn that began earlier) form their own turn.
/// Active time sums the gaps between the prompt and the response ends, each capped at
/// [`IDLE_GAP_MS`].
fn turns(prompts: &[i64], responses: &[(i64, i64)]) -> Vec<(i64, i64, i64)> {
    let mut out = Vec::new();
    let mut r = 0;
    let close = |start: i64, until: i64, r: &mut usize| {
        let mut last = start;
        let mut active = 0;
        while *r < responses.len() && responses[*r].0 < until {
            let end = responses[*r].1.max(responses[*r].0);
            active += (end - last).clamp(0, IDLE_GAP_MS);
            last = last.max(end);
            *r += 1;
        }
        (start, last, active)
    };
    if let Some(&first) = prompts.first()
        && responses.first().is_some_and(|x| x.0 < first)
    {
        let start = responses[0].0;
        out.push(close(start, first, &mut r));
    } else if prompts.is_empty() && !responses.is_empty() {
        out.push(close(responses[0].0, i64::MAX, &mut r));
    }
    for (i, &p) in prompts.iter().enumerate() {
        let until = prompts.get(i + 1).copied().unwrap_or(i64::MAX);
        out.push(close(p, until, &mut r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_cap_idle_gaps_and_keep_earlier_turns() {
        let min = 60_000;
        // A turn running into the period, then two prompts; the second turn waits 30 min for
        // the user in the middle.
        let responses = [(0, min), (5 * min, 6 * min), (7 * min, 8 * min), (40 * min, 41 * min)];
        let t = turns(&[4 * min, 6 * min + 1], &responses);
        assert_eq!(t[0], (0, min, min));
        assert_eq!(t[1], (4 * min, 6 * min, 2 * min));
        assert_eq!(t[2], (6 * min + 1, 41 * min, (2 * min - 1) + IDLE_GAP_MS));
        assert_eq!(turns(&[], &[]), vec![]);
    }
}
