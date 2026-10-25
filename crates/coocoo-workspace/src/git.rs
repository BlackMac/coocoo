//! Async `git` CLI runner and output parsers.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use coocoo_proto::{ApiError, ApiResult, CommitInfo, GitInfo, GitOperation, GithubInfo, Worktree};
use tokio::process::Command;
use tokio::task::JoinSet;

/// The well-known empty tree object: diff base for repositories without commits.
pub(crate) const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// `git log` format: full sha, short sha, author, commit time, subject (unit-separated).
const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%an%x1f%ct%x1f%s";

pub(crate) struct GitOutput {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl GitOutput {
    /// Human-readable failure text: stderr, or stdout when git reported on stdout only.
    pub fn error_text(&self) -> String {
        let err = String::from_utf8_lossy(&self.stderr);
        let err = err.trim();
        if err.is_empty() { String::from_utf8_lossy(&self.stdout).trim().to_owned() } else { err.to_owned() }
    }

    /// stdout followed by stderr, trimmed.
    pub fn combined(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.stdout).into_owned();
        let err = String::from_utf8_lossy(&self.stderr);
        if !err.trim().is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&err);
        }
        text.trim().to_owned()
    }

    pub fn stdout_line(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
}

/// A `git -C <cwd>` command with a non-interactive, locale-stable, lock-free environment.
pub(crate) fn command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        // Never take optional locks (status would otherwise rewrite the index and re-trigger
        // our own watcher).
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    cmd
}

pub(crate) fn spawn_error(err: std::io::Error) -> ApiError {
    if err.kind() == std::io::ErrorKind::NotFound {
        ApiError::unavailable("git is not installed on this host")
    } else {
        ApiError::internal(format!("failed to run git: {err}"))
    }
}

/// Run git and capture its output; a non-zero exit is not an error here.
pub(crate) async fn run(cwd: &Path, args: &[&str]) -> ApiResult<GitOutput> {
    let out = command(cwd).args(args).output().await.map_err(spawn_error)?;
    Ok(GitOutput { ok: out.status.success(), stdout: out.stdout, stderr: out.stderr })
}

/// Run git; a non-zero exit becomes a `conflict` error carrying git's message.
pub(crate) async fn run_ok(cwd: &Path, args: &[&str]) -> ApiResult<GitOutput> {
    let out = run(cwd, args).await?;
    if out.ok {
        Ok(out)
    } else {
        Err(ApiError::conflict(format!("git {} failed: {}", args.first().copied().unwrap_or(""), out.error_text())))
    }
}

/// Network operations (fetch, push) give up after this.
const NETWORK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A clone may transfer a whole repository; it gives up after this.
pub(crate) const CLONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Run a git command that talks to remotes. Never prompts: credentials come from the configured
/// helpers/keys or the command fails with git's message; a timeout becomes a failed output.
pub(crate) async fn run_network(cwd: &Path, args: &[&str]) -> ApiResult<GitOutput> {
    run_network_within(cwd, args, NETWORK_TIMEOUT).await
}

/// [`run_network`] with its own time limit.
pub(crate) async fn run_network_within(cwd: &Path, args: &[&str], limit: std::time::Duration) -> ApiResult<GitOutput> {
    let mut cmd = command(cwd);
    cmd.args(args);
    // Without a terminal ssh would wait forever for a passphrase or host-key answer.
    let custom_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
        || std::env::var_os("GIT_SSH").is_some()
        || run(cwd, &["config", "core.sshCommand"]).await?.ok;
    if !custom_ssh {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    match tokio::time::timeout(limit, cmd.output()).await {
        Ok(out) => {
            let out = out.map_err(spawn_error)?;
            Ok(GitOutput { ok: out.status.success(), stdout: out.stdout, stderr: out.stderr })
        }
        Err(_) => Ok(GitOutput {
            ok: false,
            stdout: Vec::new(),
            stderr: format!("git {} timed out after {} s", args.first().copied().unwrap_or(""), limit.as_secs()).into_bytes(),
        }),
    }
}

/// Resolve the worktree root containing `cwd`: `not_found` when missing, `invalid_params`
/// when not inside a git working tree.
pub(crate) async fn toplevel(cwd: &Path) -> ApiResult<PathBuf> {
    match tokio::fs::metadata(cwd).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(ApiError::invalid(format!("{} is not a directory", cwd.display()))),
        Err(_) => return Err(ApiError::not_found(format!("directory {}", cwd.display()))),
    }
    let out = run(cwd, &["rev-parse", "--show-toplevel"]).await?;
    let top = out.stdout_line();
    if !out.ok || top.is_empty() {
        return Err(ApiError::invalid(format!("{} is not a git repository", cwd.display())));
    }
    Ok(PathBuf::from(top))
}

