//! `git.diff` / `git.diffStat`: working tree (incl. untracked files) against HEAD or a
//! merge-base, parsed into per-file patches under a fixed size budget.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use coocoo_proto::{ApiError, ApiResult, DiffFile, DiffResult, DiffStat};
use tokio::io::AsyncReadExt;

use crate::git::{self, EMPTY_TREE};

/// Total patch text returned by one `git.diff`.
const PATCH_BUDGET: usize = 2 << 20;
/// Raw `git diff` output read before giving up on the remaining patches.
const READ_CAP: usize = PATCH_BUDGET * 2;
/// Untracked files larger than this are listed without content.
const UNTRACKED_MAX: u64 = 256 << 10;
/// Same heuristic as git: a NUL byte in the first 8000 bytes means binary.
const BINARY_SNIFF: usize = 8000;

/// Diff `cwd`'s working tree against HEAD, or against `merge-base(base, HEAD)`.
pub(crate) async fn diff(cwd: &Path, base: Option<&str>) -> ApiResult<DiffResult> {
    let top = git::toplevel(cwd).await?;
    let (rev, label) = resolve_base(&top, base).await?;

    let entries_args = ["diff", "--no-color", "--no-ext-diff", "-M", "--raw", "--numstat", "-z", rev.as_str(), "--"];
    let (entries, patch, untracked) =
        tokio::join!(git::run_ok(&top, &entries_args), read_patch(&top, &rev), untracked_paths(&top));
    let mut files = parse_entries(&entries?.stdout)?;
    let (patch, capped) = patch?;
    let untracked = untracked?;

    let mut budget = Budget { left: PATCH_BUDGET, truncated: capped };
    let blocks = split_blocks(&patch, capped);
    if blocks.len() < files.len() {
        budget.truncated = true;
    }
    for (file, block) in files.iter_mut().zip(blocks) {
        if file.binary || block_is_binary(block) {
            file.binary = true;
            continue;
        }
        let hunks = hunks(block);
        if budget.take(hunks.len()) {
            file.patch = String::from_utf8_lossy(hunks).into_owned();
        }
    }

    let (untracked_files, budget) =
        tokio::task::spawn_blocking(move || render_untracked(&top, untracked, Some(budget)))
            .await
            .map_err(ApiError::internal)?;
    files.extend(untracked_files);
    let truncated = budget.is_some_and(|b| b.truncated);

    Ok(DiffResult { cwd: cwd.to_string_lossy().into_owned(), base: label, files, truncated })
}

/// Counts for the same comparison as [`diff`].
pub(crate) async fn diff_stat(cwd: &Path, base: Option<&str>) -> ApiResult<DiffStat> {
    let top = git::toplevel(cwd).await?;
    let (rev, _) = resolve_base(&top, base).await?;
    let args = ["diff", "--no-color", "--no-ext-diff", "-M", "--numstat", "-z", rev.as_str(), "--"];
    let (numstat, untracked) = tokio::join!(git::run_ok(&top, &args), untracked_paths(&top));
    let numstat = numstat?;
    let untracked = untracked?;

    let mut stat = DiffStat::default();
    for (add, del) in parse_numstat(&numstat.stdout) {
        stat.files += 1;
        stat.additions += add;
        stat.deletions += del;
    }
    let (files, _) = tokio::task::spawn_blocking(move || render_untracked(&top, untracked, None))
        .await
        .map_err(ApiError::internal)?;
    for file in files {
        stat.files += 1;
        stat.additions += file.additions;
    }
    Ok(stat)
}

/// `(revision to diff against, label reported to the client)`.
async fn resolve_base(top: &Path, base: Option<&str>) -> ApiResult<(String, String)> {
    let has_head = git::has_head(top).await?;
    let Some(base) = base.filter(|b| !b.is_empty()) else {
        let rev = if has_head { "HEAD" } else { EMPTY_TREE };
        return Ok((rev.to_owned(), "HEAD".to_owned()));
    };
    if !has_head {
        return Err(ApiError::conflict("repository has no commits yet; cannot diff against a base branch"));
    }
    if base.starts_with('-') {
        return Err(ApiError::invalid(format!("invalid base `{base}`")));
    }
    let out = git::run(top, &["merge-base", base, "HEAD"]).await?;
    if !out.ok {
        return Err(ApiError::invalid(format!("cannot find a merge-base of `{base}` and HEAD: {}", out.error_text())));
    }
    let sha = out.stdout_line();
    Ok((sha.clone(), sha))
}

struct Budget {
    left: usize,
    truncated: bool,
}

impl Budget {
    /// Reserve `len` bytes; marks the result truncated when it does not fit.
    fn take(&mut self, len: usize) -> bool {
        if len <= self.left {
            self.left -= len;
            true
        } else {
            self.truncated = true;
            false
        }
    }
}

