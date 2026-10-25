//! Per-project secrets for agents (`secrets.*`, the `coocoo_secret_request` host tool).
//!
//! The value travels client → daemon only: it never reaches omp's stdin, a tool argument or
//! result, the session or the timeline. It is kept in `<data_dir>/secrets/<projectId>.json`
//! (directory 0700, file 0600) and handed to every omp process of the project as an env var
//! (plus `COOCOO_SECRET_<NAME>` when omp's name pattern would not pick the name up), with omp's
//! secret obfuscation switched on, so the model sees placeholders where the value appears.
//! omp loads secrets only when a session starts, so an agent that got a new secret is restarted
//! once its turn ends. Optionally the value is also written into a file in the agent's
//! directory (`NAME=value` in a dotenv file, or the whole file), which is then kept out of git
//! via `.git/info/exclude` unless it is ignored already.
//!
//! omp masks only what goes to the provider; its RPC events carry tool output verbatim, so every
//! timeline item, delta, dialog and output text is passed through a [`Redactor`] before it is
//! stored or sent. Re-encoded values (base64, hex, substrings) are not caught by either.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{new_id, now_ms};
use coocoo_proto::{
    AgentRef, ApiError, ApiResult, Project, SecretFile, SecretFileTarget, SecretFormat, SecretInfo, SecretUiPayload,
    SecretsEvent, TimelineItem, UiRequest, events,
};
use omp_rpc::HostToolContext;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::Agents;
use crate::state::{Agent, Life};

const MAX_NAME: usize = 100;
const MAX_VALUE: usize = 64 * 1024;
const MAX_DESCRIPTION: usize = 1000;
/// Shorter values are not redacted: replacing every occurrence of a 1–3 character string would
/// wreck the transcript (omp itself ignores values under 8 characters).
const MIN_REDACT: usize = 4;
/// A value cut off by a truncation (`…`) is redacted from this many leading characters on.
const MIN_TRUNCATED_PREFIX: usize = 4;
/// How often a waiting `coocoo_secret_request` checks whether omp cancelled the call.
const CANCEL_POLL: Duration = Duration::from_millis(500);
/// Env var names omp treats as secrets (`/(KEY|SECRET|…)(_|$)/i`, omp 18.12).
const OMP_SECRET_WORDS: &[&str] = &["KEY", "SECRET", "TOKEN", "PASSWORD", "PASS", "AUTH", "CREDENTIAL", "PRIVATE", "OAUTH"];
/// Prefix of the shadow variable that makes omp mask a value whose name it would not pick up.
pub(crate) const SHADOW_PREFIX: &str = "COOCOO_SECRET_";
/// Names a secret must not take: they would break the agent's process or Coocoo's own variables.
const RESERVED: &[&str] = &["PATH", "HOME", "USER", "SHELL", "PWD", "TERM", "LANG", "TMPDIR", "IFS"];

