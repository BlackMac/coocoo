//! GitHub through the `gh` CLI: remote URL parsing, `gh` availability, repository settings and
//! the pull request cache that `git::info` results are decorated with.
//!
//! Everything here that touches the network runs only on demand (a PR action, `git.prStatus`,
//! a fetch): never on watcher or turn refreshes.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use coocoo_proto::{ApiError, ApiResult, GhState, GitInfo, MergeMethod, PrCheck, PrChecks, PrReview, PrState, PullRequestInfo};
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::git::GitOutput;

const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// `gh pr list --json` fields parsed by [`parse_pr_list`].
pub(crate) const PR_FIELDS: &str =
    "number,url,title,state,isDraft,baseRefName,reviewDecision,mergeStateStatus,statusCheckRollup,isCrossRepository";

/// `owner/name` of a github.com remote URL (`https://github.com/o/r.git`, `git@github.com:o/r`,
/// `ssh://git@github.com/o/r`); `None` for other hosts.
pub(crate) fn github_repo(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        // scp-like `user@host:path`.
        None => url.split_once(':').filter(|(host, _)| !host.contains('/'))?,
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (valid(owner) && valid(name)).then(|| format!("{owner}/{name}"))
}

// ─── gh runner ───────────────────────────────────────────────────────────────────────────────

/// Run `gh` non-interactively in `cwd` with optional stdin; a non-zero exit is not an error here.
pub(crate) async fn run(cwd: &Path, args: &[&str], stdin: Option<&str>) -> ApiResult<GitOutput> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args)
        .current_dir(cwd)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ApiError::unavailable("gh is not installed on this host")
        } else {
            ApiError::internal(format!("failed to run gh: {err}"))
        }
    })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).await?;
        drop(pipe);
    }
    match tokio::time::timeout(GH_TIMEOUT, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            Ok(GitOutput { ok: out.status.success(), stdout: out.stdout, stderr: out.stderr })
        }
        Err(_) => Err(ApiError::unavailable(format!("gh {} timed out after {} s", args.first().copied().unwrap_or(""), GH_TIMEOUT.as_secs()))),
    }
}

/// Whether `gh` is installed and logged in to github.com.
async fn probe(cwd: &Path) -> GhState {
    match run(cwd, &["auth", "status", "--hostname", "github.com"], None).await {
        Ok(out) if out.ok => GhState::Ready,
        Ok(_) => GhState::LoggedOut,
        Err(err) if err.code == "unavailable" && err.message.contains("not installed") => GhState::Missing,
        Err(_) => GhState::LoggedOut,
    }
}

// ─── cache ───────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepoSettings {
    merge_methods: Vec<MergeMethod>,
    delete_branch_on_merge: bool,
}

struct PrEntry {
    pr: Option<PullRequestInfo>,
    checked_at: i64,
}

/// Per daemon: `gh` state, per repository: merge settings, per `(repo, branch)`: pull request.
#[derive(Default)]
pub(crate) struct Cache {
    gh: Mutex<GhState>,
    repos: Mutex<HashMap<String, RepoSettings>>,
    prs: Mutex<HashMap<(String, String), PrEntry>>,
}

impl Cache {
    /// Determine `gh`'s state unless it is known to work, then fail unless it does.
    pub(crate) async fn ensure_ready(&self, cwd: &Path) -> ApiResult<()> {
        let mut state = *self.gh.lock();
        if state != GhState::Ready {
            state = probe(cwd).await;
            *self.gh.lock() = state;
        }
        match state {
            GhState::Ready => Ok(()),
            GhState::Missing => Err(ApiError::unavailable("gh is not installed on this host")),
            _ => Err(ApiError::unavailable("gh is not logged in to github.com (run `gh auth login` on this host)")),
        }
    }