// ─── tracked changes ─────────────────────────────────────────────────────────────────────────

/// Parse `git diff --raw --numstat -z` into files (patch still empty).
///
/// Raw records come first (`:<modes> <shas> <status>\0path\0[path\0]`), then the numstat
/// records in the same order (`add\tdel\tpath\0` or `add\tdel\t\0old\0new\0`).
fn parse_entries(raw: &[u8]) -> ApiResult<Vec<DiffFile>> {
    let mut files = Vec::new();
    let mut tokens = raw.split(|b| *b == 0).peekable();
    while let Some(tok) = tokens.next_if(|t| t.first() == Some(&b':')) {
        let meta = String::from_utf8_lossy(tok);
        let letter = meta.rsplit(' ').next().and_then(|s| s.chars().next()).unwrap_or('M');
        let first = lossy(tokens.next());
        let (old_path, path) = if matches!(letter, 'R' | 'C') { (Some(first), lossy(tokens.next())) } else { (None, first) };
        let status = match letter {
            'A' | 'C' => "A",
            'D' => "D",
            'R' => "R",
            _ => "M",
        };
        files.push(DiffFile {
            path,
            old_path: if letter == 'R' { old_path } else { None },
            status: status.to_owned(),
            additions: 0,
            deletions: 0,
            binary: false,
            patch: String::new(),
        });
    }
    let rest: Vec<&[u8]> = tokens.collect();
    let counts = parse_numstat_tokens(&rest);
    if counts.len() != files.len() {
        return Err(ApiError::internal("unexpected `git diff --raw --numstat` output"));
    }
    for (file, (add, del, binary)) in files.iter_mut().zip(counts) {
        file.additions = add;
        file.deletions = del;
        file.binary = binary;
    }
    Ok(files)
}

fn lossy(tok: Option<&[u8]>) -> String {
    String::from_utf8_lossy(tok.unwrap_or_default()).into_owned()
}

/// `(additions, deletions)` per file of `git diff --numstat -z` (binary counts as 0/0).
fn parse_numstat(raw: &[u8]) -> Vec<(u32, u32)> {
    let tokens: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    parse_numstat_tokens(&tokens).into_iter().map(|(a, d, _)| (a, d)).collect()
}

fn parse_numstat_tokens(tokens: &[&[u8]]) -> Vec<(u32, u32, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        i += 1;
        if tok.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(tok);
        let mut parts = text.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if path.is_empty() {
            // Rename: old and new path follow as separate tokens.
            i += 2;
        }
        let binary = add == "-" || del == "-";
        out.push((add.parse().unwrap_or(0), del.parse().unwrap_or(0), binary));
    }
    out
}