/// One stored secret. No `Debug`: the value must not end up in a log line.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecretEntry {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<SecretFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<AgentRef>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SecretEntry {
    fn info(&self) -> SecretInfo {
        SecretInfo {
            name: self.name.clone(),
            files: self.files.clone(),
            added_by: self.added_by.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A project's secrets as loaded from its store file, with the redactor built from them.
#[derive(Default)]
pub(crate) struct ProjectSecrets {
    entries: Vec<SecretEntry>,
    redactor: Arc<Redactor>,
}

/// An open `coocoo_secret_request`, answered by `secrets.provide` or a decline.
pub(crate) struct Waiter {
    agent_id: String,
    name: String,
    file: Option<FileSpec>,
    tx: oneshot::Sender<Answer>,
}

enum Answer {
    Provided(Saved),
    Declined,
}

#[derive(Clone)]
struct FileSpec {
    rel: String,
    format: SecretFormat,
}

/// What saving a secret did.
struct Saved {
    info: SecretInfo,
    /// Characters of the value.
    length: usize,
    /// Files added to `.git/info/exclude` (relative paths).
    excluded: Vec<String>,
}

/// What the env of a starting omp process gets, and the redactor for what it may print.
pub(crate) struct SpawnSecrets {
    pub env: Vec<(String, String)>,
    /// Name → `updatedAt` of every secret the process gets.
    pub versions: HashMap<String, i64>,
    pub redactor: Arc<Redactor>,
}

// ─── redaction ───────────────────────────────────────────────────────────────────────────────

/// Replaces known secret values with `••••NAME`.
#[derive(Default)]
pub(crate) struct Redactor {
    /// `(needle, replacement)`, longest needle first; each value also in its JSON-escaped form
    /// (tool arguments are stored as JSON text).
    pairs: Vec<(String, String)>,
}

pub(crate) fn mask(name: &str) -> String {
    format!("••••{name}")
}

impl Redactor {
    pub fn new<'a>(secrets: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut pairs = Vec::new();
        for (name, value) in secrets {
            if value.chars().count() < MIN_REDACT {
                continue;
            }
            let replacement = mask(name);
            // A JSON string literal: the escaped value between its quotes.
            let quoted = serde_json::to_string(value).unwrap_or_default();
            let escaped = quoted.get(1..quoted.len().saturating_sub(1)).unwrap_or(value);
            if escaped != value {
                pairs.push((escaped.to_owned(), replacement.clone()));
            }
            pairs.push((value.to_owned(), replacement));
        }
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        pairs.dedup_by(|a, b| a.0 == b.0);
        Self { pairs }
    }

    /// Both redactors' values in one.
    pub fn merged(a: &Redactor, b: &Redactor) -> Self {
        let mut pairs: Vec<(String, String)> = a.pairs.iter().chain(&b.pairs).cloned().collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        pairs.dedup_by(|a, b| a.0 == b.0);
        Self { pairs }
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// `text` with every known value replaced; `None` when nothing matched. A value cut off by
    /// a truncation at the very end (`…`) is replaced too.
    pub fn redact(&self, text: &str) -> Option<String> {
        if self.pairs.is_empty() {
            return None;
        }
        let mut out = Cow::Borrowed(text);
        for (needle, replacement) in &self.pairs {
            if out.contains(needle.as_str()) {
                out = Cow::Owned(out.replace(needle.as_str(), replacement));
            }
        }
        if let Some(head) = out.strip_suffix('…')
            && let Some((cut, replacement)) = self.cut_tail(head, MIN_TRUNCATED_PREFIX)
        {
            out = Cow::Owned(format!("{}{replacement}…", &head[..head.len() - cut]));
        }
        match out {
            Cow::Borrowed(_) => None,
            Cow::Owned(text) => Some(text),
        }
    }

    pub fn redact_string(&self, text: &mut String) -> bool {
        match self.redact(text) {
            Some(redacted) => {
                *text = redacted;
                true
            }
            None => false,
        }
    }

    /// Redact every string in `value` (keys stay).
    pub fn redact_value(&self, value: &mut Value) -> bool {
        match value {
            Value::String(s) => self.redact_string(s),
            Value::Array(items) => items.iter_mut().fold(false, |changed, v| self.redact_value(v) | changed),
            Value::Object(map) => map.values_mut().fold(false, |changed, v| self.redact_value(v) | changed),
            _ => false,
        }
    }

    pub fn redact_item(&self, item: &mut TimelineItem) -> bool {
        if self.pairs.is_empty() {
            return false;
        }
        let Ok(mut value) = serde_json::to_value(&*item) else { return false };
        if !self.redact_value(&mut value) {
            return false;
        }
        match serde_json::from_value(value) {
            Ok(redacted) => {
                *item = redacted;
                true
            }
            Err(err) => {
                tracing::warn!(%err, "redacted timeline item no longer decodes");
                false
            }
        }
    }

    /// Bytes at the end of `text` that are the beginning of a value (but not all of it): what a
    /// stream must hold back until the next chunk shows whether the value follows.
    pub fn held_len(&self, text: &str) -> usize {
        self.cut_tail(text, 1).map_or(0, |(cut, _)| cut)
    }

    /// The longest suffix of `text` (at least `min` chars) that is a proper prefix of a value.
    fn cut_tail(&self, text: &str, min: usize) -> Option<(usize, &str)> {
        let mut best: Option<(usize, &str)> = None;
        for (needle, replacement) in &self.pairs {
            for (end, _) in needle.char_indices().skip(min) {
                if end > text.len() {
                    break;
                }
                if best.is_some_and(|(len, _)| len >= end) {
                    continue;
                }
                if text.ends_with(&needle[..end]) {
                    best = Some((end, replacement));
                }
            }
        }
        best
    }
}

// ─── validation ──────────────────────────────────────────────────────────────────────────────

pub(crate) fn check_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !valid || name.len() > MAX_NAME {
        return Err(ApiError::invalid(format!("`{name}` is not a valid env var name (A–Z, 0–9 and _, not starting with a digit)")));
    }
    if RESERVED.contains(&name) || name.starts_with("COOCOO_") || name.starts_with("LD_") || name.starts_with("DYLD_") {
        return Err(ApiError::invalid(format!("`{name}` is reserved; pick another name")));
    }
    Ok(name.to_owned())
}

fn check_value(value: &str) -> ApiResult<()> {
    if value.is_empty() {
        return Err(ApiError::invalid("the value must not be empty"));
    }
    if value.len() > MAX_VALUE {
        return Err(ApiError::invalid(format!("the value is longer than {} KiB", MAX_VALUE / 1024)));
    }
    if value.contains('\0') {
        return Err(ApiError::invalid("the value must not contain NUL characters"));
    }
    Ok(())
}

/// Whether omp collects env var `name` as a secret by itself.
pub(crate) fn omp_masks_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    OMP_SECRET_WORDS.iter().any(|word| {
        upper.match_indices(word).any(|(at, _)| matches!(upper.as_bytes().get(at + word.len()), None | Some(b'_')))
    })
}

/// A relative path that stays below its directory: no root, no `..`, nothing inside `.git`.
pub(crate) fn check_rel_path(path: &str) -> ApiResult<PathBuf> {
    let invalid = |why: &str| ApiError::invalid(format!("file `{path}`: {why}"));
    let mut clean = PathBuf::new();
    for component in Path::new(path.trim()).components() {
        match component {
            Component::Normal(part) if part == ".git" => return Err(invalid("must not be inside .git")),
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(invalid("must not contain ..")),
            Component::RootDir | Component::Prefix(_) => return Err(invalid("must be relative to the working directory")),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(invalid("needs a file name"));
    }
    Ok(clean)
}

fn rel_display(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn parse_format(format: Option<&str>) -> ApiResult<SecretFormat> {
    match format.map(str::trim) {
        None | Some("" | "dotenv") => Ok(SecretFormat::Dotenv),
        Some("raw") => Ok(SecretFormat::Raw),
        Some(other) => Err(ApiError::invalid(format!("unknown file format `{other}` (dotenv or raw)"))),
    }
}

// ─── dotenv ──────────────────────────────────────────────────────────────────────────────────

/// `value` as a dotenv value. Kept verbatim whenever possible (so omp finds the value in the
/// file's content): bare when it has only harmless characters, else single-quoted (literal in
/// the common parsers, may span lines). Only a value containing `'` gets double quotes with
/// escapes.
pub(crate) fn dotenv_quote(value: &str) -> String {
    let bare = |c: char| c.is_ascii_alphanumeric() || "_-.:/+=@,%~".contains(c);
    if !value.is_empty() && value.chars().all(bare) {
        value.to_owned()
    } else if !value.contains('\'') {
        format!("'{value}'")
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// If `line` defines `name` (`NAME=…`, `export NAME = …`): the rest after `=`, and whether it
/// had `export`.
fn dotenv_definition<'a>(line: &'a str, name: &str) -> Option<(&'a str, bool)> {
    let rest = line.trim_start();
    let (rest, export) = match rest.strip_prefix("export") {
        Some(after) if after.starts_with([' ', '\t']) => (after.trim_start(), true),
        _ => (rest, false),
    };
    let rest = rest.strip_prefix(name)?.trim_start_matches([' ', '\t']);
    Some((rest.strip_prefix('=')?, export))
}

/// Whether a quoted value that starts in `first` (after the opening quote `q`) closes on that
/// line.
fn closes(text: &str, q: char) -> bool {
    let mut escaped = false;
    for c in text.chars() {
        match c {
            '\\' if q == '"' && !escaped => escaped = true,
            c if c == q && !escaped => return true,
            _ => escaped = false,
        }
    }
    false
}

/// `content` with the definition of `name` set to `value` (replacing the first definition in
/// place, dropping later ones, appending when there is none) or removed (`None`). Every other
/// line stays byte for byte.
pub(crate) fn dotenv_upsert(content: &str, name: &str, value: Option<&str>) -> String {
    let mut out = String::with_capacity(content.len() + 64);
    let mut lines = content.split_inclusive('\n');
    let mut written = false;
    while let Some(line) = lines.next() {
        let Some((rest, export)) = dotenv_definition(line, name) else {
            out.push_str(line);
            continue;
        };
        // A quoted value may continue on the following lines.
        let rest = rest.trim_start_matches([' ', '\t']);
        if let Some(q) = rest.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))
            && !closes(&rest[1..], q)
        {
            for next in lines.by_ref() {
                if closes(next, q) {
                    break;
                }
            }
        }
        if let Some(value) = value.filter(|_| !written) {
            let ending = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            out.push_str(&format!("{}{name}={}{ending}", if export { "export " } else { "" }, dotenv_quote(value)));
            written = true;
        }
    }
    if let Some(value) = value.filter(|_| !written) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("{name}={}\n", dotenv_quote(value)));
    }
    out
}

