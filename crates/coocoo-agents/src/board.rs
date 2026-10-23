//! Per-project Kanban board (`boards.*`): inbox → todo → doing → done.
//!
//! The user fills `todo` (and pulls cards over from the inbox, where external sources drop new
//! entries). Starting a card hands it to one of the project's agents, or to a new one, and moves
//! it to `doing` with that agent noted on the card; the agent closes it with the `kanban_done`
//! host tool, which commits the work when its directory is a git repository. Stored in
//! `<data_dir>/boards/<project id>.json`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::insights::InsightEvent;
use coocoo_proto::util::{new_id, now_ms, read_json, truncate, write_json_atomic};
use base64::Engine as _;
use coocoo_proto::{
    AgentRef, AgentStatus, AgentSummary, ApiError, ApiResult, Attachment, Board, BoardEvent, BoardSource, Card, CardColumn, CommitInfo, DoneAction,
    PrState, Project, PullRequestInfo, PullRequestRef, Refinement, events,
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
struct BoardUpdateParams {
    project_id: String,
    done_action: Option<DoneAction>,
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
    /// Context images for the agent.
    #[serde(default)]
    images: Vec<ImageUpload>,
    /// The agent adding it (`kanban_add`); recorded as `addedBy`.
    agent_id: Option<String>,
}

/// An image the client sends with a card.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageUpload {
    /// File name; pasted images have none.
    name: Option<String>,
    mime_type: String,
    /// Base64.
    data: String,
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
    #[serde(default)]
    add_images: Vec<ImageUpload>,
    /// Names of images to remove.
    #[serde(default)]
    remove_images: Vec<String>,
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
    /// Pull request title and description when the board opens one (`DoneAction::PullRequest`).
    pr_title: Option<String>,
    pr_body: Option<String>,
}

/// Pull request title and description from the agent (`kanban_done`); Coocoo fills in what is
/// missing.
#[derive(Default)]
struct PrText {
    title: Option<String>,
    body: Option<String>,
}

/// What publishing a finished card did.
#[derive(Default)]
struct Published {
    pushed: bool,
    pull_request: Option<PullRequestRef>,
    error: Option<String>,
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
        done_at: None,
        note: None,
        commit: None,
        agent_id: None,
        agent_title: None,
        start_url: None,
        done_url: None,
        hook_error: None,
        added_by: None,
        images: Vec::new(),
        refining: false,
        refinement: None,
        pull_request: None,
        git_error: None,
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

/// Decode and check uploaded card images; names are made safe and unique among `taken` (the
/// card's images that stay) and each other.
fn decode_images(uploads: Vec<ImageUpload>, taken: &[String]) -> ApiResult<Vec<(Attachment, Vec<u8>)>> {
    if taken.len() + uploads.len() > MAX_ATTACHMENTS {
        return Err(ApiError::invalid(format!("a card can have at most {MAX_ATTACHMENTS} images")));
    }
    let mut out: Vec<(Attachment, Vec<u8>)> = Vec::with_capacity(uploads.len());
    for upload in uploads {
        let label = upload.name.clone().unwrap_or_else(|| "pasted image".to_owned());
        let ext = crate::image::ext(&upload.mime_type)
            .ok_or_else(|| ApiError::invalid(format!("{label}: only PNG, JPEG, GIF and WebP images can be attached")))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(upload.data.as_bytes())
            .map_err(|_| ApiError::invalid(format!("{label}: the image data is not valid base64")))?;
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(ApiError::invalid(format!("{label} is larger than {} MiB", MAX_ATTACHMENT_BYTES >> 20)));
        }
        let mut raw = upload.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("image.{ext}"));
        if attachment_mime(Path::new(&raw)).is_none_or(|m| !m.starts_with("image/") || m == "image/svg+xml") {
            raw = format!("{raw}.{ext}");
        }
        let names = taken.iter().map(String::as_str).chain(out.iter().map(|(a, _)| a.name.as_str()));
        let name = attachment_name(Path::new(&raw), names);
        out.push((Attachment { name, mime: upload.mime_type, size: bytes.len() as u64 }, bytes));
    }
    Ok(out)
}