    /// The repository's merge settings, fetched once per repository.
    pub(crate) async fn ensure_settings(&self, cwd: &Path, repo: &str) -> ApiResult<()> {
        if self.repos.lock().contains_key(repo) {
            return Ok(());
        }
        let out = self
            .gh(cwd, &["repo", "view", repo, "--json", "squashMergeAllowed,mergeCommitAllowed,rebaseMergeAllowed,deleteBranchOnMerge"], None)
            .await?;
        let settings = parse_repo_settings(&out.stdout).map_err(|e| ApiError::internal(format!("gh repo view: {e}")))?;
        self.repos.lock().insert(repo.to_owned(), settings);
        Ok(())
    }

    /// Run `gh`; a failure becomes a `conflict` error with gh's message and makes the next call
    /// re-check `gh` and the repository settings.
    pub(crate) async fn gh(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> ApiResult<GitOutput> {
        let out = run(cwd, args, stdin).await;
        match out {
            Ok(out) if out.ok => Ok(out),
            Ok(out) => {
                self.forget();
                Err(ApiError::conflict(format!("gh {} failed: {}", args[..2.min(args.len())].join(" "), out.error_text())))
            }
            Err(err) => {
                self.forget();
                Err(err)
            }
        }
    }

    fn forget(&self) {
        *self.gh.lock() = GhState::Unknown;
        self.repos.lock().clear();
    }

    /// Look up `branch`'s pull request on GitHub and remember it.
    pub(crate) async fn refresh_pr(&self, cwd: &Path, repo: &str, branch: &str) -> ApiResult<Option<PullRequestInfo>> {
        let out = self
            .gh(cwd, &["pr", "list", "--repo", repo, "--head", branch, "--state", "all", "--limit", "10", "--json", PR_FIELDS], None)
            .await?;
        let now = coocoo_proto::util::now_ms();
        let pr = parse_pr_list(&out.stdout, now).map_err(|e| ApiError::internal(format!("gh pr list: {e}")))?;
        self.prs.lock().insert((repo.to_owned(), branch.to_owned()), PrEntry { pr: pr.clone(), checked_at: now });
        Ok(pr)
    }

    /// Merge the cached `gh` state, repository settings and pull requests into `info`.
    pub(crate) fn decorate(&self, info: &mut GitInfo) {
        let Some(github) = &mut info.github else { return };
        github.gh = *self.gh.lock();
        if let Some(settings) = self.repos.lock().get(&github.repo) {
            github.merge_methods = settings.merge_methods.clone();
            github.delete_branch_on_merge = settings.delete_branch_on_merge;
        }
        let prs = self.prs.lock();
        for wt in &mut info.worktrees {
            let Some(branch) = &wt.branch else { continue };
            if let Some(entry) = prs.get(&(github.repo.clone(), branch.clone())) {
                wt.pull_request = entry.pr.clone();
                wt.pr_checked_at = Some(entry.checked_at);
            }
        }
    }
}

// ─── parsers ─────────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepo {
    #[serde(default)]
    squash_merge_allowed: bool,
    #[serde(default)]
    merge_commit_allowed: bool,
    #[serde(default)]
    rebase_merge_allowed: bool,
    #[serde(default)]
    delete_branch_on_merge: bool,
}