/// Whether HEAD resolves to a commit (false in a freshly initialized repository).
pub(crate) async fn has_head(cwd: &Path) -> ApiResult<bool> {
    Ok(run(cwd, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]).await?.ok)
}

// ─── status ──────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    /// The upstream is configured but its remote-tracking branch is gone (deleted on the remote
    /// and pruned by a fetch): `ahead`/`behind` are 0 then and mean nothing.
    pub upstream_gone: bool,
    pub ahead: u32,
    pub behind: u32,
    pub changed: u32,
    pub conflicts: u32,
}

/// Parse `git status --porcelain=v2 [--branch] -z`.
pub(crate) fn parse_status(raw: &[u8]) -> Status {
    let mut status = Status::default();
    let mut counted = false;
    let mut records = raw.split(|b| *b == 0);
    while let Some(rec) = records.next() {
        match rec.first() {
            Some(b'#') => {
                let line = String::from_utf8_lossy(rec);
                let Some(header) = line.strip_prefix("# ") else { continue };
                let (key, value) = header.split_once(' ').unwrap_or((header, ""));
                match key {
                    "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                    "branch.upstream" => status.upstream = Some(value.to_owned()),
                    "branch.ab" => {
                        counted = true;
                        for part in value.split(' ') {
                            if let Some(n) = part.strip_prefix('+') {
                                status.ahead = n.parse().unwrap_or(0);
                            } else if let Some(n) = part.strip_prefix('-') {
                                status.behind = n.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(b'u') => {
                status.changed += 1;
                status.conflicts += 1;
            }
            Some(b'1' | b'?') => status.changed += 1,
            Some(b'2') => {
                status.changed += 1;
                // Renamed/copied entries carry the original path as an extra record.
                records.next();
            }
            _ => {}
        }
    }
    // git prints `branch.ab` only when the upstream's commit exists.
    status.upstream_gone = status.upstream.is_some() && !counted;
    status
}

// ─── log ─────────────────────────────────────────────────────────────────────────────────────

/// One parsed `git log` entry: commit plus its abbreviated sha.
pub(crate) struct LogEntry {
    pub commit: CommitInfo,
    pub short: String,
}

/// Parse `git log -z` output produced with [`LOG_FORMAT`].
pub(crate) fn parse_log(raw: &[u8]) -> Vec<LogEntry> {
    raw.split(|b| *b == 0)
        .filter(|rec| !rec.is_empty())
        .filter_map(|rec| {
            let rec = String::from_utf8_lossy(rec);
            let mut fields = rec.trim_start_matches('\n').splitn(5, '\x1f');
            let sha = fields.next()?.to_owned();
            let short = fields.next()?.to_owned();
            let author = fields.next()?.to_owned();
            let ts = fields.next()?.parse::<i64>().unwrap_or(0) * 1000;
            let summary = fields.next().unwrap_or("").to_owned();
            Some(LogEntry { commit: CommitInfo { sha, summary, author, ts }, short })
        })
        .collect()
}

/// `git log` from HEAD; empty for repositories without commits.
pub(crate) async fn log(cwd: &Path, limit: u32) -> ApiResult<Vec<LogEntry>> {
    if !has_head(cwd).await? {
        return Ok(Vec::new());
    }
    let limit = format!("-n{limit}");
    let out = run_ok(cwd, &["log", &limit, "-z", LOG_FORMAT, "HEAD", "--"]).await?;
    Ok(parse_log(&out.stdout))
}

/// `git log` of the revisions `revs` (e.g. `["@{upstream}..HEAD"]`), newest first.
pub(crate) async fn log_revs(cwd: &Path, revs: &[&str], limit: u32) -> ApiResult<Vec<CommitInfo>> {
    let limit = format!("-n{limit}");
    let mut args = vec!["log", &limit, "-z", LOG_FORMAT];
    args.extend_from_slice(revs);
    args.push("--");
    let out = run_ok(cwd, &args).await?;
    Ok(parse_log(&out.stdout).into_iter().map(|e| e.commit).collect())
}

// ─── worktrees ───────────────────────────────────────────────────────────────────────────────

/// Parse `git worktree list --porcelain` (newline format, compatible with old git).
pub(crate) fn parse_worktrees(raw: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in raw.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(Worktree { path: path.to_owned(), is_main: out.is_empty(), ..Worktree::default() });
            continue;
        }
        let Some(wt) = out.last_mut() else { continue };
        if let Some(head) = line.strip_prefix("HEAD ") {
            wt.head = Some(head.to_owned());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            wt.branch = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).to_owned());
        }
    }
    out
}

pub(crate) async fn worktrees(cwd: &Path) -> ApiResult<Vec<Worktree>> {
    let out = run_ok(cwd, &["worktree", "list", "--porcelain"]).await?;
    Ok(parse_worktrees(&String::from_utf8_lossy(&out.stdout)))
}

/// Per-worktree state that `git worktree list` does not report.
struct Details {
    status: Option<Status>,
    /// `(ahead, behind)` against the base branch.
    base: Option<(u32, u32)>,
    /// Commits ahead of the base, but all their changes are in it already.
    changes_in_base: bool,
    operation: Option<GitOperation>,
    fetched_at: Option<i64>,
}

/// Status (unless already `known`), counts against `base` and the operation in progress of the
/// worktree at `path`.
async fn worktree_details(path: PathBuf, known: Option<Status>, base: Option<String>) -> Details {
    let status = async {
        match known {
            Some(status) => Some(status),
            None => run(&path, &["status", "--porcelain=v2", "--branch", "-z"])
                .await
                .ok()
                .filter(|o| o.ok)
                .map(|o| parse_status(&o.stdout)),
        }
    };
    let counts = async {
        let base = base?;
        let range = format!("refs/heads/{base}...HEAD");
        let out = run(&path, &["rev-list", "--left-right", "--count", &range, "--"]).await.ok().filter(|o| o.ok)?;
        let counts = parse_left_right(&out.stdout_line())?;
        let in_base = counts.0 > 0 && changes_in(&path, &base).await;
        Some((counts, in_base))
    };
    let (status, base) = tokio::join!(status, counts);
    let git_dir = worktree_git_dir(&path).await;
    Details {
        status,
        base: base.map(|(counts, _)| counts),
        changes_in_base: base.is_some_and(|(_, in_base)| in_base),
        operation: git_dir.as_deref().and_then(operation),
        fetched_at: git_dir.as_deref().and_then(fetched_at),
    }
}

/// Whether everything HEAD changed is in branch `base` already, e.g. after a squash or rebase
/// merge: HEAD has the same tree, or merging HEAD into `base` would leave its tree unchanged.
/// `false` when unknown (git < 2.38 has no `merge-tree --write-tree`, conflicts).
async fn changes_in(path: &Path, base: &str) -> bool {
    let base_tree = format!("refs/heads/{base}^{{tree}}");
    let Some(trees) = run(path, &["rev-parse", "HEAD^{tree}", &base_tree]).await.ok().filter(|o| o.ok) else {
        return false;
    };
    let trees = String::from_utf8_lossy(&trees.stdout).into_owned();
    let mut trees = trees.lines();
    let (Some(head), Some(base_tree)) = (trees.next(), trees.next()) else { return false };
    if head == base_tree {
        return true;
    }
    let base_ref = format!("refs/heads/{base}");
    run(path, &["merge-tree", "--write-tree", "--no-messages", &base_ref, "HEAD"])
        .await
        .ok()
        .filter(|o| o.ok)
        .is_some_and(|o| o.stdout_line() == base_tree)
}

/// Parse `rev-list --left-right --count base...HEAD` (`<behind>\t<ahead>`) into `(ahead, behind)`.
fn parse_left_right(line: &str) -> Option<(u32, u32)> {
    let (behind, ahead) = line.split_once('\t')?;
    Some((ahead.trim().parse().ok()?, behind.trim().parse().ok()?))
}

/// The git directory of the worktree at `path`: `.git` itself, or the `gitdir:` a linked
/// worktree's `.git` file points to.
async fn worktree_git_dir(path: &Path) -> Option<PathBuf> {
    let dot_git = path.join(".git");
    if tokio::fs::metadata(&dot_git).await.ok()?.is_dir() {
        return Some(dot_git);
    }
    let text = tokio::fs::read_to_string(&dot_git).await.ok()?;
    let dir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    Some(if dir.is_absolute() { dir } else { path.join(dir) })
}

/// The operation in progress in a worktree, from the state files git keeps in its git dir.
pub(crate) fn operation(git_dir: &Path) -> Option<GitOperation> {
    let has = |name: &str| git_dir.join(name).exists();
    if has("rebase-merge") || has("rebase-apply") {
        Some(GitOperation::Rebase)
    } else if has("MERGE_HEAD") {
        Some(GitOperation::Merge)
    } else if has("CHERRY_PICK_HEAD") {
        Some(GitOperation::CherryPick)
    } else if has("REVERT_HEAD") {
        Some(GitOperation::Revert)
    } else if has("BISECT_LOG") {
        Some(GitOperation::Bisect)
    } else {
        None
    }
}

/// When `git fetch` last ran in a worktree (`FETCH_HEAD` lives in each worktree's git dir).
fn fetched_at(git_dir: &Path) -> Option<i64> {
    let modified = std::fs::metadata(git_dir.join("FETCH_HEAD")).ok()?.modified().ok()?;
    i64::try_from(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis()).ok()
}

// ─── remotes ─────────────────────────────────────────────────────────────────────────────────

/// The repository's default remote and, when it is on github.com, its `owner/name`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Remotes {
    /// `origin`, else the only remote.
    pub default: Option<String>,
    pub github: Option<String>,
}

