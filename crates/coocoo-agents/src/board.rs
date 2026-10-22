//! Per-agent Kanban board (`agents.board*`, `agents.card*`): inbox → todo → doing → done.
//!
//! The user fills `todo` (and pulls cards over from the inbox, where external sources drop new
//! entries). Starting a card moves it to `doing` and prompts the agent with it; the agent closes
//! it with the `kanban_done` host tool, which commits the work when the directory is a git
//! repository. Stored in `<agent dir>/board.json`.

use std::sync::Arc;
use std::time::Duration;

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::{new_id, now_ms, read_json, truncate, write_json_atomic};
use coocoo_proto::{ApiError, ApiResult, Board, BoardEvent, BoardSource, Card, CardColumn, CommitInfo, events};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Agents;
use crate::state::Agent;

const FILE: &str = "board.json";
const MAX_TITLE: usize = 300;
const MAX_BODY: usize = 8 * 1024;
/// Remembered entry keys per source.
const MAX_SEEN: usize = 1000;
/// A newly added source imports only its newest entries; older ones are marked seen.
const FIRST_IMPORT: usize = 20;
const DEFAULT_INTERVAL_MINS: u32 = 15;
const MIN_INTERVAL_MINS: u32 = 5;
/// How often due sources are looked for.
const POLL_TICK: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FEED_BYTES: &str = "5000000";

