//! Bringing a worktree's work out: `git.push`, `git.outgoing` and the GitHub pull request methods
//! (`git.prCreate`, `git.prStatus`, `git.prMerge`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coocoo_proto::{ApiError, ApiResult, CommitInfo, MergeMethod, PrState, PullRequestInfo, Worktree};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::fs::expand;
use crate::projects::Emit;
use crate::{Workspace, git};

/// Upper bound of `git.outgoing`.
const MAX_OUTGOING: u32 = 200;
/// Parallel `gh pr list` calls after a fetch.
const PR_PARALLEL: usize = 8;

/// Where `git push` sends the checked-out branch.
struct PushTarget {
    branch: String,
    remote: String,
    /// Destination ref (`refs/heads/…`).
    dst: String,
    /// The branch already has an upstream (otherwise the push sets it).
    upstream: bool,
}

/// Result of `git.push`.
pub(crate) struct Pushed {
    pub ok: bool,
    pub output: String,
    /// Refused as non-fast-forward.
    pub rejected: bool,
}

async fn config(cwd: &Path, key: &str) -> ApiResult<Option<String>> {
    let out = git::run(cwd, &["config", "--get", key]).await?;
    Ok(out.ok.then(|| out.stdout_line()).filter(|v| !v.is_empty()))
}

/// The checked-out branch; `conflict` when HEAD is detached.
async fn branch(cwd: &Path) -> ApiResult<String> {
    let out = git::run(cwd, &["symbolic-ref", "--short", "-q", "HEAD"]).await?;
    if !out.ok {
        return Err(ApiError::conflict("HEAD is detached; check out a branch to push it"));
    }
    Ok(out.stdout_line())
}

async fn push_target(cwd: &Path) -> ApiResult<PushTarget> {
    let branch = branch(cwd).await?;
    let (remote_key, merge_key) = (format!("branch.{branch}.remote"), format!("branch.{branch}.merge"));
    let (remote, merge) = tokio::join!(config(cwd, &remote_key), config(cwd, &merge_key));
    if let (Some(remote), Some(merge)) = (remote?, merge?)
        && remote != "."
    {
        return Ok(PushTarget { branch, remote, dst: merge, upstream: true });
    }
    let Some(remote) = git::remotes(cwd).await.default else {
        let list = git::run(cwd, &["remote"]).await?.stdout_line();
        return Err(ApiError::conflict(if list.is_empty() {
            "the repository has no remote to push to".to_owned()
        } else {
            "the repository has several remotes and none is called `origin`; set the branch's upstream first".to_owned()
        }));
    };
    let dst = format!("refs/heads/{branch}");
    Ok(PushTarget { branch, remote, dst, upstream: false })
}

