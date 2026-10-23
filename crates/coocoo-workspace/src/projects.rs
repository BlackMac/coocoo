//! Projects registry, method dispatch and git operations bound to projects.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{new_id, now_ms, read_json, write_json_atomic};
use coocoo_proto::{ApiError, ApiResult, Emitter, Paths, Project, WorktreeRef, events};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fs::expand;
use crate::watch::{Kind, Queue, Watches};
use crate::{Workspace, actions, diff, fs, git, icon, watch};

#[derive(Default)]
pub(crate) struct State {
    pub projects: Vec<Project>,
    /// Project id → git metadata paths currently watched for it.
    watched: HashMap<String, Vec<PathBuf>>,
    /// Project id → paths watched for its actions file.
    watched_actions: HashMap<String, Vec<PathBuf>>,
}

pub(crate) fn create(emitter: Emitter, paths: &Paths) -> Arc<Workspace> {
    let file = paths.data_dir.join("projects.json");
    let mut projects: Vec<Project> = match read_json(&file) {
        Ok(list) => list.unwrap_or_default(),
        Err(err) => {
            let backup = file.with_extension("json.corrupt");
            tracing::warn!("cannot read {}: {err}; moved aside to {}", file.display(), backup.display());
            let _ = std::fs::rename(&file, &backup);
            Vec::new()
        }
    };
    for project in &mut projects {
        project.host_id = emitter.host_id().to_owned();
        project.git = None;
        project.icon = None;
        project.actions = Vec::new();
        project.action_errors = Vec::new();
    }
    let ws = Arc::new(Workspace {
        emitter,
        data_dir: paths.data_dir.clone(),
        file,
        state: Mutex::new(State { projects, ..State::default() }),
        watches: Watches::new(),
        refresh_locks: Mutex::new(HashMap::new()),
        started: AtomicBool::new(false),
    });
    if tokio::runtime::Handle::try_current().is_ok() {
        ws.ensure_started();
    }
    ws
}

#[derive(Deserialize)]
struct IdParams {
    id: String,
}