fn parse_repo_settings(raw: &[u8]) -> Result<RepoSettings, String> {
    let r: RawRepo = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    let methods = [(r.squash_merge_allowed, MergeMethod::Squash), (r.merge_commit_allowed, MergeMethod::Merge), (r.rebase_merge_allowed, MergeMethod::Rebase)];
    Ok(RepoSettings {
        merge_methods: methods.into_iter().filter(|(allowed, _)| *allowed).map(|(_, m)| m).collect(),
        delete_branch_on_merge: r.delete_branch_on_merge,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    url: String,
    #[serde(default)]
    title: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    merge_state_status: Option<String>,
    #[serde(default)]
    status_check_rollup: Option<Vec<RawCheck>>,
    #[serde(default)]
    is_cross_repository: bool,
}

/// One entry of `statusCheckRollup`: a `CheckRun` (status/conclusion) or a `StatusContext` (state).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCheck {
    #[serde(default, rename = "__typename")]
    typename: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    details_url: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Pending,
}

fn outcome(check: &RawCheck) -> Outcome {
    let upper = |s: &Option<String>| s.as_deref().unwrap_or("").to_ascii_uppercase();
    if check.typename == "StatusContext" || (check.state.is_some() && check.status.is_none()) {
        return match upper(&check.state).as_str() {
            "SUCCESS" => Outcome::Passed,
            "FAILURE" | "ERROR" => Outcome::Failed,
            _ => Outcome::Pending,
        };
    }
    if upper(&check.status) != "COMPLETED" {
        return Outcome::Pending;
    }
    match upper(&check.conclusion).as_str() {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Outcome::Passed,
        _ => Outcome::Failed,
    }
}

fn checks(raw: &[RawCheck]) -> PrChecks {
    let mut out = PrChecks { total: u32::try_from(raw.len()).unwrap_or(u32::MAX), ..PrChecks::default() };
    for check in raw {
        match outcome(check) {
            Outcome::Passed => out.passed += 1,
            Outcome::Pending => out.pending += 1,
            Outcome::Failed => {
                out.failed += 1;
                let name = check.name.clone().or_else(|| check.context.clone()).unwrap_or_else(|| "check".to_owned());
                let url = check.details_url.clone().or_else(|| check.target_url.clone()).filter(|u| !u.is_empty());
                out.failing.push(PrCheck { name, url });
            }
        }
    }
    out
}

/// Parse `gh pr list --json <PR_FIELDS>`: the branch's open pull request, else the newest one.
/// Pull requests from forks that happen to use the same branch name are ignored.
pub(crate) fn parse_pr_list(raw: &[u8], now: i64) -> Result<Option<PullRequestInfo>, String> {
    let list: Vec<RawPr> = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    let mut own = list.into_iter().filter(|p| !p.is_cross_repository);
    let mut first = None;
    let mut chosen = None;
    for pr in own.by_ref() {
        if pr.state.eq_ignore_ascii_case("OPEN") {
            chosen = Some(pr);
            break;
        }
        if first.is_none() {
            first = Some(pr);
        }
    }
    let Some(pr) = chosen.or(first) else { return Ok(None) };
    let state = match pr.state.to_ascii_uppercase().as_str() {
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        _ => PrState::Closed,
    };
    let review = match pr.review_decision.as_deref().unwrap_or("") {
        "APPROVED" => Some(PrReview::Approved),
        "CHANGES_REQUESTED" => Some(PrReview::ChangesRequested),
        "REVIEW_REQUIRED" => Some(PrReview::ReviewRequired),
        _ => None,
    };
    Ok(Some(PullRequestInfo {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        state,
        draft: pr.is_draft,
        base: pr.base_ref_name,
        checks: checks(pr.status_check_rollup.as_deref().unwrap_or_default()),
        review,
        merge_state: pr.merge_state_status.unwrap_or_else(|| "UNKNOWN".to_owned()),
        fetched_at: now,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remote_urls() {
        for url in [
            "https://github.com/BlackMac/coocoo.git",
            "https://github.com/BlackMac/coocoo",
            "https://user@github.com/BlackMac/coocoo/",
            "git@github.com:BlackMac/coocoo.git",
            "ssh://git@github.com/BlackMac/coocoo.git",
            "ssh://git@github.com:22/BlackMac/coocoo",
            "git://GitHub.com/BlackMac/coocoo",
        ] {
            assert_eq!(github_repo(url).as_deref(), Some("BlackMac/coocoo"), "{url}");
        }
        for url in [
            "https://gitlab.com/o/r.git",
            "git@gitea.example.org:o/r.git",
            "/srv/git/repo.git",
            "../bare",
            "https://github.com/onlyowner",
            "https://github.com.evil.org/o/r",
            "file:///tmp/github.com/o/r",
        ] {
            assert_eq!(github_repo(url), None, "{url}");
        }
    }

    #[test]
    fn pr_list_mixed_checks() {
        let raw = br#"[{"number":42,"url":"https://github.com/o/r/pull/42","title":"Live CDP","state":"OPEN","isDraft":false,
            "baseRefName":"master","reviewDecision":"REVIEW_REQUIRED","mergeStateStatus":"BLOCKED","isCrossRepository":false,
            "statusCheckRollup":[
              {"__typename":"CheckRun","name":"test (ubuntu-latest)","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://ci/1"},
              {"__typename":"CheckRun","name":"clippy","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://ci/2"},
              {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED","detailsUrl":""},
              {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":"","detailsUrl":"https://ci/3"},
              {"__typename":"StatusContext","context":"ci/legacy","state":"ERROR","targetUrl":"https://legacy/4"},
              {"__typename":"StatusContext","context":"deploy","state":"PENDING","targetUrl":""},
              {"__typename":"StatusContext","context":"lint","state":"SUCCESS","targetUrl":""}
            ]}]"#;
        let pr = parse_pr_list(raw, 7).unwrap().unwrap();
        assert_eq!((pr.number, pr.state, pr.draft, pr.base.as_str()), (42, PrState::Open, false, "master"));
        assert_eq!(pr.review, Some(PrReview::ReviewRequired));
        assert_eq!(pr.merge_state, "BLOCKED");
        assert_eq!((pr.checks.total, pr.checks.passed, pr.checks.failed, pr.checks.pending), (7, 3, 2, 2));
        assert_eq!(
            pr.checks.failing,
            vec![
                PrCheck { name: "test (ubuntu-latest)".into(), url: Some("https://ci/1".into()) },
                PrCheck { name: "ci/legacy".into(), url: Some("https://legacy/4".into()) },
            ]
        );
        assert_eq!(pr.fetched_at, 7);
    }

    #[test]
    fn pr_list_prefers_open_and_skips_forks() {
        let raw = br#"[
            {"number":9,"url":"u9","title":"fork","state":"OPEN","isCrossRepository":true,"statusCheckRollup":[]},
            {"number":8,"url":"u8","title":"merged","state":"MERGED","isCrossRepository":false,"reviewDecision":"APPROVED","mergeStateStatus":"UNKNOWN","statusCheckRollup":[]},
            {"number":5,"url":"u5","title":"reopened","state":"OPEN","isDraft":true,"isCrossRepository":false,"reviewDecision":"","statusCheckRollup":null}
        ]"#;
        let pr = parse_pr_list(raw, 0).unwrap().unwrap();
        assert_eq!((pr.number, pr.state, pr.draft, pr.review), (5, PrState::Open, true, None));
        assert_eq!(pr.checks, PrChecks::default());
        assert_eq!(pr.merge_state, "UNKNOWN");

        let merged_only = br#"[{"number":8,"url":"u8","title":"m","state":"MERGED","statusCheckRollup":[]},{"number":3,"url":"u3","title":"c","state":"CLOSED"}]"#;
        assert_eq!(parse_pr_list(merged_only, 0).unwrap().map(|p| (p.number, p.state)), Some((8, PrState::Merged)));
        assert_eq!(parse_pr_list(b"[]", 0).unwrap(), None);
        assert!(parse_pr_list(b"not json", 0).is_err());
    }

    #[test]
    fn repo_settings() {
        let s = parse_repo_settings(br#"{"squashMergeAllowed":true,"mergeCommitAllowed":false,"rebaseMergeAllowed":true,"deleteBranchOnMerge":true}"#).unwrap();
        assert_eq!(s.merge_methods, vec![MergeMethod::Squash, MergeMethod::Rebase]);
        assert!(s.delete_branch_on_merge);
    }
}
