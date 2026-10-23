//! Per-project Kanban board (`boards.*`): inbox → todo → doing → done.
//!
//! The user fills `todo` (and pulls cards over from the inbox, where external sources drop new
//! entries). Starting a card hands it to one of the project's agents, or to a new one, and moves
//! it to `doing` with that agent noted on the card; the agent closes it with the `kanban_done`
//! host tool, which commits the work when its directory is a git repository. Stored in
//! `<data_dir>/boards/<project id>.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{new_id, now_ms, read_json, truncate, write_json_atomic};
use base64::Engine as _;
use coocoo_proto::{
    AgentStatus, ApiError, ApiResult, Attachment, Board, BoardEvent, BoardSource, Card, CardColumn, CommitInfo, Project, Refinement, events,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::Life;
use crate::{Agents, sandbox};

const MAX_TITLE: usize = 300;
const MAX_BODY: usize = 8 * 1024;
/// Remembered entry keys per source.
const MAX_SEEN: usize = 1000;
/// A newly added source imports only its newest entries; older ones are marked seen.
const FIRST_IMPORT: usize = 20;
/// The schedules an importer can run on, in minutes (15 min, hourly, every 6 h, daily).
pub(crate) const INTERVALS: [u32; 4] = [15, 60, 360, 1440];
const DEFAULT_INTERVAL_MINS: u32 = 60;
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SCRIPT: usize = 64 * 1024;
/// How often due sources are looked for.
const POLL_TICK: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FEED_BYTES: &str = "5000000";
/// Per-agent board of earlier versions, moved into the project board on startup.
const LEGACY_FILE: &str = "board.json";
const MAX_REFINEMENT: usize = 32 * 1024;
const MAX_ATTACHMENTS: usize = 10;
const MAX_ATTACHMENT_BYTES: usize = 5 << 20;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectParams {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardAddParams {
    project_id: String,
    title: String,
    body: Option<String>,
    column: Option<CardColumn>,
    /// Insert at the top of the column instead of the bottom.
    #[serde(default)]
    top: bool,
    /// Webhooks for start and done (http/https).
    start_url: Option<String>,
    done_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardUpdateParams {
    project_id: String,
    card_id: String,
    title: Option<String>,
    /// `""` clears it.
    body: Option<String>,
    column: Option<CardColumn>,
    /// Position in the (target) column, 0 = top; past the end = bottom.
    index: Option<usize>,
    /// `""` clears it.
    start_url: Option<String>,
    /// `""` clears it.
    done_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardParams {
    project_id: String,
    card_id: String,
}

/// `boards.cardStart` and `boards.cardRefine`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardStartParams {
    project_id: String,
    card_id: String,
    /// One of the project's agents; without it a new agent is created for the card.
    agent_id: Option<String>,
    /// New agent only: run it in its own git worktree.
    #[serde(default)]
    worktree: bool,
    /// New agent only: `provider/modelId`.
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardDoneParams {
    project_id: String,
    card_id: String,
    /// The agent that did the work: its directory is committed (default: the card's agent).
    agent_id: Option<String>,
    commit_message: Option<String>,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefinementSaveParams {
    project_id: String,
    card_id: String,
    /// The agent that refined it; relative attachment paths resolve in its directory.
    agent_id: Option<String>,
    /// Markdown.
    text: String,
    /// Image and HTML files to attach (UI mocks).
    #[serde(default)]
    attachments: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentParams {
    project_id: String,
    card_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceAddParams {
    project_id: String,
    kind: String,
    #[serde(default)]
    target: String,
    name: Option<String>,
    script: Option<String>,
    interval_mins: Option<u32>,
    /// The agent creating it.
    agent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceUpdateParams {
    project_id: String,
    source_id: String,
    /// `""` clears it (not for scripts).
    name: Option<String>,
    target: Option<String>,
    script: Option<String>,
    interval_mins: Option<u32>,
    /// The agent changing it.
    agent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceParams {
    project_id: String,
    source_id: String,
}

/// One entry from a source.
#[derive(Debug, PartialEq)]
struct Entry {
    /// Dedup key: feed guid/link, the output line, or a JSON line's `id`/`url`/`title`.
    key: String,
    title: String,
    body: Option<String>,
    url: Option<String>,
}

fn clean_title(title: &str) -> ApiResult<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(ApiError::invalid("title must not be empty"));
    }
    Ok(truncate(&title, MAX_TITLE))
}

fn clean_body(body: Option<String>) -> Option<String> {
    body.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()).map(|b| truncate(&b, MAX_BODY))
}

fn new_card(title: String, body: Option<String>, column: CardColumn) -> Card {
    let now = now_ms();
    Card {
        id: new_id(),
        title,
        body,
        column,
        url: None,
        source: None,
        created_at: now,
        updated_at: now,
        note: None,
        commit: None,
        agent_id: None,
        agent_title: None,
        start_url: None,
        done_url: None,
        hook_error: None,
        refining: false,
        refinement: None,
    }
}

/// MIME type of an attachable file (images and HTML), by extension.
fn attachment_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "html" | "htm" => "text/html",
        _ => return None,
    })
}

/// A safe file name for an attachment, unique among `taken`: the file's name with anything but
/// letters, digits, `.`, `-` and `_` replaced, prefixed with a counter when it is taken.
fn attachment_name<'a>(path: &Path, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let raw = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
    let base = if base.trim_start_matches('.').is_empty() { "attachment".to_owned() } else { base };
    let mut name = base.clone();
    let mut n = 2;
    while taken.clone().any(|t| t == name) {
        name = format!("{n}-{base}");
        n += 1;
    }
    name
}

/// Remove a directory and its contents; gone already is fine.
async fn remove_dir(dir: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// A webhook URL from the client: `""` = none; anything else must be http(s).
fn clean_url(url: &str) -> ApiResult<Option<String>> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(None);
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.chars().any(char::is_whitespace) {
        return Err(ApiError::invalid(format!("`{url}` is not an http(s) URL")));
    }
    Ok(Some(url.to_owned()))
}

fn check_interval(mins: u32) -> ApiResult<u32> {
    if INTERVALS.contains(&mins) {
        return Ok(mins);
    }
    Err(ApiError::invalid(format!("intervalMins must be one of {INTERVALS:?} (minutes), not {mins}")))
}

/// Importer label; `None` when blank.
fn clean_name(name: &str) -> Option<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then(|| truncate(&name, MAX_TITLE))
}

fn check_target(kind: &str, target: &str) -> ApiResult<String> {
    let target = target.trim();
    match kind {
        "feed" if target.starts_with("http://") || target.starts_with("https://") => {}
        "feed" => return Err(ApiError::invalid("a feed needs an http(s) URL")),
        _ if target.is_empty() => return Err(ApiError::invalid("the command must not be empty")),
        _ => {}
    }
    Ok(target.to_owned())
}

fn check_script(script: String) -> ApiResult<String> {
    if script.trim().is_empty() {
        return Err(ApiError::invalid("the script must not be empty"));
    }
    if script.len() > MAX_SCRIPT {
        return Err(ApiError::invalid(format!("the script is larger than {} KiB", MAX_SCRIPT / 1024)));
    }
    Ok(script)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hook {
    Start,
    Done,
}

impl Hook {
    /// The hook a move from `from` to `to` triggers: entering doing or done.
    fn for_move(from: CardColumn, to: CardColumn) -> Option<Hook> {
        match to {
            _ if from == to => None,
            CardColumn::Doing => Some(Hook::Start),
            CardColumn::Done => Some(Hook::Done),
            _ => None,
        }
    }
}

/// Vec index at which a card lands at `index` within `column` (cards in other columns keep
/// their positions).
fn insert_at(cards: &[Card], column: CardColumn, index: usize) -> usize {
    let positions: Vec<usize> = cards.iter().enumerate().filter(|(_, c)| c.column == column).map(|(i, _)| i).collect();
    match positions.get(index) {
        Some(&at) => at,
        None => positions.last().map_or(cards.len(), |&last| last + 1),
    }
}

fn card_mut<'a>(board: &'a mut Board, card_id: &str) -> ApiResult<&'a mut Card> {
    board.cards.iter_mut().find(|c| c.id == card_id).ok_or_else(|| ApiError::not_found(format_args!("card {card_id}")))
}

