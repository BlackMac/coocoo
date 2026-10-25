//! `agents.*` request handlers.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use coocoo_proto::error::{ok, params};
use coocoo_proto::insights::slash_name;
use coocoo_proto::util::{new_id, now_ms};
use coocoo_proto::{
    AgentStatus, AgentSummary, ApiError, ApiResult, ImageInput, ItemKind, ModelInfo, Project, ProviderUsage, SlashCommand,
    WorktreeRef, events,
};
use omp_rpc::{
    AbortCommand, Client, ClientOptions, ExtensionUiResponse, GetAvailableCommandsCommand, GetAvailableModelsCommand,
    ImageContent, LitImage, RpcInbound, SetModelCommand, SetThinkingLevelCommand, StreamingBehavior, ThinkingLevel,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::process::{call, client_error};
use crate::state::{Agent, Life, PendingPrompt, State};
use crate::timeline;
use crate::{Agents, transcript, usage, write_meta};

/// `agents.models` cache lifetime.
const MODELS_TTL: Duration = Duration::from_secs(600);
const TITLE_CHARS: usize = 60;
/// Length cap of the title part of a default worktree branch.
const BRANCH_TITLE_CHARS: usize = 40;
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
    /// Why the daemon created it (`card`, `schedule`; usage log only, not a request parameter).
    #[serde(skip)]
    source: Option<&'static str>,
    /// Notice shown at the top of its chat (not a request parameter).
    #[serde(skip)]
    notice: Option<String>,
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
    /// With `remove_worktree`: also delete the worktree's branch.
    #[serde(default)]
    delete_branch: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageParams {
    provider: String,
    /// Model id within the provider; marks the windows it draws from.
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentTimelineParams {
    id: String,
    /// `Subagent.sessionFile` from the agent's timeline.
    session_file: String,
    /// The subagent still runs: unanswered tool calls are in progress, not interrupted.
    #[serde(default)]
    live: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageParams {
    id: String,
    /// `TimelineImage.name`.
    name: String,
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

/// Default worktree branch: `coocoo/<title-slug>-<first 4 id chars>`, e.g.
/// `coocoo/git-status-push-pr-1a2b` (lowercase ASCII words, German umlauts transliterated, at most
/// [`BRANCH_TITLE_CHARS`]); `coocoo/<id>` without a title or with nothing usable in it.
fn worktree_branch(title: Option<&str>, id: &str) -> String {
    let mut ascii = String::new();
    for c in title.unwrap_or_default().chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => ascii.push_str("ae"),
            'ö' => ascii.push_str("oe"),
            'ü' => ascii.push_str("ue"),
            'ß' => ascii.push_str("ss"),
            c if c.is_ascii_alphanumeric() => ascii.push(c),
            _ => ascii.push(' '),
        }
    }
    let mut slug = String::new();
    for word in ascii.split_whitespace() {
        if !slug.is_empty() {
            if slug.len() + 1 + word.len() > BRANCH_TITLE_CHARS {
                break;
            }
            slug.push('-');
        }
        slug.push_str(word);
    }
    slug.truncate(BRANCH_TITLE_CHARS);
    if slug.is_empty() {
        return format!("coocoo/{id}");
    }
    format!("coocoo/{slug}-{}", &id[..id.len().min(4)])
}

fn image_content(image: ImageInput) -> ImageContent {
    ImageContent {
        r#type: Some(LitImage),
        data: Some(image.data),
        mime_type: Some(image.mime_type),
        ..ImageContent::default()
    }
}

/// MIME type of a stored user message image name (`<seq>-<index>.<ext>`). Anything else is
/// rejected, so a name can never point outside the agent's image directory.
fn message_image_mime(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.split_once('.')?;
    let (seq, index) = stem.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(seq) && digits(index)) {
        return None;
    }
    crate::image::mime(ext)
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
            "agents.commands" => {
                let IdParams { id } = params(p)?;
                ok(self.commands(&id).await?)
            }
            "agents.usage" => ok(self.usage(params(p)?).await?),
            "agents.subagentTimeline" => self.subagent_timeline(params(p)?).await,
            "agents.image" => self.image(params(p)?).await,
            "agents.hibernate" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                self.hibernate(&agent, "Hibernated".to_owned()).await?;
                ok(agent.summary())
            }
            "agents.archive" => {
                let IdParams { id } = params(p)?;
                let agent = self.get(&id)?;
                self.stop_process(&agent, Life::Stopped, Some("Archived".to_owned())).await?;
                ok(agent.summary())
            }
            "agents.delete" => self.delete(params(p)?).await,
            m if m.starts_with("boards.") => self.board_method(m, p).await,
            m if m.starts_with("schedules.") => self.schedule_method(m, p).await,
            m if m.starts_with("secrets.") => self.secret_method(m, p).await,
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

    /// A new agent in `project_id` that starts on `prompt` right away, created by the daemon for
    /// `source` (`card`, `schedule`); `notice` opens its chat.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_started(
        self: &Arc<Self>,
        project_id: &str,
        title: &str,
        prompt: String,
        worktree: bool,
        model: Option<String>,
        source: &'static str,
        notice: Option<String>,
    ) -> ApiResult<AgentSummary> {
        self.create(CreateParams {
            project_id: Some(project_id.to_owned()),
            cwd: None,
            title: Some(title.chars().take(TITLE_CHARS).collect()),
            prompt: Some(prompt),
            worktree: worktree.then_some(WorktreeParam::Enabled(true)),
            model,
            thinking: None,
            parent_id: None,
            source: Some(source),
            notice,
        })
        .await
    }

    async fn create(self: &Arc<Self>, p: CreateParams) -> ApiResult<AgentSummary> {
        self.refuse_while_draining()?;
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
        let id = new_id();
        let prompt = p.prompt.filter(|s| !s.trim().is_empty());
        let title = p.title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        // A title of its own (given or the prompt's first line) names the worktree branch.
        let named = title.is_some() || prompt.is_some();
        let title = title.unwrap_or_else(|| default_title(prompt.as_deref(), &id));
        let worktree = match p.worktree {
            None | Some(WorktreeParam::Enabled(false)) => None,
            Some(WorktreeParam::Enabled(true)) => Some((None, None)),
            Some(WorktreeParam::Spec { branch, base }) => Some((branch, base)),
        };
        let worktree: Option<WorktreeRef> = match worktree {
            None => None,
            Some((branch, base)) => {
                let project_id = project_id.clone().ok_or_else(|| ApiError::invalid("a worktree requires `projectId`"))?;
                let branch = branch
                    .map(|b| b.trim().to_owned())
                    .filter(|b| !b.is_empty())
                    .unwrap_or_else(|| worktree_branch(named.then_some(title.as_str()), &id));
                let mut params = json!({ "projectId": project_id, "branch": branch });
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
            task: None,
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
        self.record_ledger(&summary, None);
        let source = match (&summary.parent_id, p.source) {
            (Some(_), _) => "agent",
            (None, Some(source)) => source,
            (None, None) => "user",
        };
        self.record_agent(&agent, "agent.created", |event| {
            let event = event.meta("source", source).meta("worktree", summary.worktree.is_some());
            match &summary.parent_id {
                Some(parent) => event.meta("parentId", parent.as_str()),
                None => event,
            }
        });

        if let Some(notice) = p.notice {
            // Before the process starts, so it precedes the prompt in the chat.
            self.push_item(&agent, ItemKind::Notice, notice).await;
        }
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
        self.refuse_while_draining()?;
        // A builtin that makes no sense through Coocoo: explain it in the chat, wake nothing.
        if let Some((command, reason)) = crate::slash::refused(p.message.trim_start()) {
            if agent.state.lock().life == Life::Stopped {
                return Err(ApiError::conflict("the agent is archived"));
            }
            self.push_item(&agent, ItemKind::User, p.message.trim().to_owned()).await;
            self.push_item(&agent, ItemKind::Error, format!("{command} is not available in Coocoo. {reason}")).await;
            self.record_agent(&agent, "command.refused", |event| event.meta("command", slash_name(&p.message)));
            return Ok(json!({}));
        }
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

    /// Prompt (or queue a follow-up for) agent `id` with plain text.
    pub(crate) async fn prompt_text(self: &Arc<Self>, id: &str, message: String) -> ApiResult<Value> {
        self.prompt(PromptParams { id: id.to_owned(), message, mode: None, images: Vec::new() }).await
    }

    /// During a restart's drain no new turn may start ([`Agents::drain`]).
    pub(crate) fn refuse_while_draining(&self) -> ApiResult<()> {
        if self.draining.load(Ordering::Acquire) {
            return Err(ApiError::unavailable("Coocoo is restarting; send this again once it is back"));
        }
        Ok(())
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
        let cancelled = body.get("cancelled") == Some(&Value::Bool(true));
        // A secret request is Coocoo's own, not omp's: it is declined here or answered with
        // `secrets.provide` (so the value never passes through this method).
        let secret = agent.state.lock().requests.iter().any(|r| r.id == p.request_id && r.method == "secret");
        if secret {
            if !cancelled {
                return Err(ApiError::invalid("answer a secret request with secrets.provide"));
            }
            return self.decline_secret(&agent, &p.request_id);
        }
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
        let resolved = agent.state.lock().resolve_request(&p.request_id);
        if let Some(request) = resolved {
            self.emit_resolved(&agent.id, &p.request_id);
            self.request_closed(&agent, &request, cancelled.then_some("user"));
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
        let previous = agent.state.lock().summary.model.replace(model.clone());
        if previous.as_deref() != Some(model.as_str()) {
            self.record_model_change(&agent, previous.as_deref(), &model, "user");
        }
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

    /// Run one RPC command on a throwaway `omp --mode rpc` process in `cwd` (for data only a
    /// process can answer while no agent runs).
    async fn one_shot<C>(&self, cwd: std::path::PathBuf, command: C) -> ApiResult<C::Output>
    where
        C: omp_rpc::Command + Send + 'static,
        C::Output: Send + 'static,
    {
        let (omp, extra) = (self.omp.clone(), self.config.extra_args.clone());
        tokio::task::spawn_blocking(move || -> ApiResult<C::Output> {
            std::fs::create_dir_all(&cwd)?;
            let mut process = std::process::Command::new(omp);
            process
                .args(["--mode", "rpc", "--no-session", "--allow-home", "--cwd"])
                .arg(&cwd)
                .args(extra)
                .current_dir(&cwd)
                .stderr(std::process::Stdio::null());
            let (client, _events) = Client::spawn(process, ClientOptions::default()).map_err(client_error)?;
            let output = client.call(&command).map_err(client_error);
            // Dropping the client tears the process group down.
            drop(client);
            output
        })
        .await
        .map_err(ApiError::internal)?
    }

    /// `agents.commands`: the agent's slash-command catalog. Comes from its omp process (kept
    /// current by `available_commands_update`); an agent that never reported one gets it from a
    /// throwaway process in its directory, since skills and custom commands depend on it.
    async fn commands(&self, id: &str) -> ApiResult<Vec<SlashCommand>> {
        let agent = self.get(id)?;
        if let Some(commands) = agent.state.lock().commands.clone() {
            return Ok(commands);
        }
        let raw = match agent.client() {
            Some(client) => call(client, GetAvailableCommandsCommand {}).await?,
            None => self.one_shot(agent.summary().cwd.into(), GetAvailableCommandsCommand {}).await?,
        };
        let commands = crate::mapper::slash_commands(raw);
        agent.state.lock().commands = Some(commands.clone());
        self.changed(&agent);
        Ok(commands)
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
            None => self.one_shot(self.root.clone(), GetAvailableModelsCommand {}).await?,
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

    async fn usage(&self, UsageParams { provider, model }: UsageParams) -> ApiResult<ProviderUsage> {
        if !usage::valid_provider(&provider) {
            return Err(ApiError::invalid(format!("invalid provider id `{provider}`")));
        }
        let mut report = {
            let mut cache = self.usage.lock().await;
            match cache.get(&provider) {
                Some((at, report)) if at.elapsed() < usage::USAGE_TTL => report.clone(),
                _ => {
                    let report = usage::fetch(&self.omp, &self.root, &provider, now_ms()).await?;
                    cache.insert(provider, (Instant::now(), report.clone()));
                    report
                }
            }
        };
        usage::mark_gating(&mut report, model.as_deref());
        Ok(report)
    }

    async fn subagent_timeline(&self, p: SubagentTimelineParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let (parent, project_id) = {
            let st = agent.state.lock();
            (st.summary.session_file.clone(), st.summary.project_id.clone())
        };
        let parent = parent.ok_or_else(|| ApiError::not_found(format_args!("session of agent {}", p.id)))?;
        // omp's session files hold tool output unmasked.
        let redactor = self.project_redactor(project_id.as_deref());
        let page = tokio::task::spawn_blocking(move || {
            let file = transcript::resolve(std::path::Path::new(&parent), std::path::Path::new(&p.session_file))?;
            let mut page = transcript::read(&file, p.live)?;
            for item in &mut page.items {
                redactor.redact_item(item);
            }
            ApiResult::Ok(page)
        })
        .await
        .map_err(ApiError::internal)??;
        ok(page)
    }

    /// `agents.image`: one user message image as `{ mime, data }` (base64).
    async fn image(&self, p: ImageParams) -> ApiResult<Value> {
        use base64::Engine as _;
        let mime = message_image_mime(&p.name).ok_or_else(|| ApiError::invalid(format!("invalid image name `{}`", p.name)))?;
        let path = self.get(&p.id)?.dir.join(crate::image::DIR).join(&p.name);
        let bytes = tokio::fs::read(path).await?;
        Ok(json!({ "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
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
                .call(
                    None,
                    "git.worktreeRemove".to_owned(),
                    json!({ "projectId": project_id, "path": worktree.path, "deleteBranch": p.delete_branch }),
                )
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
        self.record_agent(&agent, "agent.deleted", |event| event);
        self.record_ledger(&agent.summary(), Some(now_ms()));
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
    fn worktree_branches() {
        let id = "1a2b3c4d5e6f";
        assert_eq!(worktree_branch(Some("Git-Status: Push & PR"), id), "coocoo/git-status-push-pr-1a2b");
        assert_eq!(worktree_branch(Some("Lesbare Branch-Namen für Größe"), id), "coocoo/lesbare-branch-namen-fuer-groesse-1a2b");
        // Whole words up to the cap; a single overlong word is cut.
        let long = worktree_branch(Some("Worktree branches get readable names derived from the card title"), id);
        assert_eq!(long, "coocoo/worktree-branches-get-readable-names-1a2b");
        assert_eq!(worktree_branch(Some(&"x".repeat(100)), id), format!("coocoo/{}-1a2b", "x".repeat(BRANCH_TITLE_CHARS)));
        assert_eq!(worktree_branch(Some("🚀 …"), id), "coocoo/1a2b3c4d5e6f");
        assert_eq!(worktree_branch(None, id), "coocoo/1a2b3c4d5e6f");
    }

    #[test]
    fn image_names_cannot_leave_the_image_directory() {
        assert_eq!(message_image_mime("12-0.png"), Some("image/png"));
        assert_eq!(message_image_mime("3-2.webp"), Some("image/webp"));
        for bad in ["../x", "a/b.png", "../1-0.png", "1-0/../../x.png", "1-0.svg", "1-0.png.png", "x-0.png", "1-.jpg", "-0.gif", "10.png", "1-0", ""] {
            assert_eq!(message_image_mime(bad), None, "{bad}");
        }
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