/// Parse `git config --get-regexp ^remote\..*\.url$` (`remote.<name>.url <url>` lines).
pub(crate) fn parse_remotes(raw: &str) -> Remotes {
    let urls: Vec<(&str, &str)> = raw
        .lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            Some((key.strip_prefix("remote.")?.strip_suffix(".url")?, url.trim()))
        })
        .collect();
    let chosen = urls.iter().find(|(name, _)| *name == "origin").or(if urls.len() == 1 { urls.first() } else { None });
    match chosen {
        Some((name, url)) => Remotes { default: Some((*name).to_owned()), github: crate::github::github_repo(url) },
        None => Remotes::default(),
    }
}

pub(crate) async fn remotes(path: &Path) -> Remotes {
    match run(path, &["config", "--get-regexp", r"^remote\..*\.url$"]).await {
        Ok(out) if out.ok => parse_remotes(&String::from_utf8_lossy(&out.stdout)),
        _ => Remotes::default(),
    }
}

// ─── project summary ─────────────────────────────────────────────────────────────────────────

/// Compute the project's git summary; `None` when `path` is not inside a git working tree.
pub(crate) async fn info(path: &Path) -> Option<GitInfo> {
    let (status, last, list, remotes) = tokio::join!(
        run(path, &["status", "--porcelain=v2", "--branch", "-z"]),
        run(path, &["log", "-1", "-z", LOG_FORMAT, "HEAD", "--"]),
        run(path, &["worktree", "list", "--porcelain"]),
        remotes(path),
    );
    let status = status.ok().filter(|o| o.ok)?;
    let status = parse_status(&status.stdout);
    let last = last.ok().filter(|o| o.ok).and_then(|o| parse_log(&o.stdout).into_iter().next());
    let abbrev = last.as_ref().map_or(7, |l| l.short.len());
    let mut worktrees = list
        .ok()
        .filter(|o| o.ok)
        .map(|o| parse_worktrees(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();

    let here = path.to_string_lossy();
    // Linked worktrees are compared with the main worktree's branch, the target of `git.merge`.
    let main_branch = worktrees.iter().find(|w| w.is_main).and_then(|w| w.branch.clone());
    let mut details = JoinSet::new();
    for (i, wt) in worktrees.iter_mut().enumerate() {
        if let Some(head) = &mut wt.head {
            head.truncate(abbrev);
        }
        let wt_path = PathBuf::from(&wt.path);
        if !wt_path.is_dir() {
            continue;
        }
        let known = (wt.path == here).then(|| status.clone());
        let base = match (&main_branch, &wt.branch) {
            (Some(base), Some(own)) if !wt.is_main && base != own => Some(base.clone()),
            _ => None,
        };
        details.spawn(async move { (i, worktree_details(wt_path, known, base).await) });
    }
    let mut fetched = None;
    while let Some(Ok((i, d))) = details.join_next().await {
        let wt = &mut worktrees[i];
        if let Some(s) = d.status {
            wt.changed = Some(s.changed);
            wt.upstream = s.upstream;
            wt.upstream_gone = s.upstream_gone;
            wt.ahead = s.ahead;
            wt.behind = s.behind;
            wt.conflicts = s.conflicts;
        }
        if let Some((ahead, behind)) = d.base {
            wt.base_ahead = Some(ahead);
            wt.base_behind = Some(behind);
            wt.changes_in_base = d.changes_in_base;
        }
        wt.operation = d.operation;
        fetched = fetched.max(d.fetched_at);
    }

    let github = match (&remotes.default, remotes.github) {
        (Some(remote), Some(repo)) => Some(GithubInfo { repo, remote: remote.clone(), ..GithubInfo::default() }),
        _ => None,
    };
    Some(GitInfo {
        branch: status.branch,
        head: last.as_ref().map(|l| l.short.clone()),
        upstream: status.upstream,
        ahead: status.ahead,
        behind: status.behind,
        changed: status.changed,
        last_commit: last.map(|l| l.commit),
        worktrees,
        fetched_at: fetched,
        remote: remotes.default,
        github,
    })
}

/// Absolute `(git_dir, common_dir)` of the repository containing `path`.
pub(crate) async fn git_dirs(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let out = run(path, &["rev-parse", "--absolute-git-dir", "--git-common-dir"]).await.ok().filter(|o| o.ok)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let git_dir = PathBuf::from(lines.next()?);
    let common = PathBuf::from(lines.next()?);
    let common = if common.is_absolute() { common } else { path.join(common) };
    let common = std::fs::canonicalize(&common).unwrap_or(common);
    Some((git_dir, common))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2() {
        let raw = b"# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\0\
1 .M N... 100644 100644 100644 a a f1\0\
2 R. N... 100644 100644 100644 a a R100 new\0old\0\
u UU N... 100644 100644 100644 100644 a b c both\0\
? untracked\0! ignored\0";
        let s = parse_status(raw);
        assert_eq!(
            s,
            Status {
                branch: Some("main".into()),
                upstream: Some("origin/main".into()),
                upstream_gone: false,
                ahead: 2,
                behind: 3,
                changed: 4,
                conflicts: 1,
            }
        );
        let detached = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!(detached.branch, None);
        assert_eq!(detached.changed, 0);
        let gone = parse_status(b"# branch.oid abc\0# branch.head x\0# branch.upstream origin/x\0");
        assert_eq!((gone.upstream.as_deref(), gone.upstream_gone, gone.ahead), (Some("origin/x"), true, 0));
        assert!(!parse_status(b"# branch.head x\0").upstream_gone, "no upstream is not a gone one");
    }

    #[test]
    fn worktree_list() {
        let raw = "worktree /a\nHEAD 1234567890\nbranch refs/heads/main\n\nworktree /b\nHEAD abcdef\ndetached\n\n";
        let wts = parse_worktrees(raw);
        assert_eq!(wts.len(), 2);
        assert!(wts[0].is_main);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert!(!wts[1].is_main);
        assert_eq!(wts[1].branch, None);
        assert_eq!(wts[1].head.as_deref(), Some("abcdef"));
    }

    #[test]
    fn log_records() {
        let raw = b"aaaa\x1faa\x1fAlice\x1f10\x1fFirst: x\x1fy\0bbbb\x1fbb\x1fBob\x1f20\x1fSecond\0";
        let log = parse_log(raw);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].commit.summary, "First: x\x1fy");
        assert_eq!(log[0].commit.ts, 10_000);
        assert_eq!(log[1].short, "bb");
        assert_eq!(log[1].commit.author, "Bob");
    }

    #[test]
    fn remotes_pick_origin_or_the_only_one() {
        let both = "remote.upstream.url https://gitlab.com/x/y.git\nremote.origin.url git@github.com:BlackMac/coocoo.git\n";
        assert_eq!(parse_remotes(both), Remotes { default: Some("origin".into()), github: Some("BlackMac/coocoo".into()) });
        let single = "remote.my.fork.url https://github.com/me/repo\n";
        assert_eq!(parse_remotes(single), Remotes { default: Some("my.fork".into()), github: Some("me/repo".into()) });
        let other = "remote.origin.url /srv/git/repo.git\n";
        assert_eq!(parse_remotes(other), Remotes { default: Some("origin".into()), github: None });
        let ambiguous = "remote.a.url https://github.com/o/a\nremote.b.url https://github.com/o/b\n";
        assert_eq!(parse_remotes(ambiguous), Remotes::default());
        assert_eq!(parse_remotes(""), Remotes::default());
    }
}
