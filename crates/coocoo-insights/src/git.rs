//! Commits of a repository in a period (`git log` over its local branches, merges excluded).

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    /// Committer time, Unix ms.
    pub ts: i64,
    pub summary: String,
    pub added: u64,
    pub deleted: u64,
}

/// Commits with committer time in `[from, to)` on any local branch of the repository at `dir`.
/// Blocking.
pub fn log(dir: &Path, from: i64, to: i64) -> Result<Vec<Commit>, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["log", "--branches", "--no-merges", "--shortstat", "--format=%x1e%H%x1f%ct%x1f%s"])
        .arg(format!("--until=@{}", (to - 1).max(0) / 1000 + 1));
    // `--since=@0` matches nothing; an open-ended period simply has no lower bound.
    if from >= 1000 {
        cmd.arg(format!("--since=@{}", from / 1000));
    }
    let out = cmd
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(stderr.lines().next().unwrap_or("git log failed").to_owned());
    }
    let commits = parse(&String::from_utf8_lossy(&out.stdout));
    Ok(commits.into_iter().filter(|c| c.ts >= from && c.ts < to).collect())
}

fn parse(text: &str) -> Vec<Commit> {
    text.split('\x1e')
        .filter_map(|record| {
            let (head, stat) = record.split_once('\n').unwrap_or((record, ""));
            let mut fields = head.split('\x1f');
            let sha = fields.next()?.trim().to_owned();
            let ts = fields.next()?.trim().parse::<i64>().ok()? * 1000;
            let summary = fields.next().unwrap_or("").trim().to_owned();
            if sha.is_empty() {
                return None;
            }
            let (mut added, mut deleted) = (0, 0);
            // ` 3 files changed, 10 insertions(+), 2 deletions(-)`
            for part in stat.split(',') {
                let part = part.trim();
                let n = part.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
                if part.contains("insertion") {
                    added = n;
                } else if part.contains("deletion") {
                    deleted = n;
                }
            }
            Some(Commit { sha, ts, summary, added, deleted })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortstat_records() {
        let text = "\x1eaaa\x1f1700000000\x1fFix it\n\n 2 files changed, 10 insertions(+), 2 deletions(-)\n\x1ebbb\x1f1700000100\x1fOnly deletes\n\n 1 file changed, 4 deletions(-)\n\x1eccc\x1f1700000200\x1fEmpty\n";
        let c = parse(text);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].sha.as_str(), c[0].ts, c[0].added, c[0].deleted), ("aaa", 1_700_000_000_000, 10, 2));
        assert_eq!((c[1].added, c[1].deleted, c[2].added), (0, 4, 0));
    }
}