#[derive(Deserialize)]
struct AddParams {
    path: String,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateParams {
    id: String,
    name: Option<String>,
    dev_urls: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct CwdBaseParams {
    cwd: String,
    base: Option<String>,
}

#[derive(Deserialize)]
struct LogParams {
    cwd: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct CommitParams {
    cwd: String,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeParams {
    project_id: String,
    branch: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectParams {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FastForwardParams {
    project_id: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeCreateParams {
    project_id: String,
    branch: Option<String>,
    base: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeRemoveParams {
    project_id: String,
    path: String,
    #[serde(default)]
    delete_branch: bool,
}

#[derive(Deserialize)]
struct FsListParams {
    path: Option<String>,
}

/// Emit `project.updated` only when something visible changed, or always.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Emit {
    IfChanged,
    Always,
}

impl Workspace {
    pub(crate) async fn handle(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        self.ensure_started();
        match method {
            "projects.list" => ok(self.projects()),
            "projects.get" => ok(self.get(&params::<IdParams>(p)?.id)?),
            "projects.add" => {
                let p: AddParams = params(p)?;
                ok(self.add(&p.path, p.name).await?)
            }
            "projects.update" => ok(self.update(params(p)?)?),
            "projects.remove" => {
                self.remove(&params::<IdParams>(p)?.id)?;
                ok(json!({}))
            }
            "projects.refresh" => ok(self.refresh(&params::<IdParams>(p)?.id, Emit::IfChanged).await?),
            "projects.icon" => {
                let project = self.get(&params::<IdParams>(p)?.id)?;
                let found = project.icon.as_ref().ok_or_else(|| ApiError::not_found(format!("icon of {}", project.name)))?;
                ok(icon::read(Path::new(&project.path), found).await?)
            }
            "git.diff" => {
                let p: CwdBaseParams = params(p)?;
                ok(diff::diff(&expand(&p.cwd), p.base.as_deref()).await?)
            }
            "git.diffStat" => {
                let p: CwdBaseParams = params(p)?;
                ok(diff::diff_stat(&expand(&p.cwd), p.base.as_deref()).await?)
            }
            "git.log" => {
                let p: LogParams = params(p)?;
                let top = git::toplevel(&expand(&p.cwd)).await?;
                let limit = p.limit.unwrap_or(20).clamp(1, 1000);
                ok(git::log(&top, limit).await?.into_iter().map(|e| e.commit).collect::<Vec<_>>())
            }
            "git.commit" => {
                let p: CommitParams = params(p)?;
                ok(self.commit(&expand(&p.cwd), &p.message).await?)
            }
            "git.merge" => {
                let p: MergeParams = params(p)?;
                let (ok_, output) = self.merge(&p.project_id, &p.branch).await?;
                ok(json!({ "ok": ok_, "output": output }))
            }
            "git.fetch" => {
                let (ok_, output) = self.fetch(&params::<ProjectParams>(p)?.project_id).await?;
                ok(json!({ "ok": ok_, "output": output }))
            }
            "git.fastForward" => {
                let p: FastForwardParams = params(p)?;
                let (ok_, output) = self.fast_forward(&p.project_id, &p.path).await?;
                ok(json!({ "ok": ok_, "output": output }))
            }
            "git.worktreeCreate" => {
                let p: WorktreeCreateParams = params(p)?;
                ok(self.worktree_create(&p.project_id, p.branch, p.base).await?)
            }
            "git.worktreeRemove" => {
                let p: WorktreeRemoveParams = params(p)?;
                self.worktree_remove(&p.project_id, &p.path, p.delete_branch).await?;
                ok(json!({}))
            }
            "fs.list" => ok(fs::list(params::<FsListParams>(p)?.path.as_deref()).await?),
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    /// Start the debounced refresh task and the initial git scan (once, inside a runtime).
    fn ensure_started(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        tokio::spawn(refresh_loop(Arc::downgrade(self), self.watches.queue()));
        for project in self.projects() {
            self.watches.schedule(&project.id, Kind::Git);
        }
    }

    pub(crate) fn get(&self, id: &str) -> ApiResult<Project> {
        self.state
            .lock()
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("project {id}")))
    }

    /// Persist the registry (git info, icon and actions are runtime state and not stored).
    fn save(&self, projects: &[Project]) -> ApiResult<()> {
        let stored: Vec<Project> = projects
            .iter()
            .map(|p| Project { git: None, icon: None, actions: Vec::new(), action_errors: Vec::new(), ..p.clone() })
            .collect();
        write_json_atomic(&self.file, &stored).map_err(ApiError::from)
    }

    async fn add(self: &Arc<Self>, path: &str, name: Option<String>) -> ApiResult<Project> {
        let raw = expand(path);
        let dir = tokio::fs::canonicalize(&raw)
            .await
            .map_err(|_| ApiError::not_found(format!("directory {}", raw.display())))?;
        if !tokio::fs::metadata(&dir).await?.is_dir() {
            return Err(ApiError::invalid(format!("{} is not a directory", dir.display())));
        }
        let path = dir.to_str().ok_or_else(|| ApiError::invalid("path is not valid UTF-8"))?.to_owned();
        let name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());

        let existing = self.state.lock().projects.iter().find(|p| p.path == path).map(|p| p.id.clone());
        if let Some(id) = existing {
            if let Some(name) = name {
                self.update(UpdateParams { id: id.clone(), name: Some(name), dev_urls: None })?;
            }
            return self.refresh(&id, Emit::IfChanged).await;
        }

        let name = name.unwrap_or_else(|| {
            dir.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned())
        });
        let project = Project {
            id: new_id(),
            host_id: self.emitter.host_id().to_owned(),
            name,
            path,
            added_at: now_ms(),
            git: None,
            dev_urls: Vec::new(),
            icon: None,
            actions: Vec::new(),
            action_errors: Vec::new(),
        };
        {
            let mut state = self.state.lock();
            state.projects.push(project.clone());
            self.save(&state.projects)?;
        }
        self.refresh(&project.id, Emit::Always).await
    }

    fn update(&self, p: UpdateParams) -> ApiResult<Project> {
        let project = {
            let mut state = self.state.lock();
            let project = state
                .projects
                .iter_mut()
                .find(|x| x.id == p.id)
                .ok_or_else(|| ApiError::not_found(format!("project {}", p.id)))?;
            if let Some(name) = p.name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(ApiError::invalid("name must not be empty"));
                }
                project.name = name.to_owned();
            }
            if let Some(urls) = p.dev_urls {
                project.dev_urls = urls.into_iter().map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()).collect();
            }
            let project = project.clone();
            self.save(&state.projects)?;
            project
        };
        self.emitter.emit(events::PROJECT_UPDATED, &project);
        Ok(project)
    }

    fn remove(&self, id: &str) -> ApiResult<()> {
        let (git_paths, action_paths) = {
            let mut state = self.state.lock();
            let before = state.projects.len();
            state.projects.retain(|p| p.id != id);
            if state.projects.len() == before {
                return Err(ApiError::not_found(format!("project {id}")));
            }
            self.save(&state.projects)?;
            (state.watched.remove(id).unwrap_or_default(), state.watched_actions.remove(id).unwrap_or_default())
        };
        self.watches.remove(id, Kind::Git, &git_paths);
        self.watches.remove(id, Kind::Actions, &action_paths);
        self.refresh_locks.lock().remove(id);
        self.emitter.emit(events::PROJECT_REMOVED, &json!({ "id": id }));
        Ok(())
    }

    fn refresh_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.refresh_locks.lock().entry(id.to_owned()).or_default().clone()
    }

    /// Recompute a project's git info, icon and actions, re-sync its watches and emit when it
    /// changed.
    async fn refresh(&self, id: &str, emit: Emit) -> ApiResult<Project> {
        let lock = self.refresh_lock(id);
        let _guard = lock.lock().await;
        let path = PathBuf::from(self.get(id)?.path);
        let (info, found_icon, loaded) = tokio::join!(git::info(&path), icon::find(&path), actions::load(&path));
        let targets = match &info {
            Some(_) => match git::git_dirs(&path).await {
                Some((git_dir, common)) => watch::targets(&git_dir, &common),
                None => Vec::new(),
            },
            None => Vec::new(),
        };

        let (project, changed) = {
            let mut state = self.state.lock();
            let project = state
                .projects
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ApiError::not_found(format!("project {id}")))?;
            let changed = !same_json(&project.git, &info)
                || project.icon != found_icon
                || project.actions != loaded.actions
                || project.action_errors != loaded.errors;
            project.git = info;
            project.icon = found_icon;
            project.actions = loaded.actions;
            project.action_errors = loaded.errors;
            (project.clone(), changed)
        };
        self.sync_watches(id, Kind::Git, targets);
        self.sync_watches(id, Kind::Actions, watch::action_targets(&path));

        if changed || emit == Emit::Always {
            self.emitter.emit(events::PROJECT_UPDATED, &project);
        }
        Ok(project)
    }

