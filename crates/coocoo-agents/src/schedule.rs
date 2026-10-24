//! Scheduled agent runs (`schedules.*`): per project, a cron expression or a one-off time at
//! which a new agent starts on a fixed prompt, or one of the project's agents gets it. Stored in
//! `<data_dir>/schedules.json`; they run while the daemon runs. A run missed while it was down
//! happens once when it is back.
//!
//! The scheduler sleeps until the earliest next run (at most [`MAX_SLEEP`], since the wall clock
//! can jump, e.g. after a suspend); with no enabled schedule it only waits for a change.

use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::insights::InsightEvent;
use coocoo_proto::util::{new_id, now_ms, read_json, write_json_atomic};
use coocoo_proto::{AgentRef, AgentStatus, ApiError, ApiResult, ItemKind, RunOutcome, Schedule, ScheduleRun, SchedulesEvent, events};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Agents;
use crate::cron::{self, Cron};
use crate::state::Life;

const MAX_NAME: usize = 100;
const MAX_PROMPT: usize = 32 * 1024;
/// Runs of one schedule are at least this far apart (a new agent per minute helps nobody).
pub(crate) const MIN_GAP_MINS: u32 = 5;
const DEFAULT_PREVIEW: usize = 5;
const MAX_PREVIEW: usize = 20;
const MAX_SLEEP: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    project_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddParams {
    project_id: String,
    name: String,
    prompt: String,
    cron: Option<String>,
    at: Option<i64>,
    timezone: Option<String>,
    agent_id: Option<String>,
    #[serde(default)]
    worktree: bool,
    model: Option<String>,
    enabled: Option<bool>,
    /// The agent creating it (`coocoo_schedule_save`).
    by_agent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateParams {
    project_id: String,
    schedule_id: String,
    name: Option<String>,
    prompt: Option<String>,
    cron: Option<String>,
    at: Option<i64>,
    timezone: Option<String>,
    /// `""`: a new agent per run.
    agent_id: Option<String>,
    worktree: Option<bool>,
    /// `""`: the default model.
    model: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleParams {
    project_id: String,
    schedule_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewParams {
    cron: String,
    timezone: Option<String>,
    count: Option<usize>,
}

fn non_empty(s: &impl AsRef<str>) -> bool {
    !s.as_ref().trim().is_empty()
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid("`name` must not be empty"));
    }
    Ok(name.chars().take(MAX_NAME).collect())
}

fn clean_prompt(prompt: &str) -> ApiResult<String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(ApiError::invalid("`prompt` must not be empty"));
    }
    if prompt.len() > MAX_PROMPT {
        return Err(ApiError::invalid(format!("`prompt` is longer than {} KiB", MAX_PROMPT / 1024)));
    }
    Ok(prompt.to_owned())
}

/// A valid cron expression whose runs are at least [`MIN_GAP_MINS`] apart.
fn parse_cron(spec: &str) -> ApiResult<Cron> {
    let cron: Cron = spec.parse().map_err(ApiError::invalid)?;
    if cron.min_gap_minutes() < MIN_GAP_MINS {
        return Err(ApiError::invalid(format!("runs would be less than {MIN_GAP_MINS} minutes apart")));
    }
    Ok(cron)
}

fn check_zone(name: &str) -> ApiResult<String> {
    let name = name.trim();
    cron::zone(name).map_err(ApiError::invalid)?;
    Ok(name.to_owned())
}

fn check_at(at: i64, now: i64) -> ApiResult<i64> {
    if at <= now {
        return Err(ApiError::invalid("the time of a one-off run must lie in the future"));
    }
    Ok(at)
}

/// When `s` runs next after `after`; `None` when disabled or nothing is left.
fn next_run(s: &Schedule, after: i64) -> Option<i64> {
    if !s.enabled {
        return None;
    }
    if let Some(at) = s.at {
        return (at > after).then_some(at);
    }
    let cron: Cron = s.cron.as_deref()?.parse().ok()?;
    let tz = cron::zone(&s.timezone).ok()?;
    cron.next_after(Timestamp::from_millisecond(after).ok()?, &tz).map(|t| t.as_millisecond())
}