/// Write decoded images into `dir`; on failure the ones written are removed again.
async fn write_images(dir: &Path, images: &[(Attachment, Vec<u8>)]) -> ApiResult<()> {
    if images.is_empty() {
        return Ok(());
    }
    tokio::fs::create_dir_all(dir).await?;
    for (i, (attachment, bytes)) in images.iter().enumerate() {
        if let Err(err) = tokio::fs::write(dir.join(&attachment.name), bytes).await {
            remove_images(dir, images[..i].iter().map(|(a, _)| a.name.as_str())).await;
            return Err(err.into());
        }
    }
    Ok(())
}

/// Delete image files (missing ones are fine).
async fn remove_images(dir: &Path, names: impl Iterator<Item = &str>) {
    for name in names {
        let _ = tokio::fs::remove_file(dir.join(name)).await;
    }
}

/// Where a card's refinement attachments are kept.
fn attachment_dir(boards_dir: &Path, card_id: &str) -> PathBuf {
    boards_dir.join("attachments").join(card_id)
}

/// Where a card's context images are kept (apart from the refinement's, which a new refinement
/// replaces wholesale).
fn image_dir(boards_dir: &Path, card_id: &str) -> PathBuf {
    boards_dir.join("cards").join(card_id)
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

/// Move a card; entering done stamps `done_at`, leaving done clears it.
fn move_card(board: &mut Board, card_id: &str, column: CardColumn, index: usize) -> ApiResult<Card> {
    let at = board.cards.iter().position(|c| c.id == card_id).ok_or_else(|| ApiError::not_found(format_args!("card {card_id}")))?;
    let mut card = board.cards.remove(at);
    let now = now_ms();
    card.done_at = match column {
        CardColumn::Done if card.column == CardColumn::Done => card.done_at,
        CardColumn::Done => Some(now),
        _ => None,
    };
    card.column = column;
    card.updated_at = now;
    let to = insert_at(&board.cards, column, index);
    board.cards.insert(to, card.clone());
    Ok(card)
}

/// Name of a column in the usage log (its wire name).
fn column_name(column: CardColumn) -> &'static str {
    match column {
        CardColumn::Inbox => "inbox",
        CardColumn::Todo => "todo",
        CardColumn::Doing => "doing",
        CardColumn::Done => "done",
    }
}

/// What the usage log compares between two states of a board: column and refinement time per card.
type CardMarks = HashMap<String, (CardColumn, Option<i64>)>;

fn card_marks(board: &Board) -> CardMarks {
    board.cards.iter().map(|c| (c.id.clone(), (c.column, c.refinement.as_ref().map(|r| r.at)))).collect()
}

/// Usage-log events (without project) for what changed on `board` since `before`. Diffing the
/// whole board also catches moves made on the side, like a card started inside another request.
fn card_events(before: &CardMarks, board: &Board) -> Vec<InsightEvent> {
    let mut events = Vec::new();
    for card in &board.cards {
        let refined_at = card.refinement.as_ref().map(|r| r.at);
        let Some(&(column, was_refined_at)) = before.get(&card.id) else {
            let mut event = InsightEvent::new("card.add")
                .card(card.id.as_str())
                .meta("column", column_name(card.column))
                .meta("byAgent", card.added_by.is_some());
            if let Some(by) = &card.added_by {
                event = event.agent(by.id.as_str());
            }
            events.push(event);
            continue;
        };
        if column != card.column {
            let mut event =
                InsightEvent::new("card.move").card(card.id.as_str()).meta("from", column_name(column)).meta("to", column_name(card.column));
            if let Some(agent) = &card.agent_id {
                event = event.agent(agent.as_str());
            }
            events.push(event);
        }
        if refined_at.is_some() && refined_at != was_refined_at {
            let mut event = InsightEvent::new("card.refined").card(card.id.as_str());
            if let Some(agent) = card.refinement.as_ref().and_then(|r| r.agent_id.as_ref()).or(card.agent_id.as_ref()) {
                event = event.agent(agent.as_str());
            }
            events.push(event);
        }
    }
    let current: HashSet<&str> = board.cards.iter().map(|c| c.id.as_str()).collect();
    events.extend(before.keys().filter(|id| !current.contains(id.as_str())).map(|id| InsightEvent::new("card.delete").card(id.as_str())));
    events
}