    /// Re-read only `.coocoo/actions.toml` (it changed on disk) and emit when that changed.
    async fn reload_actions(&self, id: &str) -> ApiResult<()> {
        let lock = self.refresh_lock(id);
        let _guard = lock.lock().await;
        let path = PathBuf::from(self.get(id)?.path);
        let loaded = actions::load(&path).await;
        let project = {
            let mut state = self.state.lock();
            let project = state
                .projects
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ApiError::not_found(format!("project {id}")))?;
            let changed = project.actions != loaded.actions || project.action_errors != loaded.errors;
            project.actions = loaded.actions;
            project.action_errors = loaded.errors;
            changed.then(|| project.clone())
        };
        // `.coocoo` may have just appeared or gone.
        self.sync_watches(id, Kind::Actions, watch::action_targets(&path));
        if let Some(project) = project {
            self.emitter.emit(events::PROJECT_UPDATED, &project);
        }
        Ok(())
    }

    /// Make the project's watches of `kind` match `targets`.
    fn sync_watches(&self, id: &str, kind: Kind, targets: Vec<watch::Target>) {
        fn paths(state: &mut State, kind: Kind) -> &mut HashMap<String, Vec<PathBuf>> {
            match kind {
                Kind::Git => &mut state.watched,
                Kind::Actions => &mut state.watched_actions,
            }
        }
        let old = {
            let mut state = self.state.lock();
            let map = paths(&mut state, kind);
            match map.get(id) {
                Some(cur) if cur.iter().eq(targets.iter().map(|t| &t.path)) => return,
                Some(_) => map.remove(id).unwrap_or_default(),
                None if targets.is_empty() => return,
                None => Vec::new(),
            }
        };
        self.watches.remove(id, kind, &old);
        let watched = self.watches.add(id, kind, &targets);
        let mut state = self.state.lock();
        if state.projects.iter().any(|p| p.id == id) {
            paths(&mut state, kind).insert(id.to_owned(), targets.into_iter().map(|t| t.path).collect());
        } else {
            drop(state);
            // Removed while we were refreshing.
            self.watches.remove(id, kind, &watched);
        }
    }

    fn git_project(&self, id: &str) -> ApiResult<Project> {
        let project = self.get(id)?;
        if project.git.is_none() && !Path::new(&project.path).join(".git").exists() {
            return Err(ApiError::invalid(format!("project {} ({}) is not a git repository", project.name, project.path)));
        }
        Ok(project)
    }

    /// Schedule refreshes for every project whose repository contains `top`.
    fn touch(&self, top: &Path) {
        let top = top.to_string_lossy();
        let ids: Vec<String> = self
            .state
            .lock()
            .projects
            .iter()
            .filter(|p| p.path == top || p.git.as_ref().is_some_and(|g| g.worktrees.iter().any(|w| w.path == top)))
            .map(|p| p.id.clone())
            .collect();
        for id in ids {
            self.watches.schedule(&id, Kind::Git);
        }
    }

    async fn commit(&self, cwd: &Path, message: &str) -> ApiResult<coocoo_proto::CommitInfo> {
        if message.trim().is_empty() {
            return Err(ApiError::invalid("commit message must not be empty"));
        }
        let top = git::toplevel(cwd).await?;
        git::run_ok(&top, &["add", "-A"]).await?;
        let staged = git::run(&top, &["diff", "--cached", "--quiet"]).await?;
        if staged.ok {
            return Err(ApiError::conflict("nothing to commit"));
        }
        git::run_ok(&top, &["commit", "-q", "-m", message]).await?;
        let commit = git::log(&top, 1)
            .await?
            .into_iter()
            .next()
            .map(|e| e.commit)
            .ok_or_else(|| ApiError::internal("commit not found after git commit"))?;
        self.touch(&top);
        Ok(commit)
    }

    async fn merge(&self, project_id: &str, branch: &str) -> ApiResult<(bool, String)> {
        let project = self.git_project(project_id)?;
        if branch.is_empty() || branch.starts_with('-') {
            return Err(ApiError::invalid(format!("invalid branch `{branch}`")));
        }
        let path = PathBuf::from(&project.path);
        let out = git::run(&path, &["merge", "--no-ff", "--no-edit", branch]).await?;
        let result = if out.ok {
            (true, out.combined())
        } else {
            // Leave the checkout exactly as it was; harmless when no merge was started.
            let _ = git::run(&path, &["merge", "--abort"]).await;
            (false, out.combined())
        };
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok(result)
    }

    /// `git fetch --all` in the project's checkout. Never prompts: credentials come from the
    /// configured helpers/keys or the fetch fails with git's message.
    async fn fetch(&self, project_id: &str) -> ApiResult<(bool, String)> {
        const TIMEOUT: Duration = Duration::from_secs(60);
        let project = self.git_project(project_id)?;
        let path = PathBuf::from(&project.path);
        let mut cmd = git::command(&path);
        cmd.args(["fetch", "--all", "--no-auto-gc", "--quiet"]);
        // Without a terminal ssh would wait forever for a passphrase or host-key answer.
        let custom_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
            || std::env::var_os("GIT_SSH").is_some()
            || git::run(&path, &["config", "core.sshCommand"]).await?.ok;
        if !custom_ssh {
            cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        let result = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
            Ok(out) => {
                let out = out.map_err(git::spawn_error)?;
                let out = git::GitOutput { ok: out.status.success(), stdout: out.stdout, stderr: out.stderr };
                (out.ok, out.combined())
            }
            Err(_) => (false, format!("git fetch timed out after {} s", TIMEOUT.as_secs())),
        };
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok(result)
    }

    /// Fast-forward the worktree at `wt_path` to its upstream (as of the last fetch). Refused
    /// with `conflict` unless the branch has an upstream, no local commits and a clean tree.
    async fn fast_forward(&self, project_id: &str, wt_path: &str) -> ApiResult<(bool, String)> {
        let project = self.git_project(project_id)?;
        let path = PathBuf::from(&project.path);
        let wanted = expand(wt_path);
        let wanted = std::fs::canonicalize(&wanted).unwrap_or(wanted);
        let worktrees = git::worktrees(&path).await?;
        let wt = worktrees
            .iter()
            .find(|w| Path::new(&w.path) == wanted)
            .ok_or_else(|| ApiError::not_found(format!("worktree {wt_path}")))?;
        let wt_path = PathBuf::from(&wt.path);
        let status = git::parse_status(&git::run_ok(&wt_path, &["status", "--porcelain=v2", "--branch", "-z"]).await?.stdout);
        let Some(upstream) = status.upstream else {
            return Err(ApiError::conflict("the branch has no upstream"));
        };
        if status.ahead > 0 {
            return Err(ApiError::conflict(format!(
                "the branch has {} commit(s) not on {upstream}; a fast-forward is not possible",
                status.ahead
            )));
        }
        if status.changed > 0 {
            return Err(ApiError::conflict(format!("{} uncommitted file(s); commit or stash them first", status.changed)));
        }
        if status.behind == 0 {
            return Ok((true, format!("Already up to date with {upstream}")));
        }
        let out = git::run(&wt_path, &["merge", "--ff-only", "-q", "@{upstream}"]).await?;
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok((out.ok, out.combined()))
    }

    async fn worktree_create(
        &self,
        project_id: &str,
        branch: Option<String>,
        base: Option<String>,
    ) -> ApiResult<WorktreeRef> {
        let project = self.git_project(project_id)?;
        let path = PathBuf::from(&project.path);
        let branch = branch.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()).unwrap_or_else(|| format!("coocoo/{}", new_id()));
        if !git::run(&path, &["check-ref-format", "--branch", &branch]).await?.ok || branch.starts_with('-') {
            return Err(ApiError::invalid(format!("invalid branch name `{branch}`")));
        }
        let base = match base.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()) {
            Some(base) if base.starts_with('-') => return Err(ApiError::invalid(format!("invalid base `{base}`"))),
            Some(base) => base,
            None => current_base(&path).await?,
        };
        let dir = self
            .data_dir
            .join("worktrees")
            .join(format!("{}-{}", slug(&project.name), project.id))
            .join(slug(&branch));
        if dir.exists() {
            return Err(ApiError::conflict(format!("worktree directory {} already exists", dir.display())));
        }
        if let Some(parent) = dir.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let dir_str = dir.to_str().ok_or_else(|| ApiError::invalid("worktree path is not valid UTF-8"))?;
        git::run_ok(&path, &["worktree", "add", "-q", "-b", &branch, dir_str, &base]).await?;
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok(WorktreeRef { path: dir_str.to_owned(), branch, base })
    }

    async fn worktree_remove(&self, project_id: &str, wt_path: &str, delete_branch: bool) -> ApiResult<()> {
        let project = self.git_project(project_id)?;
        let path = PathBuf::from(&project.path);
        let wanted = expand(wt_path);
        let wanted = std::fs::canonicalize(&wanted).unwrap_or(wanted);
        let worktrees = git::worktrees(&path).await?;
        let wt = worktrees
            .iter()
            .find(|w| Path::new(&w.path) == wanted)
            .ok_or_else(|| ApiError::not_found(format!("worktree {wt_path}")))?;
        if wt.is_main {
            return Err(ApiError::invalid("cannot remove the main worktree"));
        }
        if wanted.exists() {
            git::run_ok(&path, &["worktree", "remove", "--force", &wt.path]).await?;
        } else {
            git::run_ok(&path, &["worktree", "prune"]).await?;
        }
        if delete_branch && let Some(branch) = &wt.branch {
            git::run_ok(&path, &["branch", "-D", branch]).await?;
        }
        // Drop the per-project worktrees directory once it is empty (fails harmlessly otherwise).
        if let Some(parent) = wanted.parent()
            && parent.starts_with(self.data_dir.join("worktrees"))
        {
            let _ = std::fs::remove_dir(parent);
        }
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok(())
    }
}