// ─── files and git ───────────────────────────────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git").arg("-C").arg(dir).args(args).stdin(std::process::Stdio::null()).output().ok()
}

fn git_line(dir: &Path, args: &[&str]) -> Option<String> {
    let out = git(dir, args)?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct GitState {
    repo: bool,
    ignored: bool,
    tracked: bool,
}

fn git_state(dir: &Path, rel: &Path) -> GitState {
    let Some(out) = git(dir, &["check-ignore", "-q", "--", &rel_display(rel)]) else {
        return GitState::default();
    };
    let ignored = match out.status.code() {
        Some(0) => true,
        Some(1) => false,
        // Not a repository (or git failed): nothing to keep the file out of.
        _ => return GitState::default(),
    };
    let tracked = git(dir, &["ls-files", "--error-unmatch", "--", &rel_display(rel)]).is_some_and(|o| o.status.success());
    GitState { repo: true, ignored, tracked }
}

fn git_branch(dir: &Path) -> Option<String> {
    git_line(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| !b.is_empty() && b != "HEAD")
}

/// Add `/<prefix><rel>` to the repository's `info/exclude` (shared by all its worktrees).
/// Returns `false` when it was listed already.
fn git_exclude(dir: &Path, rel: &Path) -> std::io::Result<bool> {
    let fail = || std::io::Error::other("git rev-parse failed");
    let prefix = git_line(dir, &["rev-parse", "--show-prefix"]).ok_or_else(fail)?;
    let exclude = git_line(dir, &["rev-parse", "--git-path", "info/exclude"]).ok_or_else(fail)?;
    let exclude = dir.join(exclude);
    let entry = format!("/{prefix}{}", rel_display(rel));
    let current = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    if current.lines().any(|line| line.trim() == entry) {
        return Ok(false);
    }
    if let Some(parent) = exclude.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&exclude)?;
    let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    writeln!(file, "{sep}# Secret written by Coocoo\n{entry}")?;
    Ok(true)
}

/// The absolute path for `rel` below `dir`, after making sure no existing symlink leads out of
/// `dir` and that the file itself, if it exists, is a regular file. Creates missing parents.
fn resolve_target(dir: &Path, rel: &Path) -> ApiResult<PathBuf> {
    let root = std::fs::canonicalize(dir)?;
    let target = root.join(rel);
    let escape = || ApiError::invalid(format!("file `{}` would lie outside {}", rel_display(rel), root.display()));
    let parent = target.parent().ok_or_else(escape)?;
    let mut existing = parent;
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing.parent().ok_or_else(escape)?;
    }
    if !std::fs::canonicalize(existing).map_err(|_| escape())?.starts_with(&root) {
        return Err(escape());
    }
    std::fs::create_dir_all(parent)?;
    if !std::fs::canonicalize(parent)?.starts_with(&root) {
        return Err(escape());
    }
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if !meta.file_type().is_file() => {
            Err(ApiError::invalid(format!("`{}` exists and is not a regular file (symlinks are not followed)", rel_display(rel))))
        }
        _ => Ok(target),
    }
}

/// Replace `path` atomically with `content`, keeping its permissions; a new file gets 0600.
fn write_private(path: &Path, content: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mode = match std::fs::metadata(path) {
        #[cfg(unix)]
        Ok(meta) => meta.permissions().mode() & 0o7777,
        #[cfg(not(unix))]
        Ok(_) => 0o600,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0o600,
        Err(err) => return Err(err),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.coocoo-{}", new_id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(mode);
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(content)?;
        // The umask may have narrowed the mode; an existing file's mode is kept exactly.
        #[cfg(unix)]
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.sync_all()
    });
    if let Err(err) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn read_or_empty(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

/// Write `value` for `name` into `rel` below `dir`. Returns the file record and whether the
/// file was added to `.git/info/exclude`.
fn write_new_file(dir: &Path, rel: &str, format: SecretFormat, name: &str, value: &str) -> ApiResult<(SecretFile, bool)> {
    let rel = check_rel_path(rel)?;
    let git = git_state(dir, &rel);
    if git.tracked {
        return Err(ApiError::invalid(format!(
            "`{}` is tracked by git, so the secret would be committed; use an untracked file (e.g. .env.local)",
            rel_display(&rel)
        )));
    }
    let target = resolve_target(dir, &rel)?;
    let content = match format {
        SecretFormat::Raw => value.to_owned(),
        SecretFormat::Dotenv => dotenv_upsert(&read_or_empty(&target)?, name, Some(value)),
    };
    write_private(&target, content.as_bytes())?;
    let excluded = git.repo && !git.ignored && git_exclude(dir, &rel)?;
    let file = SecretFile { path: target.display().to_string(), rel_path: rel_display(&rel), format, branch: git_branch(dir) };
    Ok((file, excluded))
}

/// Rewrite a file Coocoo wrote earlier with a new value. `Ok(false)`: its directory is gone
/// (a removed worktree), so the record is dropped.
fn rewrite_file(file: &SecretFile, name: &str, value: &str) -> ApiResult<bool> {
    let path = Path::new(&file.path);
    if !path.parent().is_some_and(Path::is_dir) {
        return Ok(false);
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(ApiError::invalid(format!("`{}` is no longer a regular file; not rewritten", file.path)));
    }
    let content = match file.format {
        SecretFormat::Raw => value.to_owned(),
        SecretFormat::Dotenv => dotenv_upsert(&read_or_empty(path)?, name, Some(value)),
    };
    write_private(path, content.as_bytes())?;
    Ok(true)
}

/// Take the secret back out of a file Coocoo wrote: its line in a dotenv file; a raw file only
/// while it still holds exactly the value.
fn unwrite_file(file: &SecretFile, name: &str, value: &str) -> std::io::Result<()> {
    let path = Path::new(&file.path);
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file()) {
        return Ok(());
    }
    match file.format {
        SecretFormat::Raw => {
            if std::fs::read(path)? == value.as_bytes() {
                std::fs::remove_file(path)?;
            }
        }
        SecretFormat::Dotenv => {
            let current = std::fs::read_to_string(path)?;
            let updated = dotenv_upsert(&current, name, None);
            if updated != current {
                write_private(path, updated.as_bytes())?;
            }
        }
    }
    Ok(())
}