fn outcome_name(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Started => "started",
        RunOutcome::Skipped => "skipped",
        RunOutcome::Failed => "failed",
    }
}

/// Schedules of `<data_dir>/schedules.json`; an unreadable file starts empty (and is replaced on
/// the next change).
pub(crate) fn load(path: &std::path::Path) -> Vec<Schedule> {
    match read_json::<Vec<Schedule>>(path) {
        Ok(list) => list.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(path = %path.display(), "unreadable schedules: {err}");
            Vec::new()
        }
    }
}

impl Agents {
    pub(crate) async fn schedule_method(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "schedules.list" => {
                let ListParams { project_id } = params(p)?;
                let list = self.schedules.lock();
                ok(list.iter().filter(|s| project_id.as_ref().is_none_or(|id| *id == s.project_id)).collect::<Vec<_>>())
            }
            "schedules.add" => ok(self.schedule_add(params(p)?).await?),
            "schedules.update" => ok(self.schedule_update(params(p)?).await?),
            "schedules.remove" => {
                let p: ScheduleParams = params(p)?;
                self.edit_schedules(|list| {
                    let before = list.len();
                    list.retain(|s| !(s.id == p.schedule_id && s.project_id == p.project_id));
                    if list.len() == before {
                        return Err(ApiError::not_found(format_args!("schedule {}", p.schedule_id)));
                    }
                    Ok(((), vec![p.project_id.clone()]))
                })?;
                Ok(json!({}))
            }
            "schedules.run" => {
                let p: ScheduleParams = params(p)?;
                let schedule = self.find_schedule(&p.project_id, &p.schedule_id)?;
                self.run_schedule(schedule, true).await;
                ok(self.find_schedule(&p.project_id, &p.schedule_id)?)
            }
            "schedules.preview" => {
                let p: PreviewParams = params(p)?;
                let cron = parse_cron(&p.cron)?;
                let tz = cron::zone(&p.timezone.filter(non_empty).unwrap_or_else(cron::host_zone)).map_err(ApiError::invalid)?;
                let mut next = Vec::new();
                let mut after = Timestamp::now();
                for _ in 0..p.count.unwrap_or(DEFAULT_PREVIEW).clamp(1, MAX_PREVIEW) {
                    let Some(at) = cron.next_after(after, &tz) else { break };
                    next.push(at.as_millisecond());
                    after = at;
                }
                Ok(json!({ "next": next }))
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    fn find_schedule(&self, project_id: &str, schedule_id: &str) -> ApiResult<Schedule> {
        let list = self.schedules.lock();
        let found = list.iter().find(|s| s.id == schedule_id && s.project_id == project_id);
        found.cloned().ok_or_else(|| ApiError::not_found(format_args!("schedule {schedule_id}")))
    }

    /// The agent a schedule prompts: one of the project's, not archived.
    fn schedule_target(&self, project_id: &str, agent_id: &str) -> ApiResult<AgentRef> {
        let agent = self.get(agent_id)?;
        let summary = agent.summary();
        if summary.project_id.as_deref() != Some(project_id) {
            return Err(ApiError::invalid(format!("agent “{}” does not belong to this project", summary.title)));
        }
        if agent.state.lock().life == Life::Stopped {
            return Err(ApiError::conflict(format!("agent “{}” is archived", summary.title)));
        }
        Ok(AgentRef { id: summary.id, title: summary.title })
    }

    async fn schedule_add(&self, p: AddParams) -> ApiResult<Schedule> {
        let project = self.project(&p.project_id).await?;
        let (name, prompt) = (clean_name(&p.name)?, clean_prompt(&p.prompt)?);
        let timezone = match p.timezone.filter(non_empty) {
            Some(zone) => check_zone(&zone)?,
            None => cron::host_zone(),
        };
        let now = now_ms();
        let (cron, at) = match (p.cron.filter(non_empty), p.at) {
            (Some(spec), None) => {
                parse_cron(&spec)?;
                (Some(spec.trim().to_owned()), None)
            }
            (None, Some(at)) => (None, Some(check_at(at, now)?)),
            _ => return Err(ApiError::invalid("give either `cron` or `at`")),
        };
        let target = p.agent_id.filter(non_empty).map(|id| self.schedule_target(&p.project_id, &id)).transpose()?;
        if target.is_none() && p.worktree && project.git.is_none() {
            return Err(ApiError::invalid(format!("“{}” is not a git repository, so runs cannot get their own worktree", project.name)));
        }
        let added_by = p.by_agent_id.filter(non_empty).map(|id| self.get(&id)).transpose()?.map(|a| {
            let s = a.summary();
            AgentRef { id: s.id, title: s.title }
        });
        let mut schedule = Schedule {
            id: new_id(),
            project_id: p.project_id.clone(),
            name,
            prompt,
            cron,
            at,
            timezone,
            worktree: target.is_none() && p.worktree,
            model: if target.is_none() { p.model.filter(non_empty) } else { None },
            agent_id: target.as_ref().map(|t| t.id.clone()),
            agent_title: target.map(|t| t.title),
            enabled: p.enabled.unwrap_or(true),
            next_run_at: None,
            last_run: None,
            created_at: now,
            updated_at: now,
            added_by,
        };
        schedule.next_run_at = next_run(&schedule, now);
        if schedule.enabled && schedule.next_run_at.is_none() {
            return Err(ApiError::invalid("this schedule would never run"));
        }
        self.edit_schedules(|list| {
            list.push(schedule.clone());
            Ok((schedule, vec![p.project_id.clone()]))
        })
    }

    async fn schedule_update(&self, p: UpdateParams) -> ApiResult<Schedule> {
        let project = self.project(&p.project_id).await?;
        let name = p.name.as_deref().map(clean_name).transpose()?;
        let prompt = p.prompt.as_deref().map(clean_prompt).transpose()?;
        let timezone = p.timezone.as_deref().filter(|z| !z.trim().is_empty()).map(check_zone).transpose()?;
        let cron = p.cron.as_deref().filter(|c| !c.trim().is_empty()).map(|c| parse_cron(c).map(|_| c.trim().to_owned())).transpose()?;
        if cron.is_some() && p.at.is_some() {
            return Err(ApiError::invalid("give either `cron` or `at`"));
        }
        let now = now_ms();
        let at = p.at.map(|at| check_at(at, now)).transpose()?;
        let target = p.agent_id.as_deref().map(|id| if id.is_empty() { Ok(None) } else { self.schedule_target(&p.project_id, id).map(Some) }).transpose()?;
        let retimed = cron.is_some() || at.is_some() || timezone.is_some() || p.enabled.is_some();
        self.edit_schedules(|list| {
            let current = list
                .iter_mut()
                .find(|s| s.id == p.schedule_id && s.project_id == p.project_id)
                .ok_or_else(|| ApiError::not_found(format_args!("schedule {}", p.schedule_id)))?;
            let mut s = current.clone();
            if let Some(name) = name {
                s.name = name;
            }
            if let Some(prompt) = prompt {
                s.prompt = prompt;
            }
            if let Some(cron) = cron {
                (s.cron, s.at) = (Some(cron), None);
            }
            if let Some(at) = at {
                (s.cron, s.at) = (None, Some(at));
            }
            if let Some(zone) = timezone {
                s.timezone = zone;
            }
            if let Some(target) = target {
                s.agent_id = target.as_ref().map(|t| t.id.clone());
                s.agent_title = target.map(|t| t.title);
            }
            if let Some(worktree) = p.worktree {
                s.worktree = worktree;
            }
            if let Some(model) = &p.model {
                s.model = Some(model.trim().to_owned()).filter(non_empty);
            }
            if s.agent_id.is_some() {
                (s.worktree, s.model) = (false, None);
            }
            if s.worktree && project.git.is_none() {
                return Err(ApiError::invalid(format!("“{}” is not a git repository, so runs cannot get their own worktree", project.name)));
            }
            if let Some(enabled) = p.enabled {
                s.enabled = enabled;
            }
            if retimed {
                if s.enabled && s.at.is_some_and(|at| at <= now) {
                    return Err(ApiError::invalid("this one-off run is over; pick a new time to run it again"));
                }
                s.next_run_at = next_run(&s, now);
                if s.enabled && s.next_run_at.is_none() {
                    return Err(ApiError::invalid("this schedule would never run"));
                }
            }
            s.updated_at = now;
            *current = s.clone();
            Ok((s, vec![p.project_id.clone()]))
        })
    }

    /// Run `f` on all schedules; persist them, announce the lists of the projects it reports as
    /// changed and wake the scheduler.
    fn edit_schedules<R>(&self, f: impl FnOnce(&mut Vec<Schedule>) -> ApiResult<(R, Vec<String>)>) -> ApiResult<R> {
        let mut list = self.schedules.lock();
        let (result, mut changed) = f(&mut list)?;
        if changed.is_empty() {
            return Ok(result);
        }
        write_json_atomic(&self.schedules_file, &*list)?;
        changed.sort_unstable();
        changed.dedup();
        for project_id in changed {
            let schedules = list.iter().filter(|s| s.project_id == project_id).cloned().collect();
            self.emitter.emit(events::PROJECT_SCHEDULES, &SchedulesEvent { project_id, schedules });
        }
        self.schedule_wake.notify_one();
        Ok(result)
    }

    /// Start the scheduler task (once, at startup).
    pub(crate) fn start_scheduler(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        let wake = self.schedule_wake.clone();
        self.handle().spawn(async move {
            loop {
                let wait = {
                    let Some(agents) = me.upgrade() else { return };
                    agents.run_due_schedules();
                    let next = agents.schedules.lock().iter().filter_map(|s| s.next_run_at).min();
                    // While draining for a restart nothing starts; the next daemon catches up.
                    if agents.draining.load(std::sync::atomic::Ordering::Acquire) {
                        Some(MAX_SLEEP)
                    } else {
                        next.map(|at| Duration::from_millis((at - now_ms()).clamp(0, MAX_SLEEP.as_millis() as i64) as u64))
                    }
                };
                match wait {
                    Some(wait) => {
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = wake.notified() => {}
                        }
                    }
                    None => wake.notified().await,
                }
            }
        });
    }

    /// Start every due run. Each schedule moves on to its next time (a one-off one is disabled)
    /// before its run starts, so none runs twice and missed runs collapse into one.
    fn run_due_schedules(self: &Arc<Self>) {
        if self.draining.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let now = now_ms();
        let due = self.edit_schedules(|list| {
            let mut due = Vec::new();
            let mut changed = Vec::new();
            for s in list.iter_mut().filter(|s| s.next_run_at.is_some_and(|at| at <= now)) {
                due.push(s.clone());
                if s.at.is_some() {
                    s.enabled = false;
                }
                s.next_run_at = next_run(s, now);
                changed.push(s.project_id.clone());
            }
            Ok((due, changed))
        });
        let due = due.unwrap_or_else(|err| {
            tracing::warn!(%err, "could not save the schedules; due runs skipped");
            Vec::new()
        });
        for schedule in due {
            let me = self.clone();
            self.handle().spawn(async move { me.run_schedule(schedule, false).await });
        }
    }

    /// Run `schedule` now and record the outcome as its `lastRun`.
    async fn run_schedule(self: &Arc<Self>, schedule: Schedule, manual: bool) -> ScheduleRun {
        let run = self.start_run(&schedule, manual).await;
        self.record(|| {
            let event = InsightEvent::new("schedule.run")
                .project(Some(schedule.project_id.as_str()))
                .meta("target", if schedule.agent_id.is_some() { "agent" } else { "new" })
                .meta("outcome", outcome_name(run.outcome))
                .meta("manual", manual);
            match &run.agent_id {
                Some(id) => event.agent(id.as_str()),
                None => event,
            }
        });
        let saved = self.edit_schedules(|list| match list.iter_mut().find(|s| s.id == schedule.id) {
            Some(s) => {
                s.last_run = Some(run.clone());
                Ok(((), vec![s.project_id.clone()]))
            }
            // Deleted meanwhile.
            None => Ok(((), Vec::new())),
        });
        if let Err(err) = saved {
            tracing::warn!(schedule = %schedule.id, %err, "could not save the run of a schedule");
        }
        run
    }

    async fn start_run(self: &Arc<Self>, s: &Schedule, manual: bool) -> ScheduleRun {
        let mut run = ScheduleRun { at: now_ms(), outcome: RunOutcome::Started, agent_id: None, agent_title: None, message: None, manual };
        let notice = format!("Started by the schedule “{}”{}", s.name, if manual { " (run now)" } else { "" });
        let failed = |mut run: ScheduleRun, message: String| {
            run.outcome = RunOutcome::Failed;
            run.message = Some(message);
            run
        };
        let Some(agent_id) = &s.agent_id else {
            return match self.create_started(&s.project_id, &s.name, s.prompt.clone(), s.worktree, s.model.clone(), "schedule", Some(notice)).await {
                Ok(agent) => {
                    (run.agent_id, run.agent_title) = (Some(agent.id), Some(agent.title));
                    run
                }
                Err(err) => failed(run, err.message),
            };
        };
        (run.agent_id, run.agent_title) = (Some(agent_id.clone()), s.agent_title.clone());
        let Ok(agent) = self.get(agent_id) else { return failed(run, "the agent no longer exists".to_owned()) };
        let summary = agent.summary();
        run.agent_title = Some(summary.title);
        if agent.state.lock().life == Life::Stopped {
            return failed(run, "the agent is archived".to_owned());
        }
        // A scheduled prompt never queues behind (or steers) a running answer.
        if matches!(summary.status, AgentStatus::Starting | AgentStatus::Running | AgentStatus::Waiting) {
            run.outcome = RunOutcome::Skipped;
            run.message = Some(if summary.status == AgentStatus::Waiting { "the agent was waiting for an answer" } else { "the agent was busy" }.to_owned());
            return run;
        }
        if let Err(err) = self.refuse_while_draining() {
            return failed(run, err.message);
        }
        self.push_item(&agent, ItemKind::Notice, notice).await;
        match self.prompt_text(agent_id, s.prompt.clone()).await {
            Ok(_) => run,
            Err(err) => failed(run, err.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(cron: Option<&str>, at: Option<i64>, enabled: bool) -> Schedule {
        Schedule {
            id: "s".into(),
            project_id: "p".into(),
            name: "n".into(),
            prompt: "p".into(),
            cron: cron.map(Into::into),
            at,
            timezone: "Europe/Berlin".into(),
            agent_id: None,
            agent_title: None,
            worktree: false,
            model: None,
            enabled,
            next_run_at: None,
            last_run: None,
            created_at: 0,
            updated_at: 0,
            added_by: None,
        }
    }

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn next_run_follows_cron_in_its_zone_and_one_off_runs_once() {
        let daily = schedule(Some("0 8 * * *"), None, true);
        assert_eq!(next_run(&daily, ms("2026-10-24T07:00:00Z")), Some(ms("2026-10-25T07:00:00Z")));
        assert_eq!(next_run(&schedule(Some("0 8 * * *"), None, false), 0), None);
        let once = schedule(None, Some(ms("2026-11-01T08:00:00Z")), true);
        assert_eq!(next_run(&once, ms("2026-10-24T07:00:00Z")), Some(ms("2026-11-01T08:00:00Z")));
        assert_eq!(next_run(&once, ms("2026-11-01T08:00:00Z")), None);
    }

    #[test]
    fn rejects_schedules_that_run_too_often() {
        assert!(parse_cron("*/5 * * * *").is_ok());
        let err = parse_cron("* * * * *").unwrap_err();
        assert!(err.message.contains("less than 5 minutes"), "{}", err.message);
        assert!(parse_cron("0,2 9 * * *").is_err());
    }
}