/// Full `git diff` patch text, read up to [`READ_CAP`]; `true` when cut short.
async fn read_patch(top: &Path, rev: &str) -> ApiResult<(Vec<u8>, bool)> {
    let mut child = git::command(top)
        .args(["diff", "--no-color", "--no-ext-diff", "-M", rev, "--"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(git::spawn_error)?;
    let Some(stdout) = child.stdout.take() else { return Err(ApiError::internal("git stdout unavailable")) };
    let mut buf = Vec::new();
    stdout.take(READ_CAP as u64 + 1).read_to_end(&mut buf).await?;
    let capped = buf.len() > READ_CAP;
    if capped {
        buf.truncate(READ_CAP);
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
    Ok((buf, capped))
}

/// Split patch text into per-file blocks at `diff --git ` lines. When the text was cut short
/// the trailing (incomplete) block is dropped.
fn split_blocks(patch: &[u8], capped: bool) -> Vec<&[u8]> {
    const HEADER: &[u8] = b"diff --git ";
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < patch.len() {
        if patch[pos..].starts_with(HEADER) {
            starts.push(pos);
        }
        match patch[pos..].iter().position(|b| *b == b'\n') {
            Some(nl) => pos += nl + 1,
            None => break,
        }
    }
    let mut blocks: Vec<&[u8]> =
        starts.iter().enumerate().map(|(i, &s)| &patch[s..starts.get(i + 1).copied().unwrap_or(patch.len())]).collect();
    if capped {
        blocks.pop();
    }
    blocks
}

/// The hunks of a block: everything from the first `@@` line.
fn hunks(block: &[u8]) -> &[u8] {
    if block.starts_with(b"@@") {
        return block;
    }
    match block.windows(3).position(|w| w == b"\n@@") {
        Some(i) => &block[i + 1..],
        None => &[],
    }
}

/// Binary blocks have no hunks, only a `Binary files … differ` / `GIT binary patch` marker.
fn block_is_binary(block: &[u8]) -> bool {
    let header_end = block.windows(3).position(|w| w == b"\n@@").unwrap_or(block.len());
    let header = &block[..header_end];
    header.windows(13).any(|w| w == b"\nBinary files") || header.windows(17).any(|w| w == b"\nGIT binary patch")
}

// ─── untracked files ─────────────────────────────────────────────────────────────────────────

/// Untracked, non-ignored files relative to `top`.
async fn untracked_paths(top: &Path) -> ApiResult<Vec<String>> {
    let out = git::run_ok(top, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty() && !p.ends_with(b"/"))
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// Content of a new file for display: `None` when binary or too large.
enum Content {
    Text(Vec<u8>),
    Opaque,
}

fn read_content(path: &Path) -> Content {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return Content::Opaque };
    if meta.file_type().is_symlink() {
        return match std::fs::read_link(path) {
            Ok(target) => Content::Text(target.into_os_string().into_encoded_bytes()),
            Err(_) => Content::Opaque,
        };
    }
    if !meta.is_file() || meta.len() > UNTRACKED_MAX {
        return Content::Opaque;
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    let Ok(file) = std::fs::File::open(path) else { return Content::Opaque };
    if file.take(UNTRACKED_MAX + 1).read_to_end(&mut buf).is_err() || buf.len() as u64 > UNTRACKED_MAX {
        return Content::Opaque;
    }
    if buf[..buf.len().min(BINARY_SNIFF)].contains(&0) {
        return Content::Opaque;
    }
    Content::Text(buf)
}

/// Render untracked files as all-added `DiffFile`s. With `budget = None` only counts lines.
fn render_untracked(top: &Path, paths: Vec<String>, mut budget: Option<Budget>) -> (Vec<DiffFile>, Option<Budget>) {
    let mut files = Vec::with_capacity(paths.len());
    for rel in paths {
        let full: PathBuf = top.join(&rel);
        let mut file = DiffFile {
            path: rel,
            old_path: None,
            status: "?".to_owned(),
            additions: 0,
            deletions: 0,
            binary: false,
            patch: String::new(),
        };
        match read_content(&full) {
            Content::Opaque => file.binary = true,
            Content::Text(bytes) => {
                file.additions = count_lines(&bytes);
                if let Some(budget) = budget.as_mut()
                    && file.additions > 0
                {
                    let patch = added_patch(&bytes, file.additions);
                    if budget.take(patch.len()) {
                        file.patch = patch;
                    }
                }
            }
        }
        files.push(file);
    }
    (files, budget)
}

fn count_lines(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    newlines + u32::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))
}

/// Unified hunk adding every line of `bytes`.
fn added_patch(bytes: &[u8], lines: u32) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut patch = String::with_capacity(text.len() + lines as usize + 64);
    patch.push_str(&format!("@@ -0,0 +1{} @@\n", if lines == 1 { String::new() } else { format!(",{lines}") }));
    for line in text.split_inclusive('\n') {
        patch.push('+');
        patch.push_str(line);
    }
    if !text.ends_with('\n') {
        patch.push_str("\n\\ No newline at end of file\n");
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_and_numstat() {
        let raw = b":100644 100644 7898192 0000000 M\0a\0:000000 100644 0000000 bdc955b A\0bin\0\
:100644 100644 6178079 6178079 R100\0b\0c\0:000000 100644 0000000 8ba3a16 A\0new\0\
1\t0\ta\0-\t-\tbin\0\x30\t0\t\0b\0c\0\x31\t0\tnew\0";
        let files = parse_entries(raw).unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!((files[0].path.as_str(), files[0].status.as_str(), files[0].additions), ("a", "M", 1));
        assert!(files[1].binary);
        assert_eq!(files[2].status, "R");
        assert_eq!(files[2].old_path.as_deref(), Some("b"));
        assert_eq!(files[2].path, "c");
        assert_eq!((files[3].path.as_str(), files[3].status.as_str()), ("new", "A"));
    }

    #[test]
    fn blocks_and_hunks() {
        let patch = b"diff --git a/a b/a\nindex 1..2 100644\n--- a/a\n+++ b/a\n@@ -1 +1,2 @@\n a\n+aa\n\
diff --git a/bin b/bin\nnew file mode 100644\nindex 0..1\nBinary files /dev/null and b/bin differ\n";
        let blocks = split_blocks(patch, false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(hunks(blocks[0]), b"@@ -1 +1,2 @@\n a\n+aa\n");
        assert!(!block_is_binary(blocks[0]));
        assert!(block_is_binary(blocks[1]));
        assert_eq!(split_blocks(patch, true).len(), 1);
    }

    #[test]
    fn untracked_patch() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a"), 1);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(added_patch(b"a\nb\n", 2), "@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(added_patch(b"x", 1), "@@ -0,0 +1 @@\n+x\n\\ No newline at end of file\n");
    }
}
