//! `.coocoo/actions.toml`: the actions a project offers in the UI (the Run button of the project
//! and agent views, ⌘K). Read from the project's main checkout; invalid entries are skipped and
//! reported with their line so the rest stays usable.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use coocoo_proto::error::{ok, params};
use coocoo_proto::{ActionKind, ActionScope, ApiError, ApiResult, ProjectAction, TerminalAction, UrlTarget};
use serde::Deserialize;
use serde_json::Value;

use crate::Workspace;
use crate::fs::expand;
use crate::terminal::{Spawn, Terminals, size};

pub(crate) const DIR: &str = ".coocoo";
pub(crate) const FILE: &str = "actions.toml";
const MAX_SIZE: u64 = 64 * 1024;
const MAX_NAME: usize = 80;
const MAX_ID: usize = 64;

/// Icon names the UI ships (see `ui/src/lib/actions.ts`); anything else falls back to the
/// kind's default icon.
const ICONS: &[&str] = &[
    "play",
    "server",
    "test-tube",
    "terminal",
    "rocket",
    "globe",
    "bug",
    "package",
    "database",
    "wrench",
    "refresh",
    "message",
    "external-link",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Loaded {
    pub actions: Vec<ProjectAction>,
    pub errors: Vec<String>,
}

impl Loaded {
    fn error(message: String) -> Self {
        Self { actions: Vec::new(), errors: vec![message] }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunParams {
    project_id: String,
    action_id: String,
    /// The project directory, or an agent's directory inside it or one of its worktrees.
    cwd: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

impl Workspace {
    /// `actions.*`: starting a `run` action needs both the project (its parsed actions and
    /// worktrees) and the terminals.
    pub async fn dispatch_actions(self: &Arc<Self>, terminals: &Arc<Terminals>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "actions.run" => ok(self.run_action(terminals, params(p)?).await?),
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    async fn run_action(&self, terminals: &Arc<Terminals>, p: RunParams) -> ApiResult<coocoo_proto::TerminalInfo> {
        let project = self.get(&p.project_id)?;
        let action = project
            .actions
            .iter()
            .find(|a| a.id == p.action_id)
            .ok_or_else(|| ApiError::not_found(format!("action {} in {}", p.action_id, project.name)))?;
        let ActionKind::Run { command, cwd: sub } = &action.kind else {
            return Err(ApiError::invalid(format!("action {} is not a command; prompts and URLs are opened by the client", action.id)));
        };
        let base = p.cwd.trim_end_matches('/').to_owned();
        let base_real = std::fs::canonicalize(expand(&base)).map_err(|_| ApiError::not_found(format!("directory {base}")))?;
        let mut roots = vec![PathBuf::from(&project.path)];
        roots.extend(project.git.iter().flat_map(|g| g.worktrees.iter().map(|w| PathBuf::from(&w.path))));
        let inside = roots.iter().any(|root| base_real.starts_with(std::fs::canonicalize(root).unwrap_or_else(|_| root.clone())));
        if !inside {
            return Err(ApiError::invalid(format!("{base} is not inside {} or one of its worktrees", project.name)));
        }
        let dir = match sub {
            Some(sub) => {
                let dir = std::fs::canonicalize(base_real.join(sub))
                    .map_err(|_| ApiError::not_found(format!("directory {sub} (the action's cwd) in {base}")))?;
                // A symlink must not lead out of the base either.
                if !dir.starts_with(&base_real) {
                    return Err(ApiError::invalid(format!("the action's cwd {sub} leads outside {base}")));
                }
                dir
            }
            None => base_real.clone(),
        };
        if !dir.is_dir() {
            return Err(ApiError::invalid(format!("{} is not a directory", dir.display())));
        }
        let dir = dir.to_str().ok_or_else(|| ApiError::invalid("path is not valid UTF-8"))?.to_owned();
        let (cols, rows) = size(p.cols, p.rows);
        let spawn = Spawn {
            cwd: dir,
            command: Some(command.clone()),
            env: vec![
                ("COOCOO_PROJECT_DIR", project.path.clone()),
                ("COOCOO_WORKDIR", base_real.to_string_lossy().into_owned()),
                ("COOCOO_ACTION", action.id.clone()),
            ],
            action: Some(TerminalAction {
                project_id: project.id.clone(),
                action_id: action.id.clone(),
                name: action.name.clone(),
                base,
            }),
        };
        terminals.run_action(spawn, cols, rows).await
    }
}

/// Read and validate `<project>/.coocoo/actions.toml`; no file = no actions, no errors.
pub(crate) async fn load(project: &Path) -> Loaded {
    let file = project.join(DIR).join(FILE);
    let meta = match tokio::fs::metadata(&file).await {
        Ok(meta) => meta,
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
            return Loaded::default();
        }
        Err(err) => return Loaded::error(format!("{FILE}: {err}")),
    };
    if !meta.is_file() {
        return Loaded::error(format!("{FILE}: not a file"));
    }
    if meta.len() > MAX_SIZE {
        return Loaded::error(format!("{FILE}: larger than {} KiB", MAX_SIZE / 1024));
    }
    match tokio::fs::read_to_string(&file).await {
        Ok(src) => parse(&src),
        Err(err) => Loaded::error(format!("{FILE}: {err}")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    action: Vec<toml::Spanned<toml::Table>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    name: Option<String>,
    id: Option<String>,
    icon: Option<String>,
    #[serde(default)]
    scope: ActionScope,
    #[serde(default)]
    default: bool,
    confirm: Option<String>,
    run: Option<String>,
    cwd: Option<String>,
    prompt: Option<String>,
    worktree: Option<bool>,
    url: Option<String>,
    target: Option<UrlTarget>,
}

pub(crate) fn parse(src: &str) -> Loaded {
    let file: RawFile = match toml::from_str(src) {
        Ok(file) => file,
        Err(err) => {
            let at = err.span().map_or_else(String::new, |span| position(src, span.start));
            return Loaded::error(format!("{FILE}{at} – {}", first_line(err.message())));
        }
    };
    let mut out = Loaded::default();
    for (index, entry) in file.action.into_iter().enumerate() {
        let at = position(src, entry.span().start);
        let table = entry.into_inner();
        let label = match table.get("name").and_then(toml::Value::as_str) {
            Some(name) => format!("action “{name}”"),
            None => format!("action #{}", index + 1),
        };
        let result = toml::Value::Table(table)
            .try_into::<RawAction>()
            .map_err(|err| first_line(err.message()).to_owned())
            .and_then(validate);
        match result {
            Ok(action) if out.actions.iter().any(|a| a.id == action.id) => {
                out.errors.push(format!("{FILE}{at} – {label}: id “{}” is used twice", action.id));
            }
            Ok(action) => out.actions.push(action),
            Err(message) => out.errors.push(format!("{FILE}{at} – {label}: {message}")),
        }
    }
    out
}

fn validate(raw: RawAction) -> Result<ProjectAction, String> {
    let name = raw.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or("`name` is required")?;
    if name.chars().count() > MAX_NAME {
        return Err(format!("`name` is longer than {MAX_NAME} characters"));
    }
    let id = match raw.id.as_deref().map(str::trim) {
        Some(id) => {
            let valid = id.len() <= MAX_ID
                && id.starts_with(|c: char| c.is_ascii_alphanumeric())
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err("`id` may only use letters, digits, - and _".to_owned());
            }
            id.to_owned()
        }
        None => slug(name).ok_or("`name` needs a letter or digit, or set `id`")?,
    };
    let present = [raw.run.is_some(), raw.prompt.is_some(), raw.url.is_some()].iter().filter(|p| **p).count();
    if present != 1 {
        return Err("expected exactly one of `run`, `prompt`, `url`".to_owned());
    }
    let only = |set: bool, key: &str, kind: &str| if set { Err(format!("`{key}` only applies to `{kind}`")) } else { Ok(()) };
    let kind = if let Some(command) = raw.run {
        only(raw.worktree.is_some(), "worktree", "prompt")?;
        only(raw.target.is_some(), "target", "url")?;
        if command.trim().is_empty() {
            return Err("`run` is empty".to_owned());
        }
        ActionKind::Run { command, cwd: raw.cwd.map(|c| sub_dir(&c)).transpose()?.flatten() }
    } else if let Some(prompt) = raw.prompt {
        only(raw.cwd.is_some(), "cwd", "run")?;
        only(raw.target.is_some(), "target", "url")?;
        if prompt.trim().is_empty() {
            return Err("`prompt` is empty".to_owned());
        }
        ActionKind::Prompt { prompt, worktree: raw.worktree.unwrap_or(false) }
    } else {
        let url = raw.url.unwrap_or_default().trim().to_owned();
        only(raw.cwd.is_some(), "cwd", "run")?;
        only(raw.worktree.is_some(), "worktree", "prompt")?;
        let scheme_ok = url.starts_with("http://") || url.starts_with("https://");
        if !scheme_ok || url.chars().any(char::is_whitespace) {
            return Err("`url` must be an http:// or https:// address".to_owned());
        }
        ActionKind::Url { url, target: raw.target }
    };
    Ok(ProjectAction {
        id,
        name: name.to_owned(),
        icon: raw.icon.filter(|i| ICONS.contains(&i.as_str())),
        scope: raw.scope,
        default: raw.default,
        confirm: raw.confirm.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()),
        kind,
    })
}

/// A relative directory below the base; `None` for "." or empty.
fn sub_dir(cwd: &str) -> Result<Option<String>, String> {
    let path = Path::new(cwd.trim());
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err("`cwd` must be a relative path inside the project (no `..`)".to_owned()),
        }
    }
    Ok((!parts.is_empty()).then(|| parts.join("/")))
}

/// `Dev Server` → `dev-server`.
fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    (!out.is_empty()).then(|| out.chars().take(MAX_ID).collect())
}

/// `:LINE:COL` (1-based) of a byte offset.
fn position(src: &str, offset: usize) -> String {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!(":{line}:{col}")
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or(message).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_file_keeps_order_kinds_and_defaults() {
        let loaded = parse(
            r#"
[[action]]
name = "Dev Server"
icon = "server"
scope = "agent"
default = true
run = "npm run dev"
cwd = "./ui"

[[action]]
name = "Review diff"
prompt = "Review {branch}"

[[action]]
id = "ci"
name = "CI"
icon = "sparkles"
url = "https://example.com/ci"
target = "external"
confirm = "  "
"#,
        );
        assert_eq!(loaded.errors, Vec::<String>::new());
        let a = &loaded.actions;
        assert_eq!(a.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["dev-server", "review-diff", "ci"]);
        assert_eq!(a[0].scope, ActionScope::Agent);
        assert!(a[0].default);
        assert_eq!(a[0].kind, ActionKind::Run { command: "npm run dev".into(), cwd: Some("ui".into()) });
        assert_eq!(a[1].scope, ActionScope::Both);
        assert_eq!(a[1].kind, ActionKind::Prompt { prompt: "Review {branch}".into(), worktree: false });
        // Unknown icons fall back to the kind's default; a blank confirm is no confirm.
        assert_eq!(a[2].icon, None);
        assert_eq!(a[2].confirm, None);
        assert_eq!(a[2].kind, ActionKind::Url { url: "https://example.com/ci".into(), target: Some(UrlTarget::External) });
    }

    #[test]
    fn invalid_actions_are_skipped_with_their_line() {
        let loaded = parse(
            "[[action]]\nname = \"ok\"\nrun = \"true\"\n\n[[action]]\nname = \"lint\"\nrun = \"x\"\nurl = \"https://a\"\n\n[[action]]\nname = \"OK\"\nrun = \"again\"\n\n[[action]]\nrun = \"no name\"\n",
        );
        assert_eq!(loaded.actions.len(), 1);
        assert_eq!(
            loaded.errors,
            [
                "actions.toml:5:1 – action “lint”: expected exactly one of `run`, `prompt`, `url`",
                "actions.toml:10:1 – action “OK”: id “ok” is used twice",
                "actions.toml:14:1 – action #4: `name` is required",
            ]
        );
    }

    #[test]
    fn misplaced_and_unsafe_fields_are_rejected() {
        let cases = [
            ("run = \"x\"\ncwd = \"../other\"", "`cwd` must be a relative path inside the project (no `..`)"),
            ("run = \"x\"\ncwd = \"/etc\"", "`cwd` must be a relative path inside the project (no `..`)"),
            ("prompt = \"x\"\ncwd = \"ui\"", "`cwd` only applies to `run`"),
            ("run = \"x\"\nworktree = true", "`worktree` only applies to `prompt`"),
            ("url = \"file:///etc/passwd\"", "`url` must be an http:// or https:// address"),
            ("run = \"   \"", "`run` is empty"),
            ("id = \"a b\"\nrun = \"x\"", "`id` may only use letters, digits, - and _"),
        ];
        for (body, message) in cases {
            let loaded = parse(&format!("[[action]]\nname = \"n\"\n{body}\n"));
            assert!(loaded.actions.is_empty(), "{body}");
            assert_eq!(loaded.errors, [format!("actions.toml:1:1 – action “n”: {message}")], "{body}");
        }
        let loaded = parse("[[action]]\nname = \"n\"\nrun = \"x\"\nscope = \"everywhere\"\n");
        assert!(loaded.errors[0].contains("unknown variant `everywhere`"), "{:?}", loaded.errors);
        let loaded = parse("[[action]]\nname = \"n\"\nrun = \"x\"\ncommand = \"y\"\n");
        assert!(loaded.errors[0].contains("unknown field `command`"), "{:?}", loaded.errors);
    }

    #[test]
    fn broken_file_yields_only_the_error() {
        let loaded = parse("[[action]]\nname = \"a\"\nrun = \"x\"\n\n[[action]\nname = \"b\"\n");
        assert!(loaded.actions.is_empty());
        assert_eq!(loaded.errors.len(), 1);
        assert!(loaded.errors[0].starts_with("actions.toml:5:"), "{:?}", loaded.errors);
        // A misspelt table name is an error, not an empty list.
        let loaded = parse("[[actions]]\nname = \"a\"\nrun = \"x\"\n");
        assert!(loaded.errors[0].contains("unknown field `actions`"), "{:?}", loaded.errors);
    }

    #[tokio::test]
    async fn missing_file_is_no_actions() {
        let dir = std::env::temp_dir().join(format!("coocoo-actions-{}", coocoo_proto::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir).await, Loaded::default());
        std::fs::write(dir.join(DIR), "a file, not a directory").unwrap();
        assert_eq!(load(&dir).await, Loaded::default());
        std::fs::remove_file(dir.join(DIR)).unwrap();
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        std::fs::write(dir.join(DIR).join(FILE), "[[action]]\nname = \"T\"\nrun = \"true\"\n").unwrap();
        assert_eq!(load(&dir).await.actions[0].id, "t");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