fn move_card(board: &mut Board, card_id: &str, column: CardColumn, index: usize) -> ApiResult<Card> {
    let at = board.cards.iter().position(|c| c.id == card_id).ok_or_else(|| ApiError::not_found(format_args!("card {card_id}")))?;
    let mut card = board.cards.remove(at);
    card.column = column;
    card.updated_at = now_ms();
    let to = insert_at(&board.cards, column, index);
    board.cards.insert(to, card.clone());
    Ok(card)
}

/// The card as the agent sees it: title, details, source link.
fn card_brief(card: &Card) -> String {
    let mut text = format!("**{}**", card.title);
    if let Some(body) = &card.body {
        text.push_str("\n\n");
        text.push_str(body);
    }
    if let Some(url) = &card.url {
        text.push_str(&format!("\n\nSource: {url}"));
    }
    text
}

/// The card's refinement for a prompt, with the stored attachments' paths.
fn refinement_brief(refinement: &Refinement, attachments: &Path) -> String {
    let mut text = refinement.text.clone();
    if !refinement.attachments.is_empty() {
        text.push_str("\n\nAttachments:");
        for a in &refinement.attachments {
            text.push_str(&format!("\n- {}", attachments.join(&a.name).display()));
        }
    }
    text
}

/// Prompt for a started card.
fn card_prompt(card: &Card, attachments: &Path) -> String {
    let mut text = format!("Work on this card from the project's Kanban board (card id `{}`):\n\n{}", card.id, card_brief(card));
    if let Some(refinement) = &card.refinement {
        text.push_str("\n\n## Refinement (agreed with the user; follow it)\n\n");
        text.push_str(&refinement_brief(refinement, attachments));
    }
    text.push_str(
        "\n\nWhen it is finished, call `kanban_done` with this card id and a commit message describing the change \
         (it commits your changes if this is a git repository and moves the card to Done). If you cannot finish it, \
         say why and leave the card in Doing.",
    );
    text
}

/// Prompt for refining a card before anyone implements it.
fn refine_prompt(card: &Card, attachments: &Path) -> String {
    let mut text = format!(
        "Refine this card from the project's Kanban board (card id `{}`) before it is implemented:\n\n{}",
        card.id,
        card_brief(card)
    );
    if let Some(refinement) = &card.refinement {
        text.push_str("\n\n## Earlier refinement (revise it)\n\n");
        text.push_str(&refinement_brief(refinement, attachments));
    }
    text.push_str(
        "\n\nDo not implement it and do not change the project. Work out what exactly should be built: research \
         everything you can find out yourself (the code, docs, the web, trying things out), and ask the user with \
         the `ask` tool only what you cannot find out or decide yourself (preferences, trade-offs, scope). Where it \
         helps, make UI mocks as images or self-contained HTML files outside the project (e.g. in /tmp). Finally \
         call `kanban_refine` with this card id, the refinement as Markdown (goal, behaviour, approach, open \
         questions and risks) and the mock files as attachments; it is shown in the chat and on the card, which \
         then waits in Todo until the user starts it. Keep your closing answer short: the refinement speaks for \
         itself.",
    );
    text
}

/// Merge legacy per-agent boards (`<agent dir>/board.json`) into their projects' boards. Cards
/// in doing/done keep the agent that had them. A migrated file is renamed, not deleted.
pub(crate) fn migrate_agent_boards(agents_root: &Path, boards_dir: &Path, project_of: impl Fn(&str) -> Option<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(agents_root) else { return };
    for entry in entries.flatten() {
        let legacy = entry.path().join(LEGACY_FILE);
        if !legacy.is_file() {
            continue;
        }
        let agent_id = entry.file_name().to_string_lossy().into_owned();
        let Some((project_id, agent_title)) = project_of(&agent_id) else {
            tracing::warn!(agent = %agent_id, "board of an agent without project left in place");
            continue;
        };
        let old: Board = match read_json(&legacy) {
            Ok(Some(board)) => board,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(agent = %agent_id, "unreadable {LEGACY_FILE}: {err}");
                continue;
            }
        };
        let file = boards_dir.join(format!("{project_id}.json"));
        let mut board: Board = read_json(&file).ok().flatten().unwrap_or_default();
        for mut card in old.cards {
            if matches!(card.column, CardColumn::Doing | CardColumn::Done) && card.agent_id.is_none() {
                card.agent_id = Some(agent_id.clone());
                card.agent_title = Some(agent_title.clone());
            }
            board.cards.push(card);
        }
        board.sources.extend(old.sources);
        if let Err(err) = write_json_atomic(&file, &board) {
            tracing::warn!(project = %project_id, "cannot write board: {err}");
            continue;
        }
        let _ = std::fs::rename(&legacy, legacy.with_extension("json.migrated"));
        tracing::info!(agent = %agent_id, project = %project_id, "moved the agent's board to its project");
    }
}

