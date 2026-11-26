//! Agents that share a working copy know of each other. A starting process gets the agents
//! working in the same directory in its system prompt ([`Agents::coworker_prompt`]), so a new
//! agent knows from its first step and a resumed one gets the current list. Agents already
//! running are told about a newcomer ([`Agents::announce_newcomer`]): as an `aside` while in a
//! turn, else at the start of their next turn, so nobody is woken for it.

use std::sync::Arc;

use coocoo_proto::AgentSummary;

use crate::Agents;
use crate::state::{Agent, Delivery, Life, PendingPrompt};

/// Co-workers named in a system prompt; the rest are counted.
const MAX_LISTED: usize = 10;

const RULES: &str = "Their changes are not yours: do not revert, discard, stash, reset, check out over or reformat \
them, commit only the files and hunks you changed yourself, and do not switch the branch. If your work overlaps \
with theirs, coordinate with `coocoo_send` (their agent id).";

/// Whether agents in `a` and `b` work in the same working copy: the same directory, or (within
/// one project's checkout) one inside the other. Worktrees have directories of their own.
fn same_working_copy(a: &AgentSummary, b: &AgentSummary) -> bool {
    let (x, y) = (a.cwd.trim_end_matches('/'), b.cwd.trim_end_matches('/'));
    let nested = |inner: &str, outer: &str| inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with('/'));
    x == y || (a.project_id.is_some() && a.project_id == b.project_id && (nested(x, y) || nested(y, x)))
}

fn describe(s: &AgentSummary) -> String {
    let status = serde_json::to_value(s.status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    let mut line = format!("- {} (agent `{}`, {status})", s.title, s.id);
    if let Some(task) = &s.task {
        line.push_str(": ");
        line.push_str(&task.text);
    }
    line
}

impl Agents {
    /// The other agents, not archived, that work in `me`'s working copy; most recently active first.
    fn coworkers(&self, me: &AgentSummary) -> Vec<(Arc<Agent>, AgentSummary)> {
        let mut found: Vec<(Arc<Agent>, AgentSummary)> = self
            .agents
            .read()
            .values()
            .filter(|a| a.id != me.id)
            .filter_map(|a| {
                let st = a.state.lock();
                (!st.deleted && st.life != Life::Stopped && same_working_copy(&st.summary, me)).then(|| (a.clone(), st.summary.clone()))
            })
            .collect();
        found.sort_by_key(|(_, s)| std::cmp::Reverse(s.last_activity_at));
        found
    }

    /// The system prompt paragraph for `agent`'s starting process; `None` while it works alone.
    pub(crate) fn coworker_prompt(&self, agent: &Agent) -> Option<String> {
        let me = agent.summary();
        let others = self.coworkers(&me);
        if others.is_empty() {
            return None;
        }
        let mut text = format!(
            "You are not the only agent in this working copy ({}): its files, git index and branch are shared with these other Coocoo agents:\n",
            me.cwd
        );
        for (_, s) in others.iter().take(MAX_LISTED) {
            text.push_str(&describe(s));
            text.push('\n');
        }
        if others.len() > MAX_LISTED {
            text.push_str(&format!("- and {} more (`coocoo_agents` lists them)\n", others.len() - MAX_LISTED));
        }
        text.push_str(RULES);
        Some(text)
    }

    /// Tell the agents already working in the new `agent`'s working copy that it joined.
    pub(crate) fn announce_newcomer(&self, agent: &Agent) {
        let new = agent.summary();
        let notice = format!(
            "Another Coocoo agent now works in your working copy ({}): {} (agent `{}`). {RULES} No reply needed; continue your task.",
            new.cwd, new.title, new.id
        );
        for (other, _) in self.coworkers(&new) {
            other.state.lock().notices.push(notice.clone());
            self.deliver_notices(&other);
        }
    }

    /// Send `agent`'s queued notices as one `aside` while it is in a turn (it reads them at its
    /// next step); an idle agent keeps them until its next turn starts.
    pub(crate) fn deliver_notices(&self, agent: &Arc<Agent>) {
        let Some(me) = self.me.upgrade() else { return };
        let (client, message) = {
            let mut st = agent.state.lock();
            if !st.streaming || st.notices.is_empty() {
                return;
            }
            let Some(proc) = &st.proc else { return };
            let client = proc.client.clone();
            let message = std::mem::take(&mut st.notices).join("\n\n");
            st.begin_prompt();
            (client, message)
        };
        let agent = agent.clone();
        self.handle().spawn(async move {
            let prompt = PendingPrompt { message, images: None, delivery: Delivery::Aside };
            if let Err(err) = me.send_prompt(&agent, client, prompt).await {
                tracing::warn!(agent = %agent.id, %err, "cannot tell the agent about a co-worker");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(cwd: &str, project: Option<&str>) -> AgentSummary {
        let mut s: AgentSummary = serde_json::from_value(serde_json::json!({
            "id": "a", "hostId": "h", "title": "t", "cwd": cwd, "status": "idle", "createdAt": 0, "lastActivityAt": 0,
            "unread": false, "pendingRequests": 0, "subagents": 0
        }))
        .unwrap();
        s.project_id = project.map(str::to_owned);
        s
    }

    #[test]
    fn working_copies() {
        let main = agent("/p/app", Some("p"));
        assert!(same_working_copy(&main, &agent("/p/app/", Some("p"))));
        assert!(same_working_copy(&main, &agent("/p/app/ui", Some("p"))), "a subdirectory of the same checkout");
        assert!(!same_working_copy(&main, &agent("/wt/app-feature", Some("p"))), "a worktree of the project");
        assert!(!same_working_copy(&main, &agent("/p/application", Some("p"))));
        // Without a project a parent directory (say $HOME) does not claim everything below it.
        assert!(!same_working_copy(&agent("/home/u", None), &agent("/home/u/x", None)));
        assert!(same_working_copy(&agent("/home/u/x", None), &agent("/home/u/x", None)));
    }
}