impl Workspace {
    /// The worktree of `project_id` containing `path` (any directory inside it).
    async fn worktree_at(&self, project_id: &str, path: &str) -> ApiResult<(PathBuf, Worktree, Vec<Worktree>)> {
        let project = self.git_project(project_id)?;
        let top = git::toplevel(&expand(path)).await?;
        let top = std::fs::canonicalize(&top).unwrap_or(top);
        let worktrees = git::worktrees(Path::new(&project.path)).await?;
        let wt = worktrees
            .iter()
            .find(|w| {
                let p = Path::new(&w.path);
                std::fs::canonicalize(p).map_or_else(|_| p == top, |c| c == top)
            })
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("worktree {path} in project {}", project.name)))?;
        Ok((PathBuf::from(&wt.path), wt, worktrees))
    }

    /// Push the checked-out branch: to its upstream, or with `-u` to `<remote>/<branch>` when it
    /// has none. Never forces; a rejected push is reported, not retried.
    pub(crate) async fn push(&self, project_id: &str, path: &str) -> ApiResult<Pushed> {
        let (wt_path, wt, _) = self.worktree_at(project_id, path).await?;
        let pushed = push_in(&wt_path).await?;
        if pushed.ok && !wt.is_main {
            self.refresh_pr_quietly(project_id, &wt_path).await;
        }
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok(pushed)
    }

    /// Commits `git.push` would send, newest first.
    pub(crate) async fn outgoing(&self, project_id: &str, path: &str) -> ApiResult<Vec<CommitInfo>> {
        let (wt_path, _, _) = self.worktree_at(project_id, path).await?;
        if !git::has_head(&wt_path).await? {
            return Ok(Vec::new());
        }
        let target = push_target(&wt_path).await?;
        let upstream_exists = target.upstream && git::run(&wt_path, &["rev-parse", "--verify", "-q", "@{upstream}"]).await?.ok;
        if upstream_exists {
            return git::log_revs(&wt_path, &["@{upstream}..HEAD"], MAX_OUTGOING).await;
        }
        // A new branch sends what no branch of the remote has yet.
        let remotes = format!("--remotes={}", target.remote);
        git::log_revs(&wt_path, &["HEAD", "--not", &remotes], MAX_OUTGOING).await
    }

    /// `owner/name` of the project's GitHub remote; `conflict` when it is not on GitHub.
    async fn github_repo(&self, wt_path: &Path) -> ApiResult<String> {
        let remotes = git::remotes(wt_path).await;
        match (remotes.github, remotes.default) {
            (Some(repo), _) => Ok(repo),
            (None, Some(remote)) => Err(ApiError::conflict(format!("{remote} is not on GitHub"))),
            (None, None) => Err(ApiError::conflict("the repository has no remote")),
        }
    }

    /// Check `gh` and the repository settings, then look up the branch's pull request.
    async fn lookup_pr(&self, wt_path: &Path) -> ApiResult<Option<PullRequestInfo>> {
        let repo = self.github_repo(wt_path).await?;
        let branch = branch(wt_path).await?;
        self.github.ensure_ready(wt_path).await?;
        self.github.ensure_settings(wt_path, &repo).await?;
        self.github.refresh_pr(wt_path, &repo, &branch).await
    }

    /// Refresh the PR status after a push; failures only matter to `git.prStatus`.
    async fn refresh_pr_quietly(&self, project_id: &str, wt_path: &Path) {
        if git::remotes(wt_path).await.github.is_none() {
            return;
        }
        if let Err(err) = self.lookup_pr(wt_path).await {
            tracing::debug!("project {project_id}: pull request status of {}: {err}", wt_path.display());
        }
    }

    /// `git.prStatus`: look the branch's pull request up now. Emits `project.updated` with the
    /// result (and the `gh` state learned on the way), also when it fails.
    pub(crate) async fn pr_status(&self, project_id: &str, path: &str) -> ApiResult<Option<PullRequestInfo>> {
        let (wt_path, _, _) = self.worktree_at(project_id, path).await?;
        let result = self.lookup_pr(&wt_path).await;
        self.refresh(project_id, Emit::IfChanged).await?;
        result
    }

    /// After a fetch: refresh the pull requests of all linked worktrees with an upstream.
    pub(crate) async fn refresh_prs_after_fetch(&self, project_path: &Path) {
        let remotes = git::remotes(project_path).await;
        let Some(repo) = remotes.github else { return };
        if self.github.ensure_ready(project_path).await.is_err() || self.github.ensure_settings(project_path, &repo).await.is_err() {
            return;
        }
        let Ok(worktrees) = git::worktrees(project_path).await else { return };
        let limit = Arc::new(Semaphore::new(PR_PARALLEL));
        let mut tasks = JoinSet::new();
        for wt in worktrees.into_iter().filter(|w| !w.is_main) {
            let (Some(branch), path) = (wt.branch, PathBuf::from(&wt.path)) else { continue };
            if !path.is_dir() || config(&path, &format!("branch.{branch}.merge")).await.ok().flatten().is_none() {
                continue;
            }
            let (cache, repo, limit) = (self.github.clone(), repo.clone(), limit.clone());
            tasks.spawn(async move {
                let _permit = limit.acquire().await;
                if let Err(err) = cache.refresh_pr(&path, &repo, &branch).await {
                    tracing::debug!("pull request status of {branch}: {err}");
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }

    /// `git.prCreate`: push when needed, then open a pull request for the worktree's branch.
    pub(crate) async fn pr_create(
        &self,
        project_id: &str,
        path: &str,
        title: &str,
        body: &str,
        draft: bool,
        base: Option<String>,
    ) -> ApiResult<PullRequestInfo> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ApiError::invalid("the pull request needs a title"));
        }
        let (wt_path, _, worktrees) = self.worktree_at(project_id, path).await?;
        let head = branch(&wt_path).await?;
        let base = match base.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()) {
            Some(base) => base,
            None => worktrees
                .iter()
                .find(|w| w.is_main)
                .and_then(|w| w.branch.clone())
                .ok_or_else(|| ApiError::conflict("the project checkout has no branch to use as base"))?,
        };
        if base == head || base.starts_with('-') {
            return Err(ApiError::invalid(format!("cannot open a pull request from {head} into {base}")));
        }
        let repo = self.github_repo(&wt_path).await?;
        self.github.ensure_ready(&wt_path).await?;

        let status = git::parse_status(&git::run_ok(&wt_path, &["status", "--porcelain=v2", "--branch", "-z"]).await?.stdout);
        if status.upstream.is_none() || status.ahead > 0 {
            let pushed = push_in(&wt_path).await?;
            if !pushed.ok {
                self.refresh(project_id, Emit::IfChanged).await?;
                return Err(ApiError::conflict(format!("push failed: {}", pushed.output)));
            }
        }
        let mut args = vec!["pr", "create", "--repo", &repo, "--base", &base, "--head", &head, "--title", title, "--body-file", "-"];
        if draft {
            args.push("--draft");
        }
        let created = self.github.gh(&wt_path, &args, Some(body)).await;
        let found = match created {
            Ok(_) => self.lookup_pr(&wt_path).await,
            Err(err) => Err(err),
        };
        self.refresh(project_id, Emit::IfChanged).await?;
        found?.ok_or_else(|| ApiError::internal("the pull request was created but gh does not list it yet"))
    }

    /// `git.prMerge`: merge the branch's open pull request on GitHub (never with `--admin` or
    /// `--auto`, and without `-d`, which would switch branches in the worktree), optionally delete
    /// the remote branch, then fetch.
    pub(crate) async fn pr_merge(&self, project_id: &str, path: &str, method: MergeMethod, delete_branch: bool) -> ApiResult<(bool, String)> {
        let (wt_path, _, _) = self.worktree_at(project_id, path).await?;
        let pr = self.lookup_pr(&wt_path).await?.filter(|p| p.state == PrState::Open);
        let Some(pr) = pr else {
            self.refresh(project_id, Emit::IfChanged).await?;
            return Err(ApiError::conflict("the branch has no open pull request"));
        };
        let repo = self.github_repo(&wt_path).await?;
        let number = pr.number.to_string();
        let merged = self.github.gh(&wt_path, &["pr", "merge", &number, "--repo", &repo, method.flag()], None).await;
        let (ok, mut output) = match merged {
            Ok(out) => (true, out.combined()),
            Err(err) => (false, err.message),
        };
        if ok && delete_branch {
            let target = push_target(&wt_path).await?;
            let remote_branch = target.dst.strip_prefix("refs/heads/").unwrap_or(&target.dst).to_owned();
            let deleted = git::run_network(&wt_path, &["push", &target.remote, "--delete", &remote_branch]).await?;
            let text = deleted.combined();
            if !text.is_empty() {
                output = if output.is_empty() { text } else { format!("{output}\n{text}") };
            }
        }
        // Fetch updates the base branch's remote ref and the PR statuses of all worktrees.
        let (_, fetched) = self.fetch(project_id).await?;
        if !fetched.is_empty() {
            tracing::debug!("fetch after merge: {fetched}");
        }
        self.refresh_pr_quietly(project_id, &wt_path).await;
        self.refresh(project_id, Emit::IfChanged).await?;
        Ok((ok, output))
    }
}

/// `git push` of the branch checked out at `wt_path`.
async fn push_in(wt_path: &Path) -> ApiResult<Pushed> {
    let target = push_target(wt_path).await?;
    let refspec = format!("HEAD:{}", target.dst);
    let args: Vec<&str> =
        if target.upstream { vec!["push", &target.remote, &refspec] } else { vec!["push", "-u", &target.remote, &refspec] };
    let out = git::run_network(wt_path, &args).await?;
    let output = out.combined();
    let rejected = !out.ok && (output.contains("non-fast-forward") || output.contains("fetch first") || output.contains("[rejected]"));
    if out.ok && !target.upstream {
        // `-u` with `HEAD:` as source does not configure the upstream on every git version.
        let upstream = format!("{}/{}", target.remote, target.branch);
        let _ = git::run(wt_path, &["branch", "--set-upstream-to", &upstream, &target.branch]).await;
    }
    Ok(Pushed { ok: out.ok, output, rejected })
}