// ─── store ───────────────────────────────────────────────────────────────────────────────────

fn store_dir_private(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectParams {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileParam {
    path: String,
    format: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetParams {
    project_id: String,
    name: String,
    value: String,
    file: Option<FileParam>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveParams {
    project_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProvideParams {
    id: String,
    request_id: String,
    value: String,
}

/// One save: the value (`None`: keep the stored one) and an extra file to write it into.
struct SetSpec {
    project_id: String,
    name: String,
    value: Option<String>,
    file: Option<(PathBuf, FileSpec)>,
    by: Option<AgentRef>,
}

/// What an agent asks for with `coocoo_secret_request`.
pub(crate) struct SecretAsk {
    pub name: String,
    pub description: String,
    pub file: Option<(String, Option<String>)>,
    pub multiline: bool,
    pub replace: bool,
}

impl Agents {
    pub(crate) async fn secret_method(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "secrets.list" => {
                let ProjectParams { project_id } = params(p)?;
                self.project(&project_id).await?;
                ok(self.secret_infos(&project_id)?)
            }
            "secrets.set" => {
                let p: SetParams = params(p)?;
                let project = self.project(&p.project_id).await?;
                let name = check_name(&p.name)?;
                check_value(&p.value)?;
                let file = p
                    .file
                    .filter(|f| !f.path.trim().is_empty())
                    .map(|f| -> ApiResult<_> { Ok((PathBuf::from(&project.path), FileSpec { rel: f.path, format: parse_format(f.format.as_deref())? })) })
                    .transpose()?;
                let spec = SetSpec { project_id: p.project_id, name, value: Some(p.value), file, by: None };
                let me = self.clone();
                let saved = tokio::task::spawn_blocking(move || me.save_secret(spec)).await.map_err(ApiError::internal)??;
                ok(saved.info)
            }
            "secrets.remove" => {
                let p: RemoveParams = params(p)?;
                self.project(&p.project_id).await?;
                let me = self.clone();
                tokio::task::spawn_blocking(move || me.remove_secret(&p.project_id, &p.name)).await.map_err(ApiError::internal)??;
                Ok(json!({}))
            }
            "secrets.provide" => self.provide_secret(params(p)?).await,
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    fn secrets_file(&self, project_id: &str) -> ApiResult<PathBuf> {
        if project_id.is_empty() || !project_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(ApiError::invalid(format!("invalid project id `{project_id}`")));
        }
        Ok(self.secrets_dir.join(format!("{project_id}.json")))
    }

    /// Run `f` on the project's secrets, loading them first. An unreadable store is moved aside
    /// (not overwritten by the next save) and the project starts empty.
    fn with_secrets<R>(&self, project_id: &str, f: impl FnOnce(&mut ProjectSecrets) -> R) -> ApiResult<R> {
        let path = self.secrets_file(project_id)?;
        let mut all = self.secrets.lock();
        let loaded = all.entry(project_id.to_owned()).or_insert_with(|| {
            let entries = match coocoo_proto::util::read_json::<Vec<SecretEntry>>(&path) {
                Ok(entries) => entries.unwrap_or_default(),
                Err(err) => {
                    let aside = path.with_extension(format!("json.broken-{}", now_ms()));
                    tracing::warn!(path = %path.display(), aside = %aside.display(), "unreadable secrets store moved aside: {err}");
                    let _ = std::fs::rename(&path, &aside);
                    Vec::new()
                }
            };
            let redactor = Arc::new(Redactor::new(entries.iter().map(|e| (e.name.as_str(), e.value.as_str()))));
            ProjectSecrets { entries, redactor }
        });
        Ok(f(loaded))
    }

    fn secret_infos(&self, project_id: &str) -> ApiResult<Vec<SecretInfo>> {
        self.with_secrets(project_id, |s| s.entries.iter().map(SecretEntry::info).collect())
    }

    fn secret_entry(&self, project_id: &str, name: &str) -> ApiResult<Option<SecretEntry>> {
        self.with_secrets(project_id, |s| s.entries.iter().find(|e| e.name == name).cloned())
    }

    /// The redactor for the project's current secrets (empty for agents outside a project).
    pub(crate) fn project_redactor(&self, project_id: Option<&str>) -> Arc<Redactor> {
        project_id.and_then(|id| self.with_secrets(id, |s| s.redactor.clone()).ok()).unwrap_or_default()
    }

    /// Env vars and redactor for an omp process of `project_id` that starts now.
    pub(crate) fn spawn_secrets(&self, project_id: Option<&str>) -> SpawnSecrets {
        let entries = project_id.and_then(|id| self.with_secrets(id, |s| s.entries.clone()).ok()).unwrap_or_default();
        let versions = entries.iter().map(|e| (e.name.clone(), e.updated_at)).collect();
        let redactor = Arc::new(Redactor::new(entries.iter().map(|e| (e.name.as_str(), e.value.as_str()))));
        SpawnSecrets { env: secret_env(&entries), versions, redactor }
    }

    /// Persist the project's secrets (0600 in a 0700 directory), rebuild its redactor and
    /// announce the list. Caller holds `secret_io`.
    fn store_secrets(&self, project_id: &str, entries: Vec<SecretEntry>) -> ApiResult<()> {
        let path = self.secrets_file(project_id)?;
        store_dir_private(&self.secrets_dir)?;
        let bytes = serde_json::to_vec_pretty(&entries).map_err(ApiError::internal)?;
        write_private(&path, &bytes)?;
        let infos = entries.iter().map(SecretEntry::info).collect();
        self.with_secrets(project_id, |s| {
            s.redactor = Arc::new(Redactor::new(entries.iter().map(|e| (e.name.as_str(), e.value.as_str()))));
            s.entries = entries;
        })?;
        self.emitter.emit(events::PROJECT_SECRETS, &SecretsEvent { project_id: project_id.to_owned(), secrets: infos });
        Ok(())
    }

    /// Create or replace a secret (blocking: files, git). Replacing rewrites every file it was
    /// written into; `spec.file` is written in addition.
    fn save_secret(&self, spec: SetSpec) -> ApiResult<Saved> {
        let _io = self.secret_io.lock();
        let mut entries = self.with_secrets(&spec.project_id, |s| s.entries.clone())?;
        let now = now_ms();
        let index = entries.iter().position(|e| e.name == spec.name);
        let mut entry = match index {
            Some(i) => entries[i].clone(),
            None => SecretEntry { name: spec.name.clone(), value: String::new(), files: Vec::new(), added_by: spec.by.clone(), created_at: now, updated_at: now },
        };
        let changed = spec.value.as_ref().is_some_and(|v| *v != entry.value);
        match spec.value {
            Some(value) => entry.value = value,
            None if index.is_none() => return Err(ApiError::not_found(format_args!("secret {}", spec.name))),
            None => {}
        }
        if changed {
            entry.updated_at = now;
            let mut kept = Vec::new();
            for file in std::mem::take(&mut entry.files) {
                if rewrite_file(&file, &entry.name, &entry.value)? {
                    kept.push(file);
                }
            }
            entry.files = kept;
        }
        let mut excluded = Vec::new();
        if let Some((dir, file)) = &spec.file {
            let (record, was_excluded) = write_new_file(dir, &file.rel, file.format, &entry.name, &entry.value)?;
            if was_excluded {
                excluded.push(record.rel_path.clone());
            }
            entry.files.retain(|f| f.path != record.path);
            entry.files.push(record);
        }
        let saved = Saved { info: entry.info(), length: entry.value.chars().count(), excluded };
        match index {
            Some(i) => entries[i] = entry,
            None => {
                entries.push(entry);
                entries.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
        self.store_secrets(&spec.project_id, entries)?;
        Ok(saved)
    }

    /// Delete a secret and take it out of the files it was written into (blocking).
    fn remove_secret(&self, project_id: &str, name: &str) -> ApiResult<()> {
        let _io = self.secret_io.lock();
        let mut entries = self.with_secrets(project_id, |s| s.entries.clone())?;
        let index = entries.iter().position(|e| e.name == name).ok_or_else(|| ApiError::not_found(format_args!("secret {name}")))?;
        let entry = entries.remove(index);
        for file in &entry.files {
            if let Err(err) = unwrite_file(file, &entry.name, &entry.value) {
                tracing::warn!(path = %file.path, %err, "could not take a deleted secret out of its file");
            }
        }
        self.store_secrets(project_id, entries)
    }

    // ─── coocoo_secret_request ───────────────────────────────────────────────────────────────

    /// The host tool: ask the user for a secret and wait for the answer (blocks the omp handler
    /// thread). Returns the text for the agent, which never contains the value.
    pub(crate) fn request_secret(self: &Arc<Self>, agent_id: &str, ask: SecretAsk, ctx: &HostToolContext) -> ApiResult<String> {
        let agent = self.get(agent_id)?;
        let summary = agent.summary();
        let project_id = summary
            .project_id
            .clone()
            .ok_or_else(|| ApiError::invalid("you are not working in a Coocoo project, so there is nowhere to keep a secret"))?;
        let name = check_name(&ask.name)?;
        let description: String = ask.description.trim().chars().take(MAX_DESCRIPTION).collect();
        if description.is_empty() {
            return Err(ApiError::invalid("`description` is required: tell the user why you need the secret"));
        }
        let file = ask
            .file
            .map(|(path, format)| -> ApiResult<FileSpec> {
                let rel = rel_display(&check_rel_path(&path)?);
                Ok(FileSpec { rel, format: parse_format(format.as_deref())? })
            })
            .transpose()?;
        let handle = self.handle();
        let project: Project = handle.block_on(self.project(&project_id))?;
        let cwd = PathBuf::from(&summary.cwd);

        if let Some(existing) = self.secret_entry(&project_id, &name)?.filter(|_| !ask.replace) {
            return self.reuse_secret(&agent, &project_id, existing, cwd, file);
        }

        let target = match &file {
            Some(f) => {
                let git = git_state(&cwd, Path::new(&f.rel));
                if git.tracked {
                    return Err(ApiError::invalid(format!("`{}` is tracked by git, so the secret would be committed; use an untracked file (e.g. .env.local)", f.rel)));
                }
                Some(SecretFileTarget { path: f.rel.clone(), format: f.format, git_ignored: git.ignored, exclude: git.repo && !git.ignored })
            }
            None => None,
        };
        let replace = self.secret_entry(&project_id, &name)?.is_some();
        let payload = SecretUiPayload {
            method: "secret".to_owned(),
            name: name.clone(),
            description,
            project_id: project_id.clone(),
            project_name: project.name.clone(),
            multiline: ask.multiline,
            replace,
            file: target,
            branch: summary.worktree.as_ref().map(|w| w.branch.clone()).or_else(|| git_branch(&cwd)),
            worktree: summary.worktree.is_some(),
        };
        let request = UiRequest {
            id: new_id(),
            agent_id: agent.id.clone(),
            method: "secret".to_owned(),
            payload: serde_json::to_value(&payload).map_err(ApiError::internal)?,
            created_at: now_ms(),
        };
        let (tx, mut rx) = oneshot::channel();
        self.secret_waiters.lock().insert(request.id.clone(), Waiter { agent_id: agent.id.clone(), name: name.clone(), file, tx });
        let added = {
            let mut st = agent.state.lock();
            let live = st.is_live();
            if live {
                st.add_request(request.clone());
            }
            live
        };
        if !added {
            self.secret_waiters.lock().remove(&request.id);
            return Err(ApiError::unavailable("the agent process is not running"));
        }
        self.emitter.emit(events::AGENT_REQUEST, &request);
        self.record_agent(&agent, "request.ask", |event| event.meta("method", "secret").meta("questions", 1));
        self.changed(&agent);

        let answer = handle.block_on(async {
            loop {
                tokio::select! {
                    answer = &mut rx => return answer.ok(),
                    () = tokio::time::sleep(CANCEL_POLL) => {
                        if ctx.is_cancelled() {
                            return None;
                        }
                    }
                }
            }
        });
        match answer {
            Some(Answer::Provided(saved)) => {
                agent.state.lock().secret_restart.push(name.clone());
                Ok(provided_text(&project.name, &name, &saved))
            }
            Some(Answer::Declined) => Ok(format!(
                "The user declined to provide {name}. Do not ask for it again in this turn; continue without it or ask the user how to proceed."
            )),
            None => {
                self.secret_waiters.lock().remove(&request.id);
                let resolved = agent.state.lock().resolve_request(&request.id);
                if let Some(request) = resolved {
                    self.emit_resolved(&agent.id, &request.id);
                    self.request_closed(&agent, &request, Some("omp"));
                    self.changed(&agent);
                }
                Ok(format!("The request for {name} was cancelled."))
            }
        }
    }

    /// `coocoo_secret_request` for a secret the project has already: no dialog. The value is
    /// written into the requested file if it is not there yet; an agent whose process started
    /// before the secret existed (or changed) is restarted after its turn.
    fn reuse_secret(&self, agent: &Arc<Agent>, project_id: &str, existing: SecretEntry, cwd: PathBuf, file: Option<FileSpec>) -> ApiResult<String> {
        let name = existing.name.clone();
        let mut notes = Vec::new();
        if let Some(file) = file {
            let wanted = std::fs::canonicalize(&cwd).unwrap_or_else(|_| cwd.clone()).join(&file.rel);
            if !existing.files.iter().any(|f| Path::new(&f.path) == wanted) {
                let rel = file.rel.clone();
                let saved = self.save_secret(SetSpec { project_id: project_id.to_owned(), name: name.clone(), value: None, file: Some((cwd, file)), by: None })?;
                let excluded = if saved.excluded.is_empty() { String::new() } else { format!(" (added {rel} to .git/info/exclude)") };
                notes.push(format!("wrote it into {rel}{excluded}"));
            }
        }
        let loaded = agent.state.lock().spawn_versions.get(&name) == Some(&existing.updated_at);
        let wrote = if notes.is_empty() { String::new() } else { format!(" Coocoo {}.", notes.join(", ")) };
        if loaded {
            return Ok(format!(
                "{name} is already set: use ${name} (or the file it was written into).{wrote} Never print, echo or re-encode its value."
            ));
        }
        agent.state.lock().secret_restart.push(name.clone());
        Ok(format!(
            "{name} is already stored for this project, but your running process started before it was set.{wrote} \
End your turn now with a one-line note; Coocoo restarts your session and tells you when to continue. Then use ${name}; never print, echo or re-encode its value."
        ))
    }

    /// `secrets.provide`: save the value the user typed for an open request and hand the agent
    /// the outcome. A failed save (bad path…) leaves the request open.
    async fn provide_secret(self: &Arc<Self>, p: ProvideParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let (name, file) = {
            let waiters = self.secret_waiters.lock();
            let waiter = waiters.get(&p.request_id).filter(|w| w.agent_id == p.id);
            let waiter = waiter.ok_or_else(|| ApiError::not_found(format_args!("secret request {}", p.request_id)))?;
            (waiter.name.clone(), waiter.file.clone())
        };
        check_value(&p.value)?;
        let summary = agent.summary();
        let project_id = summary.project_id.clone().ok_or_else(|| ApiError::conflict("the agent left its project"))?;
        let spec = SetSpec {
            project_id,
            name,
            value: Some(p.value),
            file: file.map(|f| (PathBuf::from(&summary.cwd), f)),
            by: Some(AgentRef { id: summary.id.clone(), title: summary.title.clone() }),
        };
        let me = self.clone();
        let saved = tokio::task::spawn_blocking(move || me.save_secret(spec)).await.map_err(ApiError::internal)??;
        let waiter = self.secret_waiters.lock().remove(&p.request_id);
        let resolved = agent.state.lock().resolve_request(&p.request_id);
        if let Some(request) = &resolved {
            self.emit_resolved(&agent.id, &request.id);
            self.request_closed(&agent, request, None);
            self.changed(&agent);
        }
        if let Some(waiter) = waiter {
            let _ = waiter.tx.send(Answer::Provided(saved));
        }
        Ok(json!({}))
    }

    /// The user declined an open secret request (`agents.respond` with `cancelled`).
    pub(crate) fn decline_secret(&self, agent: &Arc<Agent>, request_id: &str) -> ApiResult<Value> {
        let waiter = self.secret_waiters.lock().remove(request_id);
        let resolved = agent.state.lock().resolve_request(request_id);
        let Some(request) = resolved else {
            return Err(ApiError::not_found(format_args!("request {request_id}")));
        };
        self.emit_resolved(&agent.id, &request.id);
        self.request_closed(agent, &request, Some("user"));
        self.changed(agent);
        if let Some(waiter) = waiter {
            let _ = waiter.tx.send(Answer::Declined);
        }
        Ok(json!({}))
    }

    /// Restart the agent's process once it is idle if it got a secret its process does not have
    /// yet, then tell it to continue. Called whenever a run ends or a subagent finishes.
    pub(crate) fn restart_for_secrets_when_idle(&self, agent: &Arc<Agent>) {
        let names = {
            let mut st = agent.state.lock();
            if st.secret_restart.is_empty() || !st.is_live() || st.busy() || !st.requests.is_empty() || !st.subagents.is_empty() {
                return;
            }
            std::mem::take(&mut st.secret_restart)
        };
        let Some(me) = self.me.upgrade() else { return };
        let agent = agent.clone();
        self.handle().spawn(async move {
            if me.draining.load(std::sync::atomic::Ordering::Acquire) {
                // The restart reloads the secrets anyway.
                return;
            }
            {
                let mut st = agent.state.lock();
                if st.busy() {
                    // A prompt came in meanwhile; try again after that turn.
                    st.secret_restart.extend(names);
                    return;
                }
            }
            let mut names = names;
            names.sort();
            names.dedup();
            let list = names.join(", ");
            let note = format!("Restarting the session so it gets the secret{} {list}", if names.len() > 1 { "s" } else { "" });
            match me.stop_process(&agent, Life::Hibernated, Some(note)).await {
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(agent = %agent.id, %err, "restart for a new secret failed");
                    return;
                }
            }
            let vars = names.iter().map(|n| format!("${n}")).collect::<Vec<_>>().join(", ");
            let message = format!(
                "Coocoo restarted your session: the secret{} {list} {} now set as {vars} (and in the file{} you asked for, if any). \
Continue where you left off. Use the value only through the variable or the file; never print, echo or re-encode it.",
                if names.len() > 1 { "s" } else { "" },
                if names.len() > 1 { "are" } else { "is" },
                if names.len() > 1 { "s" } else { "" },
            );
            if let Err(err) = me.prompt_text(&agent.id, message).await {
                tracing::warn!(agent = %agent.id, %err, "cannot tell the agent that its secret is loaded");
            }
        });
    }
}

/// The env vars an omp process gets for `entries`: each secret, plus a shadow variable that
/// omp's name pattern picks up when the name itself would not be masked.
fn secret_env(entries: &[SecretEntry]) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for entry in entries {
        env.push((entry.name.clone(), entry.value.clone()));
        if !omp_masks_name(&entry.name) {
            env.push((format!("{SHADOW_PREFIX}{}", entry.name), entry.value.clone()));
        }
    }
    env
}

/// The tool result after the user saved a secret: status only.
fn provided_text(project: &str, name: &str, saved: &Saved) -> String {
    let mut text = format!("Saved {name} ({} chars) for project {project}:\n- env var ${name} for every agent of this project", saved.length);
    for file in &saved.info.files {
        let what = match file.format {
            SecretFormat::Dotenv => format!("line {name}=… in {}", file.rel_path),
            SecretFormat::Raw => format!("file {} (the whole content)", file.rel_path),
        };
        let excluded = if saved.excluded.contains(&file.rel_path) { format!(" (added {} to .git/info/exclude)", file.rel_path) } else { String::new() };
        text.push_str(&format!("\n- {what}{excluded}"));
    }
    text.push_str(&format!(
        "\nYour running process does not have it yet, and reading the file now would send the value to the model unmasked. \
End your turn now with a one-line note; Coocoo restarts your session and tells you when to continue. \
The value stays hidden from you: use ${name} or the file in commands, never print, echo or re-encode it."
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coocoo-secret-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dotenv_upsert_replaces_in_place_and_keeps_other_lines() {
        let before = "# keys\nexport API_KEY=old\nOTHER=1\nAPI_KEY=dup\n";
        assert_eq!(dotenv_upsert(before, "API_KEY", Some("new")), "# keys\nexport API_KEY=new\nOTHER=1\n");
        assert_eq!(dotenv_upsert("OTHER=1", "API_KEY", Some("v")), "OTHER=1\nAPI_KEY=v\n");
        assert_eq!(dotenv_upsert("", "API_KEY", Some("v")), "API_KEY=v\n");
        // A prefix of another name is not a match.
        assert_eq!(dotenv_upsert("API_KEY_2=x\n", "API_KEY", None), "API_KEY_2=x\n");
        assert_eq!(dotenv_upsert(before, "API_KEY", None), "# keys\nOTHER=1\n");
    }

    #[test]
    fn dotenv_upsert_replaces_multiline_values_whole() {
        let before = "A=1\nPEM='-----BEGIN\nabc\n-----END'\nB=2\n";
        assert_eq!(dotenv_upsert(before, "PEM", Some("x y")), "A=1\nPEM='x y'\nB=2\n");
        let escaped = "PEM=\"line \\\" still\nmore\"\nB=2\n";
        assert_eq!(dotenv_upsert(escaped, "PEM", None), "B=2\n");
    }

    #[test]
    fn dotenv_quoting_keeps_the_value_verbatim_when_it_can() {
        assert_eq!(dotenv_quote("sk-proj_abc.123/+="), "sk-proj_abc.123/+=");
        assert_eq!(dotenv_quote("a b#c\"$d"), "'a b#c\"$d'");
        assert_eq!(dotenv_quote("-----BEGIN\nX\n-----END"), "'-----BEGIN\nX\n-----END'");
        assert_eq!(dotenv_quote("it's \"x\" \\"), "\"it's \\\"x\\\" \\\\\"");
    }

    #[test]
    fn paths_must_stay_below_the_directory() {
        assert_eq!(check_rel_path("./config/.env").unwrap(), PathBuf::from("config/.env"));
        for bad in ["../.env", "a/../../b", "/etc/passwd", ".git/config", "", "."] {
            assert!(check_rel_path(bad).is_err(), "{bad} accepted");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_directory_are_refused() {
        let root = temp_dir();
        let (dir, outside) = (root.join("project"), root.join("outside"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("target"), dir.join("file-link")).unwrap();
        let err = write_new_file(&dir, "link/.env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap_err();
        assert!(err.message.contains("outside"), "{}", err.message);
        assert!(write_new_file(&dir, "file-link", SecretFormat::Raw, "API_KEY", "value1234").is_err());
        assert!(!outside.join(".env").exists() && !outside.join("target").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_files_are_private_and_existing_modes_are_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir();
        let (file, _) = write_new_file(&dir, "nested/.env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(Path::new(&file.path)), 0o600);
        assert_eq!(std::fs::read_to_string(&file.path).unwrap(), "API_KEY=value1234\n");
        std::fs::write(dir.join("shared.env"), "X=1\n").unwrap();
        std::fs::set_permissions(dir.join("shared.env"), std::fs::Permissions::from_mode(0o640)).unwrap();
        write_new_file(&dir, "shared.env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap();
        assert_eq!(mode(&dir.join("shared.env")), 0o640);
        assert_eq!(std::fs::read_to_string(dir.join("shared.env")).unwrap(), "X=1\nAPI_KEY=value1234\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unignored_files_are_excluded_and_tracked_ones_refused() {
        let dir = temp_dir();
        let run = |args: &[&str]| assert!(git(&dir, args).unwrap().status.success(), "git {args:?}");
        run(&["init", "-q"]);
        std::fs::write(dir.join(".gitignore"), "ignored.env\n").unwrap();
        std::fs::write(dir.join("tracked.env"), "X=1\n").unwrap();
        run(&["add", "tracked.env"]);
        let (_, excluded) = write_new_file(&dir, ".env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap();
        assert!(excluded);
        let status = git_line(&dir, &["status", "--porcelain", "--", ".env"]).unwrap();
        assert!(status.is_empty(), "still visible to git: {status}");
        // Listed once, however often it is written.
        let (_, again) = write_new_file(&dir, ".env", SecretFormat::Dotenv, "OTHER_KEY", "value5678").unwrap();
        assert!(!again);
        let (_, ignored) = write_new_file(&dir, "ignored.env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap();
        assert!(!ignored);
        let err = write_new_file(&dir, "tracked.env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap_err();
        assert!(err.message.contains("tracked"), "{}", err.message);
        assert_eq!(std::fs::read_to_string(dir.join("tracked.env")).unwrap(), "X=1\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unwrite_removes_only_what_coocoo_wrote() {
        let dir = temp_dir();
        let (dotenv, _) = write_new_file(&dir, ".env", SecretFormat::Dotenv, "API_KEY", "value1234").unwrap();
        std::fs::write(&dotenv.path, "A=1\nAPI_KEY=value1234\nB=2\n").unwrap();
        unwrite_file(&dotenv, "API_KEY", "value1234").unwrap();
        assert_eq!(std::fs::read_to_string(&dotenv.path).unwrap(), "A=1\nB=2\n");
        let (raw, _) = write_new_file(&dir, "key.pem", SecretFormat::Raw, "PEM", "pem-value").unwrap();
        let (edited, _) = write_new_file(&dir, "edited.pem", SecretFormat::Raw, "PEM", "pem-value").unwrap();
        std::fs::write(&edited.path, "changed by hand").unwrap();
        unwrite_file(&raw, "PEM", "pem-value").unwrap();
        unwrite_file(&edited, "PEM", "pem-value").unwrap();
        assert!(!Path::new(&raw.path).exists());
        assert!(Path::new(&edited.path).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn redactor_replaces_plain_escaped_and_truncated_values() {
        let r = Redactor::new([("API_KEY", "sk-abc123def456"), ("PEM", "a\"b\nc-long"), ("PIN", "123")]);
        assert_eq!(r.redact("key=sk-abc123def456;").as_deref(), Some("key=••••API_KEY;"));
        // Tool arguments are JSON text.
        assert_eq!(r.redact(r#"{"cmd":"echo a\"b\nc-long"}"#).as_deref(), Some(r#"{"cmd":"echo ••••PEM"}"#));
        assert_eq!(r.redact("cut sk-abc1…").as_deref(), Some("cut ••••API_KEY…"));
        // Values under MIN_REDACT characters are left alone.
        assert_eq!(r.redact("pin 123"), None);
        assert_eq!(r.redact("nothing here"), None);
    }

    #[test]
    fn redactor_holds_back_a_value_split_across_chunks() {
        let r = Redactor::new([("API_KEY", "sk-abc123def456")]);
        assert_eq!(r.held_len("value: sk-ab"), "sk-ab".len());
        assert_eq!(r.held_len("value: s"), 1);
        assert_eq!(r.held_len("value: x"), 0);
        // A complete value is redacted, not held.
        assert_eq!(r.held_len("sk-abc123def456"), 0);
    }

    #[test]
    fn redactor_covers_every_string_of_an_item() {
        let r = Redactor::new([("API_KEY", "sk-abc123def456")]);
        let mut item: TimelineItem = serde_json::from_value(json!({
            "seq": 1, "id": "1", "ts": 0, "kind": "tool", "text": "sk-abc123def456",
            "tool": { "name": "bash", "args": "{\"command\":\"echo sk-abc123def456\"}", "status": "ok", "output": "sk-abc123def456\n" }
        }))
        .unwrap();
        assert!(r.redact_item(&mut item));
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("sk-abc123def456"), "{json}");
        assert!(json.contains("••••API_KEY"));
    }

    #[test]
    fn names_omp_would_miss_get_a_shadow_variable() {
        let entry = |name: &str| SecretEntry { name: name.to_owned(), value: "value1234".to_owned(), files: Vec::new(), added_by: None, created_at: 0, updated_at: 0 };
        let env = secret_env(&[entry("OPENAI_API_KEY"), entry("STRIPE_SK")]);
        let names: Vec<&str> = env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["OPENAI_API_KEY", "STRIPE_SK", "COOCOO_SECRET_STRIPE_SK"]);
        assert!(env.iter().all(|(_, value)| value == "value1234"));
    }

    #[test]
    fn omp_name_pattern() {
        for name in ["OPENAI_API_KEY", "GH_TOKEN", "DB_PASSWORD", "AUTH_HEADER", "COOCOO_SECRET_STRIPE_SK", "PASS"] {
            assert!(omp_masks_name(name), "{name}");
        }
        for name in ["STRIPE_SK", "DATABASE_URL", "KEYS", "TOKENIZER"] {
            assert!(!omp_masks_name(name), "{name}");
        }
    }

    #[test]
    fn names_are_validated() {
        assert_eq!(check_name(" OPENAI_API_KEY ").unwrap(), "OPENAI_API_KEY");
        for bad in ["", "1ABC", "lower", "A-B", "PATH", "COOCOO_AGENT_ID", "LD_PRELOAD"] {
            assert!(check_name(bad).is_err(), "{bad} accepted");
        }
    }
}
