//! Graceful restarts (e.g. redeploying Coocoo from an agent working on Coocoo).
//!
//! [`Agents::drain`] runs before a restarting daemon stops its processes: every agent in a turn is
//! steered to pause, and the daemon waits until those turns ended (or `restartGraceSecs` passed).
//! Agents that paused, and those whose turn or dialog the stop cut off, are marked in `meta.json`
//! ([`RestartMark`]); [`Agents::resume_after_restart`] tells them on the next start that the
//! restart is done.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use coocoo_proto::ApiResult;
use omp_rpc::StreamingBehavior;

use crate::state::{Agent, PendingPrompt, RestartMark};
use crate::{Agents, write_meta};

/// How often the drain re-checks whether the asked agents ended their turns.
const DRAIN_POLL: Duration = Duration::from_millis(250);

const PAUSE: &str = "Coocoo is restarting (its daemon is being redeployed), which stops this agent's process. \
Pause now: do not start new tool calls or long-running commands, and end your turn with a short note on where you \
stopped and what is left. You will get a message once Coocoo is back; then continue.";

const RESUMED_PAUSED: &str =
    "Coocoo has restarted and is back. Continue the task you paused for the restart where you left off.";

const RESUMED_INTERRUPTED: &str = "Coocoo has restarted and is back. The restart cut off your previous turn: a \
running tool call or an open question to the user was cancelled and its result is lost. Check the state of your \
work (files, git status, processes you started), then continue where you left off; ask open questions again.";

impl Agents {
    /// Before a restart: refuse new prompts and agents, steer every agent in a turn to pause and
    /// wait until those turns ended or `restartGraceSecs` passed. Agents waiting for a user answer
    /// are not waited for. [`Agents::shutdown`] afterwards marks what it cuts off.
    pub async fn drain(self: &Arc<Self>) {
        if self.draining.swap(true, Ordering::AcqRel) {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(self.config.restart_grace_secs);
        let mut asked = HashSet::new();
        loop {
            let busy: Vec<Arc<Agent>> = self
                .agents
                .read()
                .values()
                .filter(|agent| {
                    let st = agent.state.lock();
                    st.is_live() && st.busy() && st.requests.is_empty()
                })
                .cloned()
                .collect();
            if busy.is_empty() {
                if !asked.is_empty() {
                    tracing::info!(agents = asked.len(), "agents paused for the restart");
                }
                return;
            }
            for agent in busy {
                if asked.insert(agent.id.clone()) {
                    let me = self.clone();
                    tokio::spawn(async move {
                        if let Err(err) = me.ask_to_pause(&agent).await {
                            tracing::warn!(agent = %agent.id, %err, "cannot ask the agent to pause");
                        }
                    });
                }
            }
            let now = Instant::now();
            if now >= deadline {
                tracing::warn!(
                    grace_secs = self.config.restart_grace_secs,
                    "agents still working after the restart grace period; stopping them"
                );
                return;
            }
            tokio::time::sleep(DRAIN_POLL.min(deadline - now)).await;
        }
    }

    /// Steer `agent` to end its turn; marks it to be told when the restart is done.
    async fn ask_to_pause(&self, agent: &Arc<Agent>) -> ApiResult<()> {
        let client = {
            let mut st = agent.state.lock();
            let Some(proc) = &st.proc else { return Ok(()) };
            let client = proc.client.clone();
            st.restart = Some(RestartMark::Paused);
            st.begin_prompt();
            client
        };
        self.changed(agent);
        let prompt = PendingPrompt { message: PAUSE.to_owned(), images: None, behavior: StreamingBehavior::Steer };
        self.send_prompt(agent, client, prompt).await
    }

    /// After a start: tell every agent the previous daemon paused or interrupted that the restart
    /// is done (wakes it). Must run inside the runtime, after the orchestrator is set.
    pub fn resume_after_restart(self: &Arc<Self>) {
        let marked: Vec<(Arc<Agent>, RestartMark)> = self
            .agents
            .read()
            .values()
            .filter_map(|agent| Some((agent.clone(), agent.state.lock().restart.take()?)))
            .collect();
        for (agent, mark) in marked {
            let me = self.clone();
            tokio::spawn(async move {
                let persisted = agent.clone();
                let _ = tokio::task::spawn_blocking(move || write_meta(&persisted)).await;
                let message = match mark {
                    RestartMark::Paused => RESUMED_PAUSED,
                    RestartMark::Interrupted => RESUMED_INTERRUPTED,
                };
                if let Err(err) = me.prompt_text(&agent.id, message.to_owned()).await {
                    tracing::warn!(agent = %agent.id, %err, "cannot tell the agent that the restart is done");
                }
            });
        }
    }
}
