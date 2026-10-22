//! Async `git` CLI runner and output parsers.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use coocoo_proto::{ApiError, ApiResult, CommitInfo, GitInfo, Worktree};
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

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changed: u32,
}

/// Parse `git status --porcelain=v2 [--branch] -z`.
pub(crate) fn parse_status(raw: &[u8]) -> Status {
    let mut status = Status::default();
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
            Some(b'1' | b'u' | b'?') => status.changed += 1,
            Some(b'2') => {
                status.changed += 1;
                // Renamed/copied entries carry the original path as an extra record.
                records.next();
            }
            _ => {}
        }
    }
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

// ─── worktrees ───────────────────────────────────────────────────────────────────────────────

/// Parse `git worktree list --porcelain` (newline format, compatible with old git).
pub(crate) fn parse_worktrees(raw: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in raw.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(Worktree {
                path: path.to_owned(),
                branch: None,
                head: None,
                is_main: out.is_empty(),
                changed: None,
            });
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

async fn changed_count(path: PathBuf) -> Option<u32> {
    let out = run(&path, &["status", "--porcelain=v2", "-z"]).await.ok().filter(|o| o.ok)?;
    Some(parse_status(&out.stdout).changed)
}

// ─── project summary ─────────────────────────────────────────────────────────────────────────

/// Compute the project's git summary; `None` when `path` is not inside a git working tree.
pub(crate) async fn info(path: &Path) -> Option<GitInfo> {
    let (status, last, list) = tokio::join!(
        run(path, &["status", "--porcelain=v2", "--branch", "-z"]),
        run(path, &["log", "-1", "-z", LOG_FORMAT, "HEAD", "--"]),
        run(path, &["worktree", "list", "--porcelain"]),
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
    let mut counts = JoinSet::new();
    for (i, wt) in worktrees.iter_mut().enumerate() {
        if let Some(head) = &mut wt.head {
            head.truncate(abbrev);
        }
        if wt.path == here {
            wt.changed = Some(status.changed);
        } else if Path::new(&wt.path).is_dir() {
            let wt_path = PathBuf::from(&wt.path);
            counts.spawn(async move { (i, changed_count(wt_path).await) });
        }
    }
    while let Some(Ok((i, changed))) = counts.join_next().await {
        worktrees[i].changed = changed;
    }

    Some(GitInfo {
        branch: status.branch,
        head: last.as_ref().map(|l| l.short.clone()),
        upstream: status.upstream,
        ahead: status.ahead,
        behind: status.behind,
        changed: status.changed,
        last_commit: last.map(|l| l.commit),
        worktrees,
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
? untracked\0! ignored\0";
        let s = parse_status(raw);
        assert_eq!(
            s,
            Status {
                branch: Some("main".into()),
                upstream: Some("origin/main".into()),
                ahead: 2,
                behind: 3,
                changed: 3,
            }
        );
        let detached = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!(detached.branch, None);
        assert_eq!(detached.changed, 0);
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
}