/// Current branch, or the HEAD sha when detached.
async fn current_base(path: &Path) -> ApiResult<String> {
    let out = git::run(path, &["symbolic-ref", "--short", "-q", "HEAD"]).await?;
    if out.ok && git::has_head(path).await? {
        return Ok(out.stdout_line());
    }
    let out = git::run(path, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]).await?;
    if !out.ok {
        return Err(ApiError::conflict("repository has no commits yet; commit something before creating a worktree"));
    }
    Ok(out.stdout_line())
}

/// Filesystem-safe name: `coocoo/abc` → `coocoo-abc`.
fn slug(s: &str) -> String {
    let slug: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    let slug = slug.trim_matches(|c| c == '-' || c == '.');
    if slug.is_empty() { "x".to_owned() } else { slug.to_owned() }
}

fn same_json<T: serde::Serialize>(a: &T, b: &T) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Runs debounced refreshes until the workspace is dropped.
async fn refresh_loop(ws: Weak<Workspace>, queue: Queue) {
    while let Some(due) = queue.next_due().await {
        let Some(ws) = ws.upgrade() else { return };
        for (id, kind) in due {
            let ws = Arc::clone(&ws);
            tokio::spawn(async move {
                let result = match kind {
                    Kind::Git => ws.refresh(&id, Emit::IfChanged).await.map(drop),
                    Kind::Actions => ws.reload_actions(&id).await,
                };
                if let Err(err) = result
                    && err.code != "not_found"
                {
                    tracing::debug!("project {id} refresh failed: {err}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs() {
        assert_eq!(slug("coocoo/abc"), "coocoo-abc");
        assert_eq!(slug("My Project!"), "My-Project");
        assert_eq!(slug("///"), "x");
    }
}