impl Agents {
    pub(crate) async fn board_method(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "boards.get" => {
                let ProjectParams { project_id } = params(p)?;
                self.project(&project_id).await?;
                ok(self.edit_board(&project_id, |b| Ok((b.clone(), false)))?)
            }
            "boards.cardAdd" => {
                let p: CardAddParams = params(p)?;
                let (title, body) = (clean_title(&p.title)?, clean_body(p.body));
                let start_url = p.start_url.as_deref().map(clean_url).transpose()?.flatten();
                let done_url = p.done_url.as_deref().map(clean_url).transpose()?.flatten();
                self.project(&p.project_id).await?;
                let column = p.column.unwrap_or(CardColumn::Todo);
                ok(self.edit_board(&p.project_id, |b| {
                    let mut card = new_card(title, body, column);
                    (card.start_url, card.done_url) = (start_url, done_url);
                    let at = insert_at(&b.cards, column, if p.top { 0 } else { usize::MAX });
                    b.cards.insert(at, card.clone());
                    Ok((card, true))
                })?)
            }
            "boards.cardUpdate" => {
                let p: CardUpdateParams = params(p)?;
                let title = p.title.as_deref().map(clean_title).transpose()?;
                let start_url = p.start_url.as_deref().map(clean_url).transpose()?;
                let done_url = p.done_url.as_deref().map(clean_url).transpose()?;
                self.project(&p.project_id).await?;
                let (card, hook) = self.edit_board(&p.project_id, |b| {
                    let card = card_mut(b, &p.card_id)?;
                    if let Some(title) = title {
                        card.title = title;
                    }
                    if let Some(body) = p.body {
                        card.body = clean_body(Some(body));
                    }
                    if let Some(url) = start_url {
                        card.start_url = url;
                    }
                    if let Some(url) = done_url {
                        card.done_url = url;
                    }
                    card.updated_at = now_ms();
                    let column = card.column;
                    let card = match (p.column, p.index) {
                        (None, None) => card.clone(),
                        (to, index) => move_card(b, &p.card_id, to.unwrap_or(column), index.unwrap_or(usize::MAX))?,
                    };
                    let hook = Hook::for_move(column, card.column);
                    Ok(((card, hook), true))
                })?;
                if let Some(hook) = hook {
                    self.fire_hook(&p.project_id, &card, hook);
                }
                ok(card)
            }
            "boards.cardDelete" => {
                let p: CardParams = params(p)?;
                self.project(&p.project_id).await?;
                self.edit_board(&p.project_id, |b| {
                    let before = b.cards.len();
                    b.cards.retain(|c| c.id != p.card_id);
                    if b.cards.len() == before {
                        return Err(ApiError::not_found(format_args!("card {}", p.card_id)));
                    }
                    Ok(((), true))
                })?;
                remove_dir(&self.attachment_dir(&p.card_id)).await?;
                Ok(json!({}))
            }
            "boards.cardStart" => self.card_start(params(p)?, false).await,
            "boards.cardRefine" => self.card_start(params(p)?, true).await,
            "boards.refinementSave" => self.refinement_save(params(p)?).await,
            "boards.attachment" => self.attachment(params(p)?).await,
            "boards.cardDone" => self.card_done(params(p)?).await,
            "boards.sourceAdd" => {
                let p: SourceAddParams = params(p)?;
                let interval_mins = check_interval(p.interval_mins.unwrap_or(DEFAULT_INTERVAL_MINS))?;
                let name = p.name.as_deref().and_then(clean_name);
                let (target, script) = match p.kind.as_str() {
                    "feed" | "command" => (check_target(&p.kind, &p.target)?, None),
                    "script" if name.is_none() => return Err(ApiError::invalid("a script importer needs a name")),
                    "script" => (String::new(), Some(check_script(p.script.unwrap_or_default())?)),
                    other => return Err(ApiError::invalid(format!("unknown source kind `{other}` (feed, command, script)"))),
                };
                self.project(&p.project_id).await?;
                let agent = p.agent_id.map(|id| self.get(&id)).transpose()?.map(|a| a.summary());
                let source = BoardSource {
                    id: new_id(),
                    kind: p.kind,
                    target,
                    name,
                    script,
                    interval_mins,
                    checked_at: None,
                    error: None,
                    sandboxed: None,
                    agent_id: agent.as_ref().map(|a| a.id.clone()),
                    agent_title: agent.map(|a| a.title),
                    seen: Vec::new(),
                };
                let source_id = source.id.clone();
                self.edit_board(&p.project_id, |b| {
                    b.sources.push(source);
                    Ok(((), true))
                })?;
                let imported = self.refresh_source(&p.project_id, &source_id).await;
                self.source_run(&p.project_id, &source_id, imported)
            }
            "boards.sourceUpdate" => {
                let p: SourceUpdateParams = params(p)?;
                let interval_mins = p.interval_mins.map(check_interval).transpose()?;
                let script = p.script.map(check_script).transpose()?;
                self.project(&p.project_id).await?;
                let agent = p.agent_id.map(|id| self.get(&id)).transpose()?.map(|a| a.summary());
                // A given feed URL, command or script runs right away (also when unchanged).
                let rerun = self.edit_board(&p.project_id, |b| {
                    let source = b.sources.iter_mut().find(|s| s.id == p.source_id).ok_or_else(|| ApiError::not_found("source"))?;
                    let script_kind = source.kind == "script";
                    let mut rerun = false;
                    if let Some(target) = &p.target {
                        if script_kind {
                            return Err(ApiError::invalid("a script importer has no target; change its script"));
                        }
                        source.target = check_target(&source.kind, target)?;
                        rerun = true;
                    }
                    if let Some(script) = script {
                        if !script_kind {
                            return Err(ApiError::invalid(format!("a {} source has no script", source.kind)));
                        }
                        source.script = Some(script);
                        rerun = true;
                    }
                    if let Some(name) = &p.name {
                        let name = clean_name(name);
                        if name.is_none() && script_kind {
                            return Err(ApiError::invalid("a script importer needs a name"));
                        }
                        source.name = name;
                    }
                    if let Some(mins) = interval_mins {
                        source.interval_mins = mins;
                    }
                    if let Some(agent) = &agent {
                        source.agent_id = Some(agent.id.clone());
                        source.agent_title = Some(agent.title.clone());
                    }
                    Ok((rerun, true))
                })?;
                let imported = if rerun { self.refresh_source(&p.project_id, &p.source_id).await } else { 0 };
                self.source_run(&p.project_id, &p.source_id, imported)
            }
            "boards.sourceRemove" => {
                let p: SourceParams = params(p)?;
                self.project(&p.project_id).await?;
                let board = self.edit_board(&p.project_id, |b| {
                    b.sources.retain(|s| s.id != p.source_id);
                    Ok((b.clone(), true))
                })?;
                // A script's state directory goes with it.
                let _ = std::fs::remove_dir_all(self.importer_dir(&p.source_id));
                ok(board)
            }
            "boards.sourceRefresh" => {
                let p: SourceParams = params(p)?;
                self.project(&p.project_id).await?;
                let imported = self.refresh_source(&p.project_id, &p.source_id).await;
                self.source_run(&p.project_id, &p.source_id, imported)
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    /// The project (on this host); `not_found` when unknown, so no board file is made for it.
    async fn project(&self, project_id: &str) -> ApiResult<Project> {
        let value = self.orch()?.call(None, "projects.get".into(), json!({ "id": project_id })).await?;
        Ok(serde_json::from_value(value)?)
    }

    fn board_file(&self, project_id: &str) -> PathBuf {
        self.boards_dir.join(format!("{project_id}.json"))
    }

    /// Where a card's refinement attachments are kept.
    fn attachment_dir(&self, card_id: &str) -> PathBuf {
        self.boards_dir.join("attachments").join(card_id)
    }

    /// Working and state directory of a script importer (its files persist between runs).
    fn importer_dir(&self, source_id: &str) -> PathBuf {
        self.boards_dir.join("importers").join(source_id)
    }

    /// Result of adding/changing/running a source: `{ source, imported }` (`source.error` says
    /// why a run failed).
    fn source_run(&self, project_id: &str, source_id: &str, imported: usize) -> ApiResult<Value> {
        let source = self.edit_board(project_id, |b| Ok((b.sources.iter().find(|s| s.id == source_id).cloned(), false)))?;
        let mut source = source.ok_or_else(|| ApiError::not_found("source"))?;
        source.seen = Vec::new();
        Ok(json!({ "source": source, "imported": imported }))
    }

    /// Run `f` on the project's board (loaded on first use); persist and announce it when `f`
    /// reports a change. Callers check the project exists first.
    pub(crate) fn edit_board<R>(&self, project_id: &str, f: impl FnOnce(&mut Board) -> ApiResult<(R, bool)>) -> ApiResult<R> {
        let mut boards = self.boards.lock();
        let board = boards.entry(project_id.to_owned()).or_insert_with(|| match read_json::<Board>(&self.board_file(project_id)) {
            Ok(board) => board.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(project = %project_id, "unreadable board: {err}");
                Board::default()
            }
        });
        let (result, changed) = f(board)?;
        if changed {
            write_json_atomic(&self.board_file(project_id), &*board)?;
            self.emitter.emit(events::PROJECT_BOARD, &BoardEvent { project_id: project_id.to_owned(), board: board.clone() });
        }
        Ok(result)
    }

    /// Hand the card to an agent (`agentId`, else a new one): to implement it (`refine` false;
    /// it moves to doing) or to refine it first (it stays in todo, marked `refining`).
    async fn card_start(self: &Arc<Self>, p: CardStartParams, refine: bool) -> ApiResult<Value> {
        self.project(&p.project_id).await?;
        let card = self.edit_board(&p.project_id, |b| Ok((card_mut(b, &p.card_id)?.clone(), false)))?;
        if card.column == CardColumn::Doing || card.column == CardColumn::Done {
            let column = if card.column == CardColumn::Doing { "Doing" } else { "Done" };
            return Err(ApiError::conflict(format!("card “{}” is already in {column}", card.title)));
        }
        let attachments = self.attachment_dir(&card.id);
        let prompt = if refine { refine_prompt(&card, &attachments) } else { card_prompt(&card, &attachments) };
        let summary = match p.agent_id.filter(|a| !a.is_empty()) {
            Some(agent_id) => {
                let agent = self.get(&agent_id)?;
                let summary = agent.summary();
                if summary.project_id.as_deref() != Some(p.project_id.as_str()) {
                    return Err(ApiError::invalid(format!("agent “{}” does not belong to this project", summary.title)));
                }
                if agent.state.lock().life == Life::Stopped {
                    return Err(ApiError::conflict(format!("agent “{}” is archived", summary.title)));
                }
                // A card is a new task: it must not be queued behind (or steer) a running turn.
                if matches!(summary.status, AgentStatus::Starting | AgentStatus::Running | AgentStatus::Waiting) {
                    return Err(ApiError::conflict(format!("agent “{}” is still busy; wait until its answer is finished", summary.title)));
                }
                self.prompt_text(&agent_id, prompt).await?;
                summary
            }
            None => self.create_for_card(&p.project_id, &card.title, prompt, p.worktree, p.model).await?,
        };
        let (card, hook) = self.edit_board(&p.project_id, |b| {
            let c = card_mut(b, &p.card_id)?;
            c.agent_id = Some(summary.id.clone());
            c.agent_title = Some(summary.title.clone());
            c.refining = refine;
            let from = c.column;
            let card = match refine {
                false => move_card(b, &p.card_id, CardColumn::Doing, 0)?,
                // Refining accepts a card from the inbox: it is a task now.
                true if from == CardColumn::Inbox => move_card(b, &p.card_id, CardColumn::Todo, usize::MAX)?,
                true => c.clone(),
            };
            Ok(((card, (!refine).then_some(Hook::Start)), true))
        })?;
        if let Some(hook) = hook {
            self.fire_hook(&p.project_id, &card, hook);
        }
        ok(card)
    }

    /// Store an agent's refinement of a card (`kanban_refine`), replacing an earlier one and its
    /// attachments. The files are copied, so the agent may delete or change its own afterwards.
    async fn refinement_save(self: &Arc<Self>, p: RefinementSaveParams) -> ApiResult<Value> {
        self.project(&p.project_id).await?;
        let text = p.text.trim();
        if text.is_empty() {
            return Err(ApiError::invalid("the refinement must not be empty"));
        }
        if text.len() > MAX_REFINEMENT {
            return Err(ApiError::invalid(format!("the refinement is longer than {} KiB", MAX_REFINEMENT / 1024)));
        }
        if p.attachments.len() > MAX_ATTACHMENTS {
            return Err(ApiError::invalid(format!("at most {MAX_ATTACHMENTS} attachments")));
        }
        let agent = p.agent_id.as_deref().filter(|a| !a.is_empty()).map(|id| self.get(id)).transpose()?;
        let summary = agent.map(|a| a.summary());
        self.edit_board(&p.project_id, |b| card_mut(b, &p.card_id).map(|_| ((), false)))?;

        // Read and check every file before the stored ones are replaced.
        let mut files: Vec<(Attachment, Vec<u8>)> = Vec::with_capacity(p.attachments.len());
        for path in &p.attachments {
            let path = Path::new(path.trim());
            let path = match (&summary, path.is_absolute()) {
                (_, true) => path.to_path_buf(),
                (Some(s), false) => Path::new(&s.cwd).join(path),
                (None, false) => return Err(ApiError::invalid(format!("`{}`: give an absolute path", path.display()))),
            };
            let mime = attachment_mime(&path).ok_or_else(|| {
                ApiError::invalid(format!("`{}`: only images (png, jpg, gif, webp, svg) and HTML files can be attached", path.display()))
            })?;
            let bytes = tokio::fs::read(&path).await.map_err(|e| ApiError::invalid(format!("cannot read `{}`: {e}", path.display())))?;
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(ApiError::invalid(format!("`{}` is larger than {} MiB", path.display(), MAX_ATTACHMENT_BYTES >> 20)));
            }
            let name = attachment_name(&path, files.iter().map(|(a, _)| a.name.as_str()));
            files.push((Attachment { name, mime: mime.to_owned(), size: bytes.len() as u64 }, bytes));
        }
        let dir = self.attachment_dir(&p.card_id);
        remove_dir(&dir).await?;
        if !files.is_empty() {
            tokio::fs::create_dir_all(&dir).await?;
            for (attachment, bytes) in &files {
                tokio::fs::write(dir.join(&attachment.name), bytes).await?;
            }
        }

        let refinement = Refinement {
            text: text.to_owned(),
            attachments: files.into_iter().map(|(a, _)| a).collect(),
            agent_id: summary.as_ref().map(|s| s.id.clone()),
            agent_title: summary.map(|s| s.title),
            at: now_ms(),
        };
        ok(self.edit_board(&p.project_id, |b| {
            let c = card_mut(b, &p.card_id)?;
            c.refining = false;
            c.refinement = Some(refinement);
            c.updated_at = now_ms();
            Ok((c.clone(), true))
        })?)
    }

