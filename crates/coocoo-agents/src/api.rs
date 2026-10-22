//! `agents.*` request handlers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{new_id, now_ms};
use coocoo_proto::{
    AgentStatus, AgentSummary, ApiError, ApiResult, ImageInput, ItemKind, ModelInfo, Project, WorktreeRef, events,
};
use omp_rpc::{
    AbortCommand, Client, ClientOptions, ExtensionUiResponse, GetAvailableModelsCommand, ImageContent, LitImage,
    RpcInbound, SetModelCommand, SetThinkingLevelCommand, StreamingBehavior, ThinkingLevel,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::process::{call, client_error};
use crate::state::{Agent, Life, PendingPrompt, State};
use crate::timeline;
use crate::{Agents, write_meta};

/// `agents.models` cache lifetime.
const MODELS_TTL: Duration = Duration::from_secs(600);
const TITLE_CHARS: usize = 60;
const DEFAULT_PAGE: usize = 200;
const MAX_PAGE: usize = 2000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdParams {
    id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    project_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WorktreeParam {
    Enabled(bool),
    Spec {
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        base: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateParams {
    project_id: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    prompt: Option<String>,
    worktree: Option<WorktreeParam>,
    model: Option<String>,
    thinking: Option<String>,
    parent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptParams {
    id: String,
    message: String,
    mode: Option<String>,
    #[serde(default)]
    images: Vec<ImageInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondParams {
    id: String,
    request_id: String,
    response: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineParams {
    id: String,
    before: Option<u64>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameParams {
    id: String,
    title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelParams {
    id: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingParams {
    id: String,
    level: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteParams {
    id: String,
    #[serde(default)]
    remove_worktree: bool,
}

/// Default title: first non-empty prompt line, at most [`TITLE_CHARS`] characters.
fn default_title(prompt: Option<&str>, id: &str) -> String {
    let line = prompt.and_then(|p| p.lines().map(str::trim).find(|l| !l.is_empty()));
    match line {
        Some(line) if line.chars().count() > TITLE_CHARS => {
            let mut title: String = line.chars().take(TITLE_CHARS - 1).collect();
            title.push('…');
            title
        }
        Some(line) => line.to_owned(),
        None => format!("Agent {}", &id[..id.len().min(6)]),
    }
}

fn image_content(image: ImageInput) -> ImageContent {
    ImageContent {
        r#type: Some(LitImage),
        data: Some(image.data),
        mime_type: Some(image.mime_type),
        ..ImageContent::default()
    }
}

fn parse_thinking(level: &str) -> ApiResult<ThinkingLevel> {
    serde_json::from_value(Value::String(level.to_owned()))
        .map_err(|_| ApiError::invalid(format!("unknown thinking level `{level}`")))
}

impl Agents {
    pub(crate) async fn handle_method(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "agents.list" => self.list(params(p)?),
            "agents.get" => {
                let IdParams { id } = params(p)?;
                ok(self.get(&id)?.state.lock().detail())
            }
            "agents.create" => ok(self.create(params(p)?).await?),
            "agents.prompt" => self.prompt(params(p)?).await,
            "agents.abort" => self.abort(params(p)?).await,
            "agents.respond" => self.respond(params(p)?),
            "agents.timeline" => self.timeline(params(p)?).await,
            "agents.output" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                let st = agent.state.lock();
                Ok(json!({ "status": st.summary.status, "text": st.last_output }))
            }
            "agents.markRead" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                let changed = std::mem::replace(&mut agent.state.lock().summary.unread, false);
                if changed {
                    self.changed(&agent);
                }
                Ok(json!({}))
            }
            "agents.rename" => {
                let RenameParams { id, title } = params(p)?;
                let title = title.trim();
                if title.is_empty() {
                    return Err(ApiError::invalid("title must not be empty"));
                }
                let agent = self.get(&id)?;
                agent.state.lock().summary.title = title.to_owned();
                self.changed(&agent);
                ok(agent.summary())
            }
            "agents.setModel" => self.set_model(params(p)?).await,
            "agents.setThinking" => self.set_thinking(params(p)?).await,
            "agents.models" => ok(self.models().await?),
            "agents.hibernate" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                self.stop_process(&agent, Life::Hibernated, Some("Hibernated".to_owned())).await?;
                ok(agent.summary())
            }
            "agents.archive" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                self.stop_process(&agent, Life::Stopped, Some("Archived".to_owned())).await?;
                ok(agent.summary())
            }
            "agents.delete" => self.delete(params(p)?).await,
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    fn list(&self, p: ListParams) -> ApiResult<Value> {
        let mut list: Vec<AgentSummary> = self
            .agents
            .read()
            .values()
            .map(|a| a.summary())
            .filter(|s| p.project_id.is_none() || s.project_id == p.project_id)
            .collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.last_activity_at));
        ok(list)
    }

    async fn create(self: &Arc<Self>, p: CreateParams) -> ApiResult<AgentSummary> {
        let orchestrator = self.orch()?;
        let mut project_id = p.project_id.filter(|s| !s.is_empty());
        let project: Option<Project> = match &project_id {
            Some(id) => Some(serde_json::from_value(
                orchestrator.call(None, "projects.get".to_owned(), json!({ "id": id })).await?,
            )?),
            None => None,
        };
        let mut cwd = match (p.cwd.filter(|c| !c.is_empty()), &project) {
            (Some(cwd), _) => cwd,
            (None, Some(project)) => project.path.clone(),
            (None, None) => return Err(ApiError::invalid("`projectId` or `cwd` is required")),
        };
        if project_id.is_none() {
            // Attribute a bare directory to the project containing it (browser profile, lists).
            if let Ok(Value::Array(projects)) = orchestrator.call(None, "projects.list".to_owned(), Value::Null).await {
                project_id = projects
                    .iter()
                    .filter_map(|p| Some((p.get("id")?.as_str()?, p.get("path")?.as_str()?)))
                    .filter(|(_, path)| {
                        let path = path.trim_end_matches('/');
                        cwd == path || cwd.strip_prefix(path).is_some_and(|rest| rest.starts_with('/'))
                    })
                    .max_by_key(|(_, path)| path.len())
                    .map(|(id, _)| id.to_owned());
            }
        }
        let worktree = match p.worktree {
            None | Some(WorktreeParam::Enabled(false)) => None,
            Some(WorktreeParam::Enabled(true)) => Some((None, None)),
            Some(WorktreeParam::Spec { branch, base }) => Some((branch, base)),
        };
        let worktree: Option<WorktreeRef> = match worktree {
            None => None,
            Some((branch, base)) => {
                let project_id = project_id.clone().ok_or_else(|| ApiError::invalid("a worktree requires `projectId`"))?;
                let mut params = json!({ "projectId": project_id });
                if let Some(branch) = branch.filter(|b| !b.is_empty()) {
                    params["branch"] = Value::String(branch);
                }
                if let Some(base) = base.filter(|b| !b.is_empty()) {
                    params["base"] = Value::String(base);
                }
                let wt: WorktreeRef =
                    serde_json::from_value(orchestrator.call(None, "git.worktreeCreate".to_owned(), params).await?)?;
                cwd = wt.path.clone();
                Some(wt)
            }
        };
        match tokio::fs::metadata(&cwd).await {
            Ok(meta) if meta.is_dir() => {}
            _ => return Err(ApiError::invalid(format!("`{cwd}` is not a directory"))),
        }

        let id = new_id();
        let prompt = p.prompt.filter(|s| !s.trim().is_empty());
        let title = p
            .title
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| default_title(prompt.as_deref(), &id));
        let now = now_ms();
        let summary = AgentSummary {
            id: id.clone(),
            host_id: self.host_id().to_owned(),
            project_id,
            title,
            cwd,
            worktree,
            status: AgentStatus::Starting,
            model: p.model.filter(|m| !m.is_empty()).or_else(|| self.config.default_model.clone()),
            thinking: p.thinking.filter(|t| !t.is_empty()).or_else(|| self.config.default_thinking.clone()),
            created_at: now,
            last_activity_at: now,
            preview: None,
            unread: false,
            context_percent: None,
            cost_usd: None,
            tokens: None,
            pending_requests: 0,
            parent_id: p.parent_id.filter(|s| !s.is_empty()),
            error: None,
            todos: None,
            session_file: None,
            diff: None,
            subagents: 0,
        };
        let mut state = State::new(summary, Life::Starting);
        if let Some(message) = prompt {
            state.pending.push(PendingPrompt { message, images: None, behavior: StreamingBehavior::FollowUp });
            state.begin_prompt();
        }
        let agent = Arc::new(Agent::new(self.root.join(&id), state));
        let persisted = agent.clone();
        tokio::task::spawn_blocking(move || write_meta(&persisted)).await.map_err(ApiError::internal)?;
        self.agents.write().insert(id, agent.clone());
        let summary = agent.summary();
        self.emitter.emit(events::AGENT_UPDATED, &summary);

        let me = self.clone();
        tokio::spawn(async move {
            // Failures are recorded on the agent (status error + timeline item).
            let _ = me.ensure_live(&agent).await;
        });
        Ok(summary)
    }

    async fn prompt(self: &Arc<Self>, p: PromptParams) -> ApiResult<Value> {
        if p.message.trim().is_empty() && p.images.is_empty() {
            return Err(ApiError::invalid("message must not be empty"));
        }
        let behavior = match p.mode.as_deref() {
            None | Some("auto" | "followUp") => StreamingBehavior::FollowUp,
            Some("steer") => StreamingBehavior::Steer,
            Some(other) => return Err(ApiError::invalid(format!("unknown mode `{other}`"))),
        };
        let agent = self.get(&p.id)?;
        {
            let mut st = agent.state.lock();
            if st.life == Life::Stopped {
                return Err(ApiError::conflict("the agent is archived"));
            }
            st.begin_prompt();
        }
        self.changed(&agent);
        let client = match self.ensure_live(&agent).await {
            Ok(client) => client,
            Err(err) => {
                agent.state.lock().cancel_prompt();
                self.changed(&agent);
                return Err(err);
            }
        };
        let images = (!p.images.is_empty()).then(|| p.images.into_iter().map(image_content).collect());
        self.send_prompt(&agent, client, PendingPrompt { message: p.message, images, behavior }).await?;
        Ok(json!({}))
    }

    async fn abort(&self, p: IdParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        if let Some(client) = agent.client() {
            call(client, AbortCommand {}).await?;
        }
        Ok(json!({}))
    }

    fn respond(&self, p: RespondParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let Value::Object(mut body) = p.response else {
            return Err(ApiError::invalid("`response` must be an object"));
        };
        body.insert("id".to_owned(), Value::String(p.request_id.clone()));
        let response = ExtensionUiResponse::from_value(Value::Object(body)).map_err(|e| ApiError::invalid(e.to_string()))?;
        let client = {
            let st = agent.state.lock();
            if !st.requests.iter().any(|r| r.id == p.request_id) {
                return Err(ApiError::not_found(format_args!("request {}", p.request_id)));
            }
            st.proc.as_ref().map(|proc| proc.client.clone())
        };
        let client = client.ok_or_else(|| ApiError::conflict("the agent process is not running"))?;
        // Queues the frame for the writer thread; does not block.
        client.send(&RpcInbound::ExtensionUiResponse(response)).map_err(client_error)?;
        if agent.state.lock().resolve_request(&p.request_id).is_some() {
            self.emit_resolved(&agent.id, &p.request_id);
            self.changed(&agent);
        }
        Ok(json!({}))
    }

    async fn timeline(&self, p: TimelineParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let limit = p.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let live = agent.state.lock().is_live();
        let (path, tail) = {
            let tl = agent.timeline.lock();
            (tl.path().to_owned(), tl.tail())
        };
        let page = tokio::task::spawn_blocking(move || timeline::page(&path, tail, p.before, limit, live))
            .await
            .map_err(ApiError::internal)?;
        ok(page)
    }

    async fn set_model(&self, p: ModelParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let (provider, model_id) = p
            .model
            .split_once('/')
            .filter(|(provider, id)| !provider.is_empty() && !id.is_empty())
            .ok_or_else(|| ApiError::invalid("model must be `provider/modelId`"))?;
        let model = match agent.client() {
            Some(client) => {
                let info = call(client, SetModelCommand { provider: provider.to_owned(), model_id: model_id.to_owned() })
                    .await
                    .map_err(|e| ApiError::invalid(e.message))?;
                format!("{}/{}", info.provider, info.id)
            }
            None => p.model.clone(),
        };
        agent.state.lock().summary.model = Some(model.clone());
        self.push_item(&agent, ItemKind::Notice, format!("Model: {model}")).await;
        self.changed(&agent);
        ok(agent.summary())
    }

    async fn set_thinking(&self, p: ThinkingParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let level = p.level.trim();
        match agent.client() {
            Some(client) => {
                let parsed = parse_thinking(level)?;
                call(client, SetThinkingLevelCommand { level: parsed }).await.map_err(|e| ApiError::invalid(e.message))?;
            }
            // `auto` is valid on the command line (applied at the next start) but not over RPC.
            None if level == "auto" => {}
            None => {
                parse_thinking(level)?;
            }
        }
        agent.state.lock().summary.thinking = Some(level.to_owned());
        self.changed(&agent);
        ok(agent.summary())
    }

    async fn models(&self) -> ApiResult<Vec<ModelInfo>> {
        let mut cache = self.models.lock().await;
        if let Some((at, models)) = cache.as_ref()
            && at.elapsed() < MODELS_TTL
        {
            return Ok(models.clone());
        }
        let live = self.agents.read().values().find_map(|a| a.client());
        let raw = match live {
            Some(client) => call(client, GetAvailableModelsCommand {}).await?,
            None => {
                let (omp, cwd, extra) = (self.omp.clone(), self.root.clone(), self.config.extra_args.clone());
                tokio::task::spawn_blocking(move || -> ApiResult<Vec<omp_rpc::ModelInfo>> {
                    std::fs::create_dir_all(&cwd)?;
                    let mut process = std::process::Command::new(omp);
                    process
                        .args(["--mode", "rpc", "--no-session", "--allow-home", "--cwd"])
                        .arg(&cwd)
                        .args(extra)
                        .current_dir(&cwd)
                        .stderr(std::process::Stdio::null());
                    let (client, _events) = Client::spawn(process, ClientOptions::default()).map_err(client_error)?;
                    let models = client.call(&GetAvailableModelsCommand {}).map_err(client_error);
                    // Dropping the client tears the process group down.
                    drop(client);
                    models
                })
                .await
                .map_err(ApiError::internal)??
            }
        };
        let models: Vec<ModelInfo> = raw
            .into_iter()
            .map(|m| ModelInfo {
                provider: m.provider,
                id: m.id,
                name: Some(m.name).filter(|n| !n.is_empty()),
                context_window: m.context_window.and_then(|w| u64::try_from(w).ok()),
                reasoning: m.reasoning,
            })
            .collect();
        *cache = Some((Instant::now(), models.clone()));
        Ok(models)
    }

    async fn delete(&self, p: DeleteParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        // Hibernate first: if removing the worktree fails, the agent stays intact and resumable.
        self.stop_process(&agent, Life::Hibernated, None).await?;
        let (project_id, worktree) = {
            let st = agent.state.lock();
            (st.summary.project_id.clone(), st.summary.worktree.clone())
        };
        if p.remove_worktree
            && let (Some(project_id), Some(worktree)) = (project_id, worktree)
        {
            self.orch()?
                .call(None, "git.worktreeRemove".to_owned(), json!({ "projectId": project_id, "path": worktree.path }))
                .await?;
        }
        {
            let _lifecycle = agent.lifecycle.lock().await;
            let (proc, _) = {
                let mut st = agent.state.lock();
                st.deleted = true;
                st.take_process(Life::Stopped)
            };
            self.agents.write().remove(&agent.id);
            let doomed = agent.clone();
            tokio::task::spawn_blocking(move || {
                drop(proc);
                let _guard = doomed.meta_write.lock();
                doomed.timeline.lock().close();
                if let Err(err) = std::fs::remove_dir_all(&doomed.dir)
                    && err.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(agent = %doomed.id, %err, "failed to remove agent data");
                }
            })
            .await
            .map_err(ApiError::internal)?;
        }
        self.emitter.emit(events::AGENT_REMOVED, &json!({ "id": agent.id }));
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(default_title(Some("\n  Fix the login bug\nmore"), "abcdef123456"), "Fix the login bug");
        assert_eq!(default_title(None, "abcdef123456"), "Agent abcdef");
        let long = "x".repeat(100);
        let title = default_title(Some(&long), "abcdef123456");
        assert_eq!(title.chars().count(), TITLE_CHARS);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn ui_responses_decode() {
        for (body, expect) in [
            (json!({ "value": "b" }), "value"),
            (json!({ "confirmed": true }), "confirmed"),
            (json!({ "cancelled": true }), "cancelled"),
            (json!({ "answers": [{ "id": "q", "selectedOptions": ["A"] }] }), "answers"),
        ] {
            let mut body = body;
            body["id"] = json!("ui1");
            let response = ExtensionUiResponse::from_value(body).unwrap();
            let wire = serde_json::to_value(RpcInbound::ExtensionUiResponse(response)).unwrap();
            assert_eq!(wire["type"], "extension_ui_response");
            assert_eq!(wire["id"], "ui1");
            assert!(wire.get(expect).is_some(), "{wire}");
        }
    }
}