#[derive(Deserialize)]
struct IdParams {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardAddParams {
    id: String,
    title: String,
    body: Option<String>,
    column: Option<CardColumn>,
    /// Insert at the top of the column instead of the bottom.
    #[serde(default)]
    top: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardUpdateParams {
    id: String,
    card_id: String,
    title: Option<String>,
    /// `""` clears it.
    body: Option<String>,
    column: Option<CardColumn>,
    /// Position in the (target) column, 0 = top; past the end = bottom.
    index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardParams {
    id: String,
    card_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardDoneParams {
    id: String,
    card_id: String,
    commit_message: Option<String>,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceAddParams {
    id: String,
    kind: String,
    target: String,
    interval_mins: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceParams {
    id: String,
    source_id: String,
}

/// One entry from a source.
#[derive(Debug, PartialEq)]
struct Entry {
    /// Dedup key: feed guid/link, or the command's output line.
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

/// Prompt for a started card.
fn card_prompt(card: &Card) -> String {
    let mut text = format!("Work on this card from your Kanban board (card id `{}`):\n\n**{}**", card.id, card.title);
    if let Some(body) = &card.body {
        text.push_str("\n\n");
        text.push_str(body);
    }
    if let Some(url) = &card.url {
        text.push_str(&format!("\n\nSource: {url}"));
    }
    text.push_str(
        "\n\nWhen it is finished, call `kanban_done` with this card id and a commit message describing the change \
         (it commits your changes if this is a git repository and moves the card to Done). If you cannot finish it, \
         say why and leave the card in Doing.",
    );
    text
}

impl Agents {
    pub(crate) async fn board_method(self: &Arc<Self>, method: &str, p: Value) -> ApiResult<Value> {
        match method {
            "agents.board" => {
                let IdParams { id } = params(p)?;
                ok(self.edit_board(&*self.get(&id)?, |b| Ok((b.clone(), false)))?)
            }
            "agents.cardAdd" => {
                let p: CardAddParams = params(p)?;
                let (title, body) = (clean_title(&p.title)?, clean_body(p.body));
                let column = p.column.unwrap_or(CardColumn::Todo);
                ok(self.edit_board(&*self.get(&p.id)?, |b| {
                    let now = now_ms();
                    let card = Card {
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
                    };
                    let at = insert_at(&b.cards, column, if p.top { 0 } else { usize::MAX });
                    b.cards.insert(at, card.clone());
                    Ok((card, true))
                })?)
            }
            "agents.cardUpdate" => {
                let p: CardUpdateParams = params(p)?;
                let title = p.title.as_deref().map(clean_title).transpose()?;
                ok(self.edit_board(&*self.get(&p.id)?, |b| {
                    let card = card_mut(b, &p.card_id)?;
                    if let Some(title) = title {
                        card.title = title;
                    }
                    if let Some(body) = p.body {
                        card.body = clean_body(Some(body));
                    }
                    card.updated_at = now_ms();
                    let column = card.column;
                    let card = match (p.column, p.index) {
                        (None, None) => card.clone(),
                        (to, index) => move_card(b, &p.card_id, to.unwrap_or(column), index.unwrap_or(usize::MAX))?,
                    };
                    Ok((card, true))
                })?)
            }
            "agents.cardDelete" => {
                let p: CardParams = params(p)?;
                self.edit_board(&*self.get(&p.id)?, |b| {
                    let before = b.cards.len();
                    b.cards.retain(|c| c.id != p.card_id);
                    if b.cards.len() == before {
                        return Err(ApiError::not_found(format_args!("card {}", p.card_id)));
                    }
                    Ok(((), true))
                })?;
                Ok(json!({}))
            }
            "agents.cardStart" => {
                let p: CardParams = params(p)?;
                let agent = self.get(&p.id)?;
                let card = self.edit_board(&agent, |b| Ok((card_mut(b, &p.card_id)?.clone(), false)))?;
                if card.column == CardColumn::Doing || card.column == CardColumn::Done {
                    let column = if card.column == CardColumn::Doing { "Doing" } else { "Done" };
                    return Err(ApiError::conflict(format!("card “{}” is already in {column}", card.title)));
                }
                self.prompt_text(&p.id, card_prompt(&card)).await?;
                ok(self.edit_board(&agent, |b| Ok((move_card(b, &p.card_id, CardColumn::Doing, 0)?, true)))?)
            }
            "agents.cardDone" => self.card_done(params(p)?).await,
            "agents.boardSourceAdd" => {
                let p: SourceAddParams = params(p)?;
                let target = p.target.trim().to_owned();
                match p.kind.as_str() {
                    "feed" if target.starts_with("http://") || target.starts_with("https://") => {}
                    "feed" => return Err(ApiError::invalid("a feed needs an http(s) URL")),
                    "command" if !target.is_empty() => {}
                    "command" => return Err(ApiError::invalid("the command must not be empty")),
                    other => return Err(ApiError::invalid(format!("unknown source kind `{other}` (feed, command)"))),
                }
                let agent = self.get(&p.id)?;
                let source = BoardSource {
                    id: new_id(),
                    kind: p.kind,
                    target,
                    interval_mins: p.interval_mins.unwrap_or(DEFAULT_INTERVAL_MINS).max(MIN_INTERVAL_MINS),
                    checked_at: None,
                    error: None,
                    seen: Vec::new(),
                };
                let source_id = source.id.clone();
                self.edit_board(&agent, |b| {
                    b.sources.push(source);
                    Ok(((), true))
                })?;
                self.refresh_source(&agent, &source_id).await;
                ok(self.edit_board(&agent, |b| Ok((b.clone(), false)))?)
            }
            "agents.boardSourceRemove" => {
                let p: SourceParams = params(p)?;
                ok(self.edit_board(&*self.get(&p.id)?, |b| {
                    b.sources.retain(|s| s.id != p.source_id);
                    Ok((b.clone(), true))
                })?)
            }
            "agents.boardSourceRefresh" => {
                let p: SourceParams = params(p)?;
                let agent = self.get(&p.id)?;
                self.refresh_source(&agent, &p.source_id).await;
                ok(self.edit_board(&agent, |b| Ok((b.clone(), false)))?)
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    /// Run `f` on the agent's board (loaded on first use); persist and announce it when `f`
    /// reports a change.
    fn edit_board<R>(&self, agent: &Agent, f: impl FnOnce(&mut Board) -> ApiResult<(R, bool)>) -> ApiResult<R> {
        let mut slot = agent.board.lock();
        let board = slot.get_or_insert_with(|| match read_json::<Board>(&agent.dir.join(FILE)) {
            Ok(board) => board.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(agent = %agent.id, "unreadable {FILE}: {err}");
                Board::default()
            }
        });
        let (result, changed) = f(board)?;
        if changed {
            write_json_atomic(&agent.dir.join(FILE), &*board)?;
            self.emitter.emit(events::AGENT_BOARD, &BoardEvent { agent_id: agent.id.clone(), board: board.clone() });
        }
        Ok(result)
    }

    async fn card_done(self: &Arc<Self>, p: CardDoneParams) -> ApiResult<Value> {
        let agent = self.get(&p.id)?;
        let card = self.edit_board(&agent, |b| Ok((card_mut(b, &p.card_id)?.clone(), false)))?;
        let cwd = agent.summary().cwd;
        let message = p.commit_message.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()).unwrap_or_else(|| card.title.clone());
        let orchestrator = self.orchestrator.get().cloned().ok_or_else(|| ApiError::unavailable("daemon is starting"))?;
        let (commit, skipped) = match orchestrator.call(None, "git.commit".into(), json!({ "cwd": cwd, "message": message })).await {
            Ok(value) => (Some(serde_json::from_value::<CommitInfo>(value)?), None),
            Err(err) if err.code == "invalid_params" && err.message.contains("not a git repository") => (None, Some("not a git repository")),
            Err(err) if err.code == "conflict" => (None, Some("nothing to commit")),
            // Leave the card in place so the agent can fix the problem (e.g. a failing hook).
            Err(err) => return Err(ApiError::new(err.code, format!("commit failed: {}", err.message))),
        };
        let note = clean_body(p.note);
        let card = self.edit_board(&agent, |b| {
            let card = card_mut(b, &p.card_id)?;
            card.note = note;
            card.commit = commit.clone();
            move_card(b, &p.card_id, CardColumn::Done, 0).map(|c| (c, true))
        })?;
        Ok(json!({ "card": card, "commit": commit, "commitSkipped": skipped }))
    }

    /// Fetch one source and drop its new entries at the top of the inbox.
    async fn refresh_source(&self, agent: &Agent, source_id: &str) {
        let source = self.edit_board(agent, |b| Ok((b.sources.iter().find(|s| s.id == source_id).cloned(), false)));
        let Ok(Some(source)) = source else { return };
        let cwd = agent.summary().cwd;
        let fetched = match source.kind.as_str() {
            "feed" => fetch_feed(&source.target).await,
            _ => run_command(&source.target, &cwd).await,
        };
        let _ = self.edit_board(agent, |b| {
            let Some(source) = b.sources.iter_mut().find(|s| s.id == source_id) else { return Ok(((), false)) };
            source.checked_at = Some(now_ms());
            let entries = match fetched {
                Ok(entries) => {
                    source.error = None;
                    entries
                }
                Err(err) => {
                    source.error = Some(err);
                    return Ok(((), true));
                }
            };
            let first = source.seen.is_empty();
            let fresh: Vec<Entry> = entries.into_iter().filter(|e| !source.seen.contains(&e.key)).collect();
            source.seen.extend(fresh.iter().map(|e| e.key.clone()));
            let excess = source.seen.len().saturating_sub(MAX_SEEN);
            source.seen.drain(..excess);
            let take = if first { FIRST_IMPORT } else { usize::MAX };
            let now = now_ms();
            let source_id = source.id.clone();
            for (i, entry) in fresh.into_iter().take(take).enumerate() {
                let Ok(title) = clean_title(&entry.title) else { continue };
                let card = Card {
                    id: new_id(),
                    title,
                    body: clean_body(entry.body),
                    column: CardColumn::Inbox,
                    url: entry.url,
                    source: Some(source_id.clone()),
                    created_at: now,
                    updated_at: now,
                    note: None,
                    commit: None,
                };
                let at = insert_at(&b.cards, CardColumn::Inbox, i);
                b.cards.insert(at, card);
            }
            Ok(((), true))
        });
    }

    /// Check due sources every [`POLL_TICK`] while the daemon runs.
    pub(crate) fn start_board_poller(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        self.handle().spawn(async move {
            let mut tick = tokio::time::interval(POLL_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(agents) = me.upgrade() else { return };
                let all: Vec<Arc<Agent>> = agents.agents.read().values().cloned().collect();
                for agent in all {
                    if agent.state.lock().deleted || agent.summary().status == coocoo_proto::AgentStatus::Stopped {
                        continue;
                    }
                    let now = now_ms();
                    let due: Vec<String> = agents
                        .edit_board(&agent, |b| {
                            let due = b.sources.iter().filter(|s| s.checked_at.is_none_or(|t| now - t >= s.interval_mins as i64 * 60_000));
                            Ok((due.map(|s| s.id.clone()).collect(), false))
                        })
                        .unwrap_or_default();
                    for id in due {
                        agents.refresh_source(&agent, &id).await;
                    }
                }
            }
        });
    }
}

async fn output(mut command: tokio::process::Command, what: &str) -> Result<String, String> {
    command.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(FETCH_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{what} timed out"))?
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let reason = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("failed");
        return Err(if reason.starts_with(what) { reason.to_owned() } else { format!("{what}: {reason}") });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Fetched with the `curl` CLI (no HTTP/TLS stack in the daemon).
async fn fetch_feed(url: &str) -> Result<Vec<Entry>, String> {
    let mut command = tokio::process::Command::new("curl");
    command.args(["-fsSL", "--max-time", "25", "--max-filesize", MAX_FEED_BYTES, "-A", "coocoo", "--", url]);
    parse_feed(&output(command, "curl").await?)
}

/// One card per non-empty output line, keyed by the line.
async fn run_command(command_line: &str, cwd: &str) -> Result<Vec<Entry>, String> {
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", command_line]).current_dir(cwd);
    let out = output(command, "command").await?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| Entry { key: line.to_owned(), title: line.to_owned(), body: None, url: None })
        .collect())
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
        Card { id: id.into(), title: id.into(), body: None, column, url: None, source: None, created_at: 0, updated_at: 0, note: None, commit: None }
    }

    fn columns(board: &Board, column: CardColumn) -> Vec<&str> {
        board.cards.iter().filter(|c| c.column == column).map(|c| c.id.as_str()).collect()
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