    /// `boards.attachment`: one file of a card's refinement as `{ mime, data }` (base64).
    async fn attachment(&self, p: AttachmentParams) -> ApiResult<Value> {
        self.project(&p.project_id).await?;
        let found = self.edit_board(&p.project_id, |b| {
            let card = card_mut(b, &p.card_id)?;
            Ok((card.refinement.as_ref().and_then(|r| r.attachments.iter().find(|a| a.name == p.name).cloned()), false))
        })?;
        // Only names on the card are read, so a name cannot point outside the directory.
        let attachment = found.ok_or_else(|| ApiError::not_found(format_args!("attachment {}", p.name)))?;
        let bytes = tokio::fs::read(self.attachment_dir(&p.card_id).join(&attachment.name)).await?;
        Ok(json!({ "mime": attachment.mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
    }

    async fn card_done(self: &Arc<Self>, p: CardDoneParams) -> ApiResult<Value> {
        let project = self.project(&p.project_id).await?;
        let card = self.edit_board(&p.project_id, |b| Ok((card_mut(b, &p.card_id)?.clone(), false)))?;
        let agent = p.agent_id.or(card.agent_id.clone()).and_then(|id| self.get(&id).ok());
        let cwd = agent.as_ref().map_or(project.path, |a| a.summary().cwd);
        let message = p.commit_message.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()).unwrap_or_else(|| card.title.clone());
        let (commit, skipped) = match self.orch()?.call(None, "git.commit".into(), json!({ "cwd": cwd, "message": message })).await {
            Ok(value) => (Some(serde_json::from_value::<CommitInfo>(value)?), None),
            Err(err) if err.code == "invalid_params" && err.message.contains("not a git repository") => (None, Some("not a git repository")),
            Err(err) if err.code == "conflict" => (None, Some("nothing to commit")),
            // Leave the card in place so the agent can fix the problem (e.g. a failing hook).
            Err(err) => return Err(ApiError::new(err.code, format!("commit failed: {}", err.message))),
        };
        let note = clean_body(p.note);
        let card = self.edit_board(&p.project_id, |b| {
            let c = card_mut(b, &p.card_id)?;
            c.note = note;
            c.commit = commit.clone();
            if let Some(agent) = &agent {
                let summary = agent.summary();
                c.agent_id = Some(summary.id);
                c.agent_title = Some(summary.title);
            }
            move_card(b, &p.card_id, CardColumn::Done, 0).map(|c| (c, true))
        })?;
        self.fire_hook(&p.project_id, &card, Hook::Done);
        Ok(json!({ "card": card, "commit": commit, "commitSkipped": skipped }))
    }

    /// POST the card's webhook for `hook`, if it has one, in the background. A failure is noted
    /// on the card (`hookError`) and never blocks the board.
    fn fire_hook(self: &Arc<Self>, project_id: &str, card: &Card, hook: Hook) {
        let Some(url) = (match hook {
            Hook::Start => card.start_url.clone(),
            Hook::Done => card.done_url.clone(),
        }) else {
            return;
        };
        let (me, project_id, card) = (self.clone(), project_id.to_owned(), card.clone());
        self.handle().spawn(async move {
            let project = me.project(&project_id).await.ok();
            let payload = hook_payload(&me, project.as_ref(), &card, hook);
            let result = post_json(&url, &payload).await;
            let label = if hook == Hook::Start { "start" } else { "done" };
            if let Err(err) = &result {
                tracing::warn!(card = %card.id, %url, "{label} webhook failed: {err}");
            }
            let _ = me.edit_board(&project_id, |b| {
                let Some(c) = b.cards.iter_mut().find(|c| c.id == card.id) else { return Ok(((), false)) };
                let error = result.err().map(|e| format!("{label} webhook: {e}"));
                let changed = c.hook_error != error;
                c.hook_error = error;
                Ok(((), changed))
            });
        });
    }

    /// Fetch one source and drop its new entries at the top of the inbox; returns how many
    /// cards it added. Commands run in the project directory, scripts sandboxed in their own.
    async fn refresh_source(&self, project_id: &str, source_id: &str) -> usize {
        let source = self.edit_board(project_id, |b| Ok((b.sources.iter().find(|s| s.id == source_id).cloned(), false)));
        let Ok(Some(source)) = source else { return 0 };
        let mut sandboxed = None;
        let fetched = match source.kind.as_str() {
            "feed" => fetch_feed(&source.target).await,
            kind => match self.project(project_id).await {
                Ok(project) if kind == "script" => {
                    let (result, confined) = run_script(source.script.as_deref().unwrap_or_default(), &self.importer_dir(source_id), &project.path).await;
                    sandboxed = Some(confined);
                    result
                }
                Ok(project) => run_command(&source.target, &project.path).await,
                Err(err) => Err(err.message),
            },
        };
        self.edit_board(project_id, |b| {
            let Some(source) = b.sources.iter_mut().find(|s| s.id == source_id) else { return Ok((0, false)) };
            source.checked_at = Some(now_ms());
            if sandboxed.is_some() {
                source.sandboxed = sandboxed;
            }
            let entries = match fetched {
                Ok(entries) => {
                    source.error = None;
                    entries
                }
                Err(err) => {
                    source.error = Some(err);
                    return Ok((0, true));
                }
            };
            let first = source.seen.is_empty();
            let fresh: Vec<Entry> = entries.into_iter().filter(|e| !source.seen.contains(&e.key)).collect();
            source.seen.extend(fresh.iter().map(|e| e.key.clone()));
            let excess = source.seen.len().saturating_sub(MAX_SEEN);
            source.seen.drain(..excess);
            let take = if first { FIRST_IMPORT } else { usize::MAX };
            let source_id = source.id.clone();
            let mut imported = 0;
            for entry in fresh.into_iter().take(take) {
                let Ok(title) = clean_title(&entry.title) else { continue };
                let mut card = new_card(title, clean_body(entry.body), CardColumn::Inbox);
                card.url = entry.url;
                card.source = Some(source_id.clone());
                let at = insert_at(&b.cards, CardColumn::Inbox, imported);
                b.cards.insert(at, card);
                imported += 1;
            }
            Ok((imported, true))
        })
        .unwrap_or(0)
    }

    /// Check due sources of every board every [`POLL_TICK`] while the daemon runs.
    pub(crate) fn start_board_poller(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        self.handle().spawn(async move {
            let mut tick = tokio::time::interval(POLL_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(agents) = me.upgrade() else { return };
                let projects: Vec<String> = std::fs::read_dir(&agents.boards_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
                    .collect();
                for project_id in projects {
                    // Boards of removed projects stay on disk but are not polled.
                    if agents.project(&project_id).await.is_err() {
                        continue;
                    }
                    let now = now_ms();
                    let due: Vec<String> = agents
                        .edit_board(&project_id, |b| {
                            let due = b.sources.iter().filter(|s| s.checked_at.is_none_or(|t| now - t >= s.interval_mins as i64 * 60_000));
                            Ok((due.map(|s| s.id.clone()).collect(), false))
                        })
                        .unwrap_or_default();
                    for id in due {
                        agents.refresh_source(&project_id, &id).await;
                    }
                }
            }
        });
    }
}

async fn output(mut command: tokio::process::Command, what: &str, timeout: Duration) -> Result<String, String> {
    command.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("{what} timed out after {} s", timeout.as_secs()))?
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    if !out.status.success() {
        // The last few stderr lines: enough to see a traceback's point of failure.
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = stderr.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        let reason = lines[lines.len().saturating_sub(5)..].join("\n");
        let reason = if reason.is_empty() { format!("exited with {}", out.status) } else { truncate(&reason, 1500) };
        return Err(if reason.starts_with(what) { reason } else { format!("{what}: {reason}") });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Webhook body. `text` is a one-line summary, so chat webhooks that read a `text` field (Slack,
/// Mattermost, ntfy via its JSON API…) show something useful without a template.
fn hook_payload(agents: &Agents, project: Option<&Project>, card: &Card, hook: Hook) -> Value {
    let agent = card.agent_id.as_ref().map(|id| {
        let title = agents.get(id).map(|a| a.summary().title).ok().or(card.agent_title.clone());
        json!({ "id": id, "title": title })
    });
    // Agents created for a card carry its title: naming them again adds nothing.
    let who = agent.as_ref().and_then(|a| a["title"].as_str()).filter(|t| *t != card.title).map(|t| format!(" ({t})")).unwrap_or_default();
    let text = match hook {
        Hook::Start => format!("▶ Started: {}{who}", card.title),
        Hook::Done => match &card.commit {
            Some(commit) => format!("✓ Done: {}{who} — {} {}", card.title, &commit.sha[..commit.sha.len().min(7)], commit.summary),
            None => format!("✓ Done: {}{who}", card.title),
        },
    };
    json!({
        "event": if hook == Hook::Start { "start" } else { "done" },
        "text": text,
        "card": {
            "id": card.id, "title": card.title, "body": card.body, "url": card.url,
            "column": card.column, "note": card.note, "commit": card.commit,
        },
        "agent": agent,
        "project": project.map(|p| json!({ "id": p.id, "name": p.name, "path": p.path })),
        "host": agents.host_id(),
        "ts": now_ms(),
    })
}

/// POST `body` as JSON with the `curl` CLI; any non-2xx status is an error.
async fn post_json(url: &str, body: &Value) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new("curl");
    command
        .args(["-fsS", "-o", "/dev/null", "--max-time", "15", "-X", "POST", "-H", "Content-Type: application/json", "-A", "coocoo"])
        .args(["--data-binary", "@-", "--", url])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| format!("cannot run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.to_string().as_bytes()).await;
    }
    let out = tokio::time::timeout(FETCH_TIMEOUT, child.wait_with_output()).await.map_err(|_| "timed out".to_owned())?.map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("failed").trim_start_matches("curl: ").to_owned())
}

/// Fetched with the `curl` CLI (no HTTP/TLS stack in the daemon).
async fn fetch_feed(url: &str) -> Result<Vec<Entry>, String> {
    let mut command = tokio::process::Command::new("curl");
    command.args(["-fsSL", "--max-time", "25", "--max-filesize", MAX_FEED_BYTES, "-A", "coocoo", "--", url]);
    parse_feed(&output(command, "curl", FETCH_TIMEOUT).await?)
}

async fn run_command(command_line: &str, cwd: &str) -> Result<Vec<Entry>, String> {
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", command_line]).current_dir(cwd);
    Ok(parse_lines(&output(command, "command", FETCH_TIMEOUT).await?))
}

/// Run a script importer in `dir` (its cwd, `$HOME` and state directory), sandboxed where
/// possible: it may read the system and the project and change only `dir`. The shebang line
/// picks the interpreter (`sh` without one). The environment is not inherited (it may hold
/// API keys). Returns the entries and whether it ran sandboxed.
async fn run_script(script: &str, dir: &Path, project_dir: &str) -> (Result<Vec<Entry>, String>, bool) {
    let tmp = dir.join("tmp");
    let file = dir.join("importer");
    if let Err(err) = std::fs::create_dir_all(&tmp).and_then(|()| std::fs::write(&file, script)) {
        return (Err(format!("cannot write the script: {err}")), false);
    }
    // Run the interpreter on the file instead of exec'ing the file itself: no exec bit, and no
    // ETXTBSY when another thread forks while the file is being written.
    let shebang = script.strip_prefix("#!").and_then(|s| s.lines().next()).map(str::trim).filter(|l| !l.is_empty());
    let mut command = match shebang {
        Some(line) => {
            let (interpreter, arg) = line.split_once(char::is_whitespace).map_or((line, None), |(i, a)| (i, Some(a.trim())));
            let mut command = tokio::process::Command::new(interpreter);
            command.args(arg);
            command
        }
        None => tokio::process::Command::new("sh"),
    };
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    let lang = std::env::var_os("LANG").unwrap_or_else(|| "C.UTF-8".into());
    command
        .arg(&file)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path)
        .env("LANG", lang)
        .env("HOME", dir)
        .env("TMPDIR", &tmp)
        .env("COOCOO_STATE_DIR", dir)
        .env("COOCOO_PROJECT_DIR", project_dir);
    let sandboxed = match sandbox::confine(&mut command, &[Path::new(project_dir)], &[dir]) {
        Ok(()) => true,
        Err(reason) => {
            tracing::warn!(dir = %dir.display(), "importer runs unsandboxed: {reason}");
            false
        }
    };
    (output(command, "script", SCRIPT_TIMEOUT).await.map(|out| parse_lines(&out)), sandboxed)
}

/// One entry per non-empty line: a JSON object `{ title, body?, url?, id? }` (deduped by `id`,
/// else `url`, else `title`; objects without a title are skipped) or plain text (the title,
/// deduped by the line).
fn parse_lines(out: &str) -> Vec<Entry> {
    let lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    lines
        .filter_map(|line| {
            let object = line.starts_with('{').then(|| serde_json::from_str::<serde_json::Map<String, Value>>(line).ok()).flatten();
            let Some(object) = object else {
                return Some(Entry { key: line.to_owned(), title: line.to_owned(), body: None, url: None });
            };
            let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
            let title = text("title")?;
            let url = text("url").filter(|u| u.starts_with("http://") || u.starts_with("https://"));
            let id = object.get("id").and_then(|v| v.as_str().map(str::to_owned).or_else(|| v.as_number().map(ToString::to_string)));
            Some(Entry { key: id.or_else(|| url.clone()).unwrap_or_else(|| title.clone()), title, body: text("body"), url })
        })
        .collect()
}

/// Items of an RSS 2.0/RDF (`item`) or Atom (`entry`) feed, in document order.
fn parse_feed(xml: &str) -> Result<Vec<Entry>, String> {
    use quick_xml::events::Event;

    #[derive(Default)]
    struct Item {
        title: String,
        link: Option<String>,
        id: Option<String>,
        body: String,
    }

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut item: Option<Item> = None;
    // Element whose text is being collected inside the item.
    let mut field: Option<String> = None;
    let mut text = String::new();
    let mut saw_feed = false;
    loop {
        match reader.read_event().map_err(|e| format!("not a feed: {e}"))? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_ascii_lowercase();
                match name.as_str() {
                    "rss" | "feed" | "rdf" => saw_feed = true,
                    "item" | "entry" => item = Some(Item::default()),
                    _ if item.is_some() && field.is_none() => {
                        if name == "link"
                            && let Some(href) = link_href(&e)
                        {
                            item.as_mut().expect("checked").link.get_or_insert(href);
                        }
                        field = Some(name);
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Empty(e) if item.is_some() => {
                if e.local_name().as_ref().eq_ignore_ascii_case("link")
                    && let Some(href) = link_href(&e)
                {
                    item.as_mut().expect("checked").link.get_or_insert(href);
                }
            }
            Event::Text(t) if field.is_some() => text.push_str(&t.xml10_content()),
            Event::CData(t) if field.is_some() => text.push_str(&t.into_inner()),
            Event::GeneralRef(r) if field.is_some() => match r.resolve_char_ref() {
                Ok(Some(c)) => text.push(c),
                _ => text.push_str(match &*r.xml10_content() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => " ",
                }),
            },
            Event::End(e) => {
                let name = e.local_name().as_ref().to_ascii_lowercase();
                if field.as_deref() == Some(name.as_str()) {
                    let value = std::mem::take(&mut text).trim().to_owned();
                    let it = item.as_mut().expect("fields only inside items");
                    match name.as_str() {
                        "title" => it.title = strip_html(&value),
                        "link" if !value.is_empty() => {
                            it.link.get_or_insert(value);
                        }
                        "guid" | "id" if !value.is_empty() => it.id = Some(value),
                        "description" | "summary" | "content" | "encoded" if it.body.is_empty() => it.body = strip_html(&value),
                        _ => {}
                    }
                    field = None;
                } else if (name == "item" || name == "entry")
                    && let Some(it) = item.take()
                {
                    let key = it.id.clone().or_else(|| it.link.clone()).unwrap_or_else(|| it.title.clone());
                    if !it.title.is_empty() && !key.is_empty() {
                        entries.push(Entry { key, title: it.title, body: Some(it.body).filter(|b| !b.is_empty()), url: it.link });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_feed {
        return Err("not an RSS or Atom feed".to_owned());
    }
    Ok(entries)
}

/// Atom `<link href>` (the `alternate` one, or one without `rel`).
fn link_href(e: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    let attr = |name: &str| {
        let value = e.try_get_attribute(name).ok().flatten()?;
        value.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.trim().to_owned())
    };
    if attr("rel").is_some_and(|r| r != "alternate") {
        return None;
    }
    attr("href").filter(|h| !h.is_empty())
}

/// Plain text of an HTML snippet (feed descriptions): tags dropped, common entities decoded,
/// whitespace collapsed.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, column: CardColumn) -> Card {
        Card { id: id.into(), ..new_card(id.into(), None, column) }
    }

    fn columns(board: &Board, column: CardColumn) -> Vec<&str> {
        board.cards.iter().filter(|c| c.column == column).map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn attachment_names_are_safe_and_unique() {
        let taken = ["mock.png", "2-mock.png"];
        assert_eq!(attachment_name(Path::new("/tmp/a/mock.png"), taken.iter().copied()), "3-mock.png");
        assert_eq!(attachment_name(Path::new("Main screen (dark).html"), [].iter().copied()), "Main-screen--dark-.html");
        assert_eq!(attachment_name(Path::new("/tmp/.."), [].iter().copied()), "attachment");
        assert_eq!(attachment_mime(Path::new("x.JPEG")), Some("image/jpeg"));
        assert_eq!(attachment_mime(Path::new("x.pdf")), None);
        assert_eq!(attachment_mime(Path::new("Makefile")), None);
    }

    #[test]
    fn moving_keeps_other_columns_and_respects_index() {
        let mut b = Board { cards: vec![card("a", CardColumn::Todo), card("x", CardColumn::Done), card("b", CardColumn::Todo), card("c", CardColumn::Todo)], sources: vec![] };
        move_card(&mut b, "c", CardColumn::Todo, 0).unwrap();
        assert_eq!(columns(&b, CardColumn::Todo), ["c", "a", "b"]);
        move_card(&mut b, "a", CardColumn::Doing, 0).unwrap();
        assert_eq!((columns(&b, CardColumn::Todo), columns(&b, CardColumn::Doing)), (vec!["c", "b"], vec!["a"]));
        move_card(&mut b, "b", CardColumn::Done, usize::MAX).unwrap();
        assert_eq!(columns(&b, CardColumn::Done), ["x", "b"]);
        assert!(move_card(&mut b, "nope", CardColumn::Done, 0).is_err());
    }

    #[test]
    fn webhooks_fire_on_entering_doing_or_done_only() {
        use CardColumn::*;
        let hooks: Vec<_> = [(Todo, Doing), (Doing, Doing), (Doing, Done), (Inbox, Done), (Done, Todo), (Todo, Inbox)]
            .map(|(from, to)| Hook::for_move(from, to).map(|h| h == Hook::Start))
            .to_vec();
        assert_eq!(hooks, [Some(true), None, Some(false), Some(false), None, None]);
        assert_eq!(clean_url(" https://x.y/h?a=1 ").unwrap().as_deref(), Some("https://x.y/h?a=1"));
        assert_eq!(clean_url("").unwrap(), None);
        assert!(clean_url("file:///etc/passwd").is_err() && clean_url("http://a b").is_err());
    }

    #[test]
    fn output_lines_become_entries() {
        let out = "plain line\n\n{\"title\": \"v2.1\", \"body\": \"notes\", \"url\": \"https://x.y/2.1\", \"id\": 21}\n{\"title\": \"no id\", \"url\": \"javascript:alert(1)\"}\n{\"body\": \"no title\"}\n{not json";
        let entries = parse_lines(out);
        let keys: Vec<(&str, &str, Option<&str>)> = entries.iter().map(|e| (e.key.as_str(), e.title.as_str(), e.url.as_deref())).collect();
        assert_eq!(keys, [("plain line", "plain line", None), ("21", "v2.1", Some("https://x.y/2.1")), ("no id", "no id", None), ("{not json", "{not json", None)]);
        assert_eq!(entries[1].body.as_deref(), Some("notes"));
    }

    #[test]
    fn agent_boards_move_into_their_projects() {
        let root = std::env::temp_dir().join(format!("coocoo-boards-{}", new_id()));
        let (agents_dir, boards_dir) = (root.join("agents"), root.join("boards"));
        let legacy = |agent: &str, cards: Vec<Card>| {
            std::fs::create_dir_all(agents_dir.join(agent)).unwrap();
            write_json_atomic(&agents_dir.join(agent).join(LEGACY_FILE), &Board { cards, sources: vec![] }).unwrap();
        };
        legacy("a1", vec![card("t1", CardColumn::Todo), card("d1", CardColumn::Done)]);
        legacy("a2", vec![card("g2", CardColumn::Doing)]);
        legacy("loose", vec![card("l", CardColumn::Todo)]);
        migrate_agent_boards(&agents_dir, &boards_dir, |id| (id != "loose").then(|| ("p1".to_owned(), format!("Agent {id}"))));
        let board: Board = read_json(&boards_dir.join("p1.json")).unwrap().unwrap();
        let cards: Vec<(&str, Option<&str>)> = board.cards.iter().map(|c| (c.id.as_str(), c.agent_id.as_deref())).collect();
        assert_eq!(cards, [("t1", None), ("d1", Some("a1")), ("g2", Some("a2"))], "only doing/done cards keep their agent");
        assert_eq!(board.cards[1].agent_title.as_deref(), Some("Agent a1"));
        assert!(!agents_dir.join("a1").join(LEGACY_FILE).exists() && agents_dir.join("a1/board.json.migrated").exists());
        assert!(agents_dir.join("loose").join(LEGACY_FILE).exists(), "no project: left alone");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rss_and_atom() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title>
            <item><title>First &amp; best</title><link>https://e.x/1</link><guid>g1</guid>
              <description><![CDATA[<p>Hello <b>world</b>&nbsp;!</p>]]></description></item>
            <item><title>Second</title><link>https://e.x/2</link></item>
          </channel></rss>"#;
        let items = parse_feed(rss).unwrap();
        assert_eq!(
            items,
            [
                Entry { key: "g1".into(), title: "First & best".into(), body: Some("Hello world !".into()), url: Some("https://e.x/1".into()) },
                Entry { key: "https://e.x/2".into(), title: "Second".into(), body: None, url: Some("https://e.x/2".into()) },
            ]
        );
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title>
            <entry><title type="html">Issue #4</title><id>tag:e.x,4</id>
              <link rel="replies" href="https://e.x/4/comments"/><link href="https://e.x/4"/>
              <summary>Fix it</summary></entry></feed>"#;
        let items = parse_feed(atom).unwrap();
        assert_eq!(items, [Entry { key: "tag:e.x,4".into(), title: "Issue #4".into(), body: Some("Fix it".into()), url: Some("https://e.x/4".into()) }]);
        assert!(parse_feed("<html><body>nope</body></html>").is_err());
        assert!(parse_feed("not xml <<<").is_err());
    }
}