/// The card as the agent sees it: title, details, source link, images.
fn card_brief(card: &Card, boards_dir: &Path) -> String {
    let mut text = format!("**{}**", card.title);
    if let Some(body) = &card.body {
        text.push_str("\n\n");
        text.push_str(body);
    }
    if let Some(url) = &card.url {
        text.push_str(&format!("\n\nSource: {url}"));
    }
    if !card.images.is_empty() {
        text.push_str("\n\nImages (context from the user; look at them with your read tool):");
        let dir = image_dir(boards_dir, &card.id);
        for image in &card.images {
            text.push_str(&format!("\n- {}", dir.join(&image.name).display()));
        }
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
fn card_prompt(card: &Card, boards_dir: &Path, done_action: DoneAction) -> String {
    let attachments = &attachment_dir(boards_dir, &card.id);
    let mut text = format!("Work on this card from the project's Kanban board (card id `{}`):\n\n{}", card.id, card_brief(card, boards_dir));
    if let Some(refinement) = &card.refinement {
        text.push_str("\n\n## Refinement (agreed with the user; follow it)\n\n");
        text.push_str(&refinement_brief(refinement, attachments));
    }
    text.push_str(
        "\n\nWhen it is finished, call `kanban_done` with this card id and a commit message describing the change \
         (it commits your changes if this is a git repository and moves the card to Done). If you cannot finish it, \
         say why and leave the card in Doing.",
    );
    if let Some(hint) = done_hint(done_action) {
        text.push(' ');
        text.push_str(hint);
    }
    text
}

/// What `kanban_done` does on this board beyond committing, for the agent; `None` for commit only.
pub(crate) fn done_hint(action: DoneAction) -> Option<&'static str> {
    match action {
        DoneAction::Commit => None,
        DoneAction::Push => Some("On this board `kanban_done` also pushes the branch after the commit."),
        DoneAction::PullRequest => Some(
            "On this board `kanban_done` also pushes the branch and, for an agent in its own worktree, opens a pull \
             request: pass `prTitle` and `prBody` (what changed and why) with it.",
        ),
    }
}

/// The card's goal for a pull request description: the "Goal"/"Ziel" section of its refinement,
/// else its text, shortened.
fn card_goal(card: &Card) -> Option<String> {
    const MAX: usize = 600;
    let heading = |line: &str| line.trim_start().starts_with('#');
    let goal = card.refinement.as_ref().and_then(|r| {
        let mut lines = r.text.lines().skip_while(|l| {
            let name = l.trim_start().trim_start_matches('#').trim().trim_end_matches(':');
            !(heading(l) && (name.eq_ignore_ascii_case("goal") || name.eq_ignore_ascii_case("ziel")))
        });
        lines.next()?;
        let section: Vec<&str> = lines.take_while(|l| !heading(l)).collect();
        Some(section.join("\n").trim().to_owned()).filter(|s| !s.is_empty())
    });
    goal.or_else(|| card.body.clone()).map(|text| truncate(text.trim(), MAX)).filter(|t| !t.is_empty())
}

/// Pull request description from the card and the commits that go out with it.
fn default_pr_body(card: &Card, commits: &[CommitInfo]) -> String {
    let mut body = format!("Kanban: {} ({})", card.title, card.id);
    if let Some(goal) = card_goal(card) {
        body.push_str("\n\n");
        body.push_str(&goal);
    }
    if !commits.is_empty() {
        body.push_str("\n\nCommits:\n");
        for c in commits.iter().rev() {
            body.push_str(&format!("- {} {}\n", &c.sha[..c.sha.len().min(7)], c.summary));
        }
    }
    body.trim_end().to_owned()
}

/// Prompt for refining a card before anyone implements it.
fn refine_prompt(card: &Card, boards_dir: &Path) -> String {
    let attachments = &attachment_dir(boards_dir, &card.id);
    let mut text = format!(
        "Refine this card from the project's Kanban board (card id `{}`) before it is implemented:\n\n{}",
        card.id,
        card_brief(card, boards_dir)
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
            "boards.update" => {
                let p: BoardUpdateParams = params(p)?;
                self.project(&p.project_id).await?;
                ok(self.edit_board(&p.project_id, |b| {
                    let changed = p.done_action.is_some_and(|a| a != b.done_action);
                    if let Some(action) = p.done_action {
                        b.done_action = action;
                    }
                    Ok((b.clone(), changed))
                })?)
            }
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
                let images = decode_images(p.images, &[])?;
                let added_by = p.agent_id.as_deref().filter(|a| !a.is_empty()).map(|id| self.get(id)).transpose()?.map(|a| {
                    let s = a.summary();
                    AgentRef { id: s.id, title: s.title }
                });
                self.project(&p.project_id).await?;
                let column = p.column.unwrap_or(CardColumn::Todo);
                let mut card = new_card(title, body, column);
                (card.start_url, card.done_url, card.added_by) = (start_url, done_url, added_by);
                let dir = image_dir(&self.boards_dir, &card.id);
                write_images(&dir, &images).await?;
                card.images = images.into_iter().map(|(a, _)| a).collect();
                let added = self.edit_board(&p.project_id, |b| {
                    let at = insert_at(&b.cards, column, if p.top { 0 } else { usize::MAX });
                    b.cards.insert(at, card.clone());
                    Ok((card, true))
                });
                if added.is_err() {
                    remove_dir(&dir).await?;
                }
                ok(added?)
            }
            "boards.cardUpdate" => {
                let p: CardUpdateParams = params(p)?;
                let title = p.title.as_deref().map(clean_title).transpose()?;
                let start_url = p.start_url.as_deref().map(clean_url).transpose()?;
                let done_url = p.done_url.as_deref().map(clean_url).transpose()?;
                self.project(&p.project_id).await?;
                let dir = image_dir(&self.boards_dir, &p.card_id);
                let current = self.edit_board(&p.project_id, |b| Ok((card_mut(b, &p.card_id)?.images.clone(), false)))?;
                let kept: Vec<String> = current.iter().map(|a| a.name.clone()).filter(|n| !p.remove_images.contains(n)).collect();
                let added = decode_images(p.add_images, &kept)?;
                write_images(&dir, &added).await?;
                let added: Vec<Attachment> = added.into_iter().map(|(a, _)| a).collect();
                let result = self.edit_board(&p.project_id, |b| {
                    let card = card_mut(b, &p.card_id)?;
                    // Images added meanwhile by someone else still count against the limit.
                    card.images.retain(|a| !p.remove_images.contains(&a.name));
                    if card.images.len() + added.len() > MAX_ATTACHMENTS {
                        return Err(ApiError::invalid(format!("a card can have at most {MAX_ATTACHMENTS} images")));
                    }
                    card.images.extend(added.iter().cloned());
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
                });
                let (card, hook) = match result {
                    Ok(done) => done,
                    Err(err) => {
                        remove_images(&dir, added.iter().map(|a| a.name.as_str())).await;
                        return Err(err);
                    }
                };
                let removed = current.iter().map(|a| a.name.as_str()).filter(|n| !card.images.iter().any(|a| a.name == *n));
                remove_images(&dir, removed).await;
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
                remove_dir(&image_dir(&self.boards_dir, &p.card_id)).await?;
                Ok(json!({}))
            }
            "boards.cardStart" => self.card_start(params(p)?, false).await,
            "boards.cardRefine" => self.card_start(params(p)?, true).await,
            "boards.refinementSave" => self.refinement_save(params(p)?).await,
            "boards.attachment" => self.attachment(params(p)?, false).await,
            "boards.cardImage" => self.attachment(params(p)?, true).await,
            "boards.cardDone" => self.card_done(params(p)?).await,
            "boards.cardPublish" => self.card_publish(params(p)?).await,
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

    fn attachment_dir(&self, card_id: &str) -> PathBuf {
        attachment_dir(&self.boards_dir, card_id)
    }

    /// Path of one of a card's images, as the agent is told it.
    pub(crate) fn card_image_path(&self, card_id: &str, name: &str) -> PathBuf {
        image_dir(&self.boards_dir, card_id).join(name)
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
    /// reports a change, and report the card changes to the usage log. Callers check the project
    /// exists first.
    pub(crate) fn edit_board<R>(&self, project_id: &str, f: impl FnOnce(&mut Board) -> ApiResult<(R, bool)>) -> ApiResult<R> {
        let mut boards = self.boards.lock();
        let board = boards.entry(project_id.to_owned()).or_insert_with(|| match read_json::<Board>(&self.board_file(project_id)) {
            Ok(board) => board.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(project = %project_id, "unreadable board: {err}");
                Board::default()
            }
        });
        let before = self.insights.get().map(|_| card_marks(board));
        let (result, changed) = f(board)?;
        if changed {
            write_json_atomic(&self.board_file(project_id), &*board)?;
            if let Some(before) = before {
                for event in card_events(&before, board) {
                    self.record(|| event.project(Some(project_id)));
                }
            }
            self.emitter.emit(events::PROJECT_BOARD, &BoardEvent { project_id: project_id.to_owned(), board: board.clone() });
        }
        Ok(result)
    }

    /// Hand the card to an agent (`agentId`, else a new one): to implement it (`refine` false;
    /// it moves to doing) or to refine it first (it stays in todo, marked `refining`).
    async fn card_start(self: &Arc<Self>, p: CardStartParams, refine: bool) -> ApiResult<Value> {
        self.project(&p.project_id).await?;
        let (card, done_action) = self.edit_board(&p.project_id, |b| Ok(((card_mut(b, &p.card_id)?.clone(), b.done_action), false)))?;
        if card.column == CardColumn::Doing || card.column == CardColumn::Done {
            let column = if card.column == CardColumn::Doing { "Doing" } else { "Done" };
            return Err(ApiError::conflict(format!("card “{}” is already in {column}", card.title)));
        }
        let prompt = if refine { refine_prompt(&card, &self.boards_dir) } else { card_prompt(&card, &self.boards_dir, done_action) };
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

    /// `boards.attachment` / `boards.cardImage`: one file of a card's refinement, or one of its
    /// images, as `{ mime, data }` (base64).
    async fn attachment(&self, p: AttachmentParams, image: bool) -> ApiResult<Value> {
        self.project(&p.project_id).await?;
        let found = self.edit_board(&p.project_id, |b| {
            let card = card_mut(b, &p.card_id)?;
            let list = if image { Some(&card.images) } else { card.refinement.as_ref().map(|r| &r.attachments) };
            Ok((list.and_then(|l| l.iter().find(|a| a.name == p.name).cloned()), false))
        })?;
        // Only names on the card are read, so a name cannot point outside the directory.
        let attachment = found.ok_or_else(|| ApiError::not_found(format_args!("{} {}", if image { "image" } else { "attachment" }, p.name)))?;
        let dir = if image { image_dir(&self.boards_dir, &p.card_id) } else { self.attachment_dir(&p.card_id) };
        let bytes = tokio::fs::read(dir.join(&attachment.name)).await?;
        Ok(json!({ "mime": attachment.mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
    }

    async fn card_done(self: &Arc<Self>, p: CardDoneParams) -> ApiResult<Value> {
        let project = self.project(&p.project_id).await?;
        let (card, done_action) = self.edit_board(&p.project_id, |b| Ok(((card_mut(b, &p.card_id)?.clone(), b.done_action), false)))?;
        let agent = p.agent_id.or(card.agent_id.clone()).and_then(|id| self.get(&id).ok()).map(|a| a.summary());
        let cwd = agent.as_ref().map_or(project.path, |a| a.cwd.clone());
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
            c.git_error = None;
            if let Some(agent) = &agent {
                c.agent_id = Some(agent.id.clone());
                c.agent_title = Some(agent.title.clone());
            }
            move_card(b, &p.card_id, CardColumn::Done, 0).map(|c| (c, true))
        })?;
        self.fire_hook(&p.project_id, &card, Hook::Done);
        let mut result = json!({ "card": card, "commit": commit, "commitSkipped": skipped });
        // The commit is made: whatever happens to the push, the card stays done.
        if done_action != DoneAction::Commit && skipped != Some("not a git repository") {
            let pr_text = PrText { title: p.pr_title, body: p.pr_body };
            let (card, published) = self.publish_card(&p.project_id, &card.id, agent.as_ref(), &cwd, done_action, pr_text).await?;
            result["card"] = json!(card);
            result["pushed"] = json!(published.pushed);
            result["pullRequest"] = json!(published.pull_request);
            result["gitError"] = json!(published.error);
        }
        Ok(result)
    }

    /// `boards.cardPublish`: push (and open the pull request) for a done card again, e.g. after
    /// the push in `kanban_done` failed.
    async fn card_publish(self: &Arc<Self>, p: CardParams) -> ApiResult<Value> {
        let project = self.project(&p.project_id).await?;
        let (card, done_action) = self.edit_board(&p.project_id, |b| Ok(((card_mut(b, &p.card_id)?.clone(), b.done_action), false)))?;
        if card.column != CardColumn::Done {
            return Err(ApiError::conflict(format!("card “{}” is not done yet", card.title)));
        }
        let agent = match &card.agent_id {
            Some(id) => Some(self.get(id).map_err(|_| ApiError::conflict("the agent that finished the card no longer exists; push from its project instead"))?.summary()),
            None => None,
        };
        let cwd = agent.as_ref().map_or(project.path, |a| a.cwd.clone());
        // Retrying is an explicit request to push, also when the board only commits by now.
        let action = if done_action == DoneAction::Commit { DoneAction::Push } else { done_action };
        let (card, published) = self.publish_card(&p.project_id, &card.id, agent.as_ref(), &cwd, action, PrText::default()).await?;
        ok(json!({ "card": card, "pushed": published.pushed, "pullRequest": published.pull_request, "error": published.error }))
    }

    /// Push the agent's branch and, for [`DoneAction::PullRequest`] and worktree agents, open a
    /// pull request unless one is open already; the outcome is noted on the card (`gitError`,
    /// `pullRequest`, and the number of pushed commits when an agent in the shared checkout
    /// pushed more than its own).
    async fn publish_card(
        self: &Arc<Self>,
        project_id: &str,
        card_id: &str,
        agent: Option<&AgentSummary>,
        cwd: &str,
        action: DoneAction,
        text: PrText,
    ) -> ApiResult<(Card, Published)> {
        let orch = self.orch()?;
        let at = json!({ "projectId": project_id, "path": cwd });
        let call = |method: &str| orch.call(None, method.to_owned(), at.clone());
        let mut published = Published::default();
        let outgoing: Vec<CommitInfo> = match call("git.outgoing").await {
            Ok(v) => serde_json::from_value(v).unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        match call("git.push").await {
            Ok(v) if v["ok"] == true => published.pushed = true,
            Ok(v) if v["rejected"] == true => {
                published.error = Some(format!("push rejected (the remote branch has commits this one lacks): {}", v["output"].as_str().unwrap_or("")));
            }
            Ok(v) => published.error = Some(format!("push failed: {}", v["output"].as_str().unwrap_or(""))),
            Err(err) => published.error = Some(format!("push failed: {}", err.message)),
        }
        let worktree_agent = agent.is_some_and(|a| a.worktree.is_some());
        if published.pushed && action == DoneAction::PullRequest && worktree_agent {
            let card = self.edit_board(project_id, |b| Ok((card_mut(b, card_id)?.clone(), false)))?;
            let pr = match call("git.prStatus").await {
                Ok(v) => match serde_json::from_value::<Option<PullRequestInfo>>(v)?.filter(|p| p.state == PrState::Open) {
                    Some(open) => Ok(open),
                    None => {
                        let title = text.title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).unwrap_or_else(|| card.title.clone());
                        let body = text.body.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).unwrap_or_else(|| default_pr_body(&card, &outgoing));
                        let params = json!({ "projectId": project_id, "path": cwd, "title": title, "body": body });
                        match orch.call(None, "git.prCreate".into(), params).await {
                            Ok(v) => Ok(serde_json::from_value::<PullRequestInfo>(v)?),
                            Err(err) => Err(err),
                        }
                    }
                },
                Err(err) => Err(err),
            };
            match pr {
                Ok(pr) => published.pull_request = Some(PullRequestRef { number: pr.number, url: pr.url }),
                Err(err) => published.error = Some(format!("pull request failed: {}", err.message)),
            }
        }
        let shared_extra = (published.pushed && !worktree_agent && outgoing.len() > 1).then_some(outgoing.len());
        let card = self.edit_board(project_id, |b| {
            let c = card_mut(b, card_id)?;
            c.git_error = published.error.clone();
            if published.pull_request.is_some() {
                c.pull_request = published.pull_request.clone();
            }
            if let Some(n) = shared_extra {
                let line = format!("Pushed {n} commits.");
                c.note = Some(match c.note.take() {
                    Some(note) => format!("{note}\n\n{line}"),
                    None => line,
                });
            }
            Ok((c.clone(), true))
        })?;
        Ok((card, published))
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

    fn upload(name: Option<&str>, mime: &str, bytes: &[u8]) -> ImageUpload {
        ImageUpload { name: name.map(Into::into), mime_type: mime.into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }

    #[test]
    fn card_images_are_checked_and_named() {
        let taken = vec!["image.png".to_owned()];
        let out = decode_images(vec![upload(None, "image/png", b"a"), upload(Some("shot"), "image/jpeg", b"b"), upload(Some("x.gif"), "image/gif", b"c")], &taken).unwrap();
        let names: Vec<&str> = out.iter().map(|(a, _)| a.name.as_str()).collect();
        assert_eq!(names, ["2-image.png", "shot.jpg", "x.gif"]);
        assert_eq!((out[1].0.size, out[1].1.as_slice()), (1, b"b".as_slice()));
        // SVG can carry scripts; a disguised name gets the real type's extension.
        assert!(decode_images(vec![upload(Some("a.svg"), "image/svg+xml", b"<svg/>")], &[]).is_err());
        assert_eq!(decode_images(vec![upload(Some("a.svg"), "image/png", b"x")], &[]).unwrap()[0].0.name, "a.svg.png");
        assert!(decode_images(vec![ImageUpload { name: None, mime_type: "image/png".into(), data: "!!".into() }], &[]).is_err());
        assert!(decode_images(vec![upload(None, "image/png", &vec![0; MAX_ATTACHMENT_BYTES + 1])], &[]).is_err());
        let ten: Vec<String> = (0..10).map(|i| format!("{i}.png")).collect();
        assert!(decode_images(vec![upload(None, "image/png", b"a")], &ten).is_err());
    }

    #[test]
    fn prompts_list_card_images() {
        let mut c = card("c1", CardColumn::Todo);
        c.images = vec![Attachment { name: "screen.png".into(), mime: "image/png".into(), size: 1 }];
        let prompt = card_prompt(&c, Path::new("/data/boards"), DoneAction::Commit);
        assert!(prompt.contains("look at them with your read tool):\n- /data/boards/cards/c1/screen.png"), "{prompt}");
        assert!(refine_prompt(&c, Path::new("/data/boards")).contains("/data/boards/cards/c1/screen.png"));
    }

    #[test]
    fn pr_body_takes_the_refinement_goal_and_commits_oldest_first() {
        let mut c = card("c1", CardColumn::Doing);
        c.title = "Push from the chip".into();
        c.body = Some("Long card text".into());
        let commit = |sha: &str, summary: &str| CommitInfo { sha: sha.into(), summary: summary.into(), author: "a".into(), ts: 0 };
        let commits = [commit("bbbbbbbbbb", "second"), commit("aaaaaaaaaa", "first")];
        c.refinement = Some(Refinement {
            text: "# Title\n\n## Ziel\nPush and PRs from the git chip.\n\nSecond line.\n\n## Verhalten\n- more".into(),
            attachments: vec![],
            agent_id: None,
            agent_title: None,
            at: 0,
        });
        assert_eq!(
            default_pr_body(&c, &commits),
            "Kanban: Push from the chip (c1)\n\nPush and PRs from the git chip.\n\nSecond line.\n\nCommits:\n- aaaaaaa first\n- bbbbbbb second"
        );
        // Without a goal section the card text stands in; without commits there is no list.
        c.refinement.as_mut().unwrap().text = "## Approach\nx".into();
        assert_eq!(default_pr_body(&c, &[]), "Kanban: Push from the chip (c1)\n\nLong card text");
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
        let mut b = Board { cards: vec![card("a", CardColumn::Todo), card("x", CardColumn::Done), card("b", CardColumn::Todo), card("c", CardColumn::Todo)], ..Board::default() };
        move_card(&mut b, "c", CardColumn::Todo, 0).unwrap();
        assert_eq!(columns(&b, CardColumn::Todo), ["c", "a", "b"]);
        move_card(&mut b, "a", CardColumn::Doing, 0).unwrap();
        assert_eq!((columns(&b, CardColumn::Todo), columns(&b, CardColumn::Doing)), (vec!["c", "b"], vec!["a"]));
        move_card(&mut b, "b", CardColumn::Done, usize::MAX).unwrap();
        assert_eq!(columns(&b, CardColumn::Done), ["x", "b"]);
        assert!(move_card(&mut b, "nope", CardColumn::Done, 0).is_err());
    }

    #[test]
    fn done_at_follows_the_done_column() {
        let mut b = Board { cards: vec![card("a", CardColumn::Doing), card("x", CardColumn::Done)], ..Board::default() };
        let done_at = move_card(&mut b, "a", CardColumn::Done, 0).unwrap().done_at;
        assert!(done_at.is_some());
        assert_eq!(move_card(&mut b, "a", CardColumn::Done, usize::MAX).unwrap().done_at, done_at, "reordering within done keeps it");
        assert_eq!(move_card(&mut b, "a", CardColumn::Todo, 0).unwrap().done_at, None);
    }

    #[test]
    fn board_changes_become_card_events() {
        let mut b = Board { cards: vec![card("a", CardColumn::Inbox), card("b", CardColumn::Todo), card("gone", CardColumn::Todo)], ..Board::default() };
        let before = card_marks(&b);
        move_card(&mut b, "a", CardColumn::Todo, 0).unwrap();
        b.cards.retain(|c| c.id != "gone");
        let c = card_mut(&mut b, "b").unwrap();
        c.agent_id = Some("worker".into());
        c.refinement = Some(Refinement { text: "r".into(), attachments: vec![], agent_id: Some("refiner".into()), agent_title: None, at: 5 });
        let mut added = card("n", CardColumn::Inbox);
        added.added_by = Some(AgentRef { id: "suggester".into(), title: "S".into() });
        b.cards.push(added);
        let events: Vec<(String, String, Option<String>, Value)> =
            card_events(&before, &b).into_iter().map(|e| (e.kind, e.card_id.unwrap(), e.agent_id, Value::Object(e.meta))).collect();
        assert_eq!(
            events,
            vec![
                ("card.move".into(), "a".into(), None, json!({ "from": "inbox", "to": "todo" })),
                ("card.refined".into(), "b".into(), Some("refiner".into()), json!({})),
                ("card.add".into(), "n".into(), Some("suggester".into()), json!({ "column": "inbox", "byAgent": true })),
                ("card.delete".into(), "gone".into(), None, json!({})),
            ]
        );
        assert!(card_events(&card_marks(&b), &b).is_empty(), "an unchanged board reports nothing");
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
            write_json_atomic(&agents_dir.join(agent).join(LEGACY_FILE), &Board { cards, ..Board::default() }).unwrap();
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
