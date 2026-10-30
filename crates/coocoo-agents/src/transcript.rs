//! `agents.subagentTimeline`: a subagent's transcript, read from its omp session file, and
//! [`ImageScan`], which picks up the images a running subagent's tool calls return.
//!
//! omp keeps each subagent's session next to its parent's (`<parent>.jsonl` →
//! `<parent>/<Subagent>.jsonl`, nested ones deeper). Reading the file works whether or not the
//! parent's process still runs, and only files inside the agent's own session directory are
//! served.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use coocoo_proto::util::truncate;
use coocoo_proto::{ApiError, ApiResult, ItemKind, TimelineItem, TimelinePage, ToolCall, ToolStatus};
use omp_rpc::{AgentMessage, AssistantContent};
use serde::Deserialize;
use serde_json::Value;

use crate::mapper::{DecodedImage, MAX_ARGS, MAX_OUTPUT, content_text, tool_images, tool_output};

/// Newest items returned; long subagent runs are cut at the front.
const LIMIT: usize = 500;

/// An image a subagent's tool call returned.
pub(crate) struct FoundImage {
    /// The call's id in the subagent's session.
    pub tool_call: String,
    /// The call's intent, else the tool's name.
    pub title: String,
    pub image: DecodedImage,
}

/// Reads a growing subagent session file from where the last scan stopped and returns the
/// images of tool results added since. A line still being written is left for the next scan.
#[derive(Default)]
pub(crate) struct ImageScan {
    /// Bytes of complete lines already read.
    offset: u64,
    /// Calls seen without a result yet: id → title.
    calls: HashMap<String, String>,
}

impl ImageScan {
    pub(crate) fn scan(&mut self, path: &Path) -> std::io::Result<Vec<FoundImage>> {
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        if len <= self.offset {
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        file.take(len - self.offset).read_to_end(&mut buf)?;
        let Some(end) = buf.iter().rposition(|&b| b == b'\n') else { return Ok(Vec::new()) };
        self.offset += end as u64 + 1;
        let blobs = blob_dir(path);
        let mut found = Vec::new();
        for line in buf[..end].split(|&b| b == b'\n') {
            let Ok(entry) = serde_json::from_slice::<Value>(line) else { continue };
            if entry.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            let message = &entry["message"];
            match message.get("role").and_then(Value::as_str) {
                Some("assistant") => {
                    let parts = message.get("content").and_then(Value::as_array).into_iter().flatten();
                    for call in parts.filter(|p| p.get("type").and_then(Value::as_str) == Some("toolCall")) {
                        let Some(id) = call.get("id").and_then(Value::as_str) else { continue };
                        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
                        let title = text(call.get("intent"))
                            .or_else(|| text(call.get("arguments").and_then(|a| a.get("i"))))
                            .or_else(|| text(call.get("name")))
                            .unwrap_or_default();
                        self.calls.insert(id.to_owned(), title);
                    }
                }
                Some("toolResult") => {
                    let Some(id) = message.get("toolCallId").and_then(Value::as_str) else { continue };
                    let title = self.calls.remove(id);
                    let images = result_images(message, blobs.as_deref());
                    if images.is_empty() {
                        continue;
                    }
                    let title = title.or_else(|| message.get("toolName").and_then(Value::as_str).map(str::to_owned)).unwrap_or_default();
                    found.extend(images.into_iter().map(|image| FoundImage { tool_call: id.to_owned(), title: title.clone(), image }));
                }
                _ => {}
            }
        }
        Ok(found)
    }
}

/// omp's blob store (`<agent dir>/blobs`), found from a session file below `<agent dir>/sessions`.
fn blob_dir(session: &Path) -> Option<PathBuf> {
    let sessions = session.ancestors().find(|p| p.file_name().is_some_and(|n| n == "sessions"))?;
    Some(sessions.parent()?.join("blobs"))
}

/// Images of a tool result in a session file: inline base64 data, or (as omp stores them) a
/// `blob:sha256:<hex>` reference into its blob store.
fn result_images(message: &Value, blobs: Option<&Path>) -> Vec<DecodedImage> {
    let mut out = tool_images(message);
    let parts = message.get("content").and_then(Value::as_array).into_iter().flatten();
    for part in parts.filter(|p| p.get("type").and_then(Value::as_str) == Some("image")) {
        let Some(hash) = part.get("data").and_then(Value::as_str).and_then(|d| d.strip_prefix("blob:sha256:")) else { continue };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let Some(mime) = part.get("mimeType").and_then(Value::as_str) else { continue };
        let (Some(ext), Some(blobs)) = (crate::image::ext(mime), blobs) else { continue };
        if let Ok(bytes) = std::fs::read(blobs.join(hash)) {
            out.push((mime.to_owned(), ext, bytes));
        }
    }
    out
}

/// `requested` if it is a session file below `parent`'s session directory.
pub(crate) fn resolve(parent: &Path, requested: &Path) -> ApiResult<PathBuf> {
    let denied = || ApiError::new("forbidden", format!("{} is not a subagent session of this agent", requested.display()));
    let root = parent.with_extension("");
    let root = std::fs::canonicalize(&root).map_err(|_| denied())?;
    let file = std::fs::canonicalize(requested).map_err(|_| ApiError::not_found(format_args!("session {}", requested.display())))?;
    if file.starts_with(&root) && file.extension().is_some_and(|e| e == "jsonl") && file.is_file() {
        Ok(file)
    } else {
        Err(denied())
    }
}

/// The session's messages as timeline items. Unless `live`, tool calls without a result are
/// shown as interrupted.
pub(crate) fn read(path: &Path, live: bool) -> ApiResult<TimelinePage> {
    let file = std::fs::File::open(path)?;
    let mut items: Vec<TimelineItem> = Vec::new();
    // Tool call id → index of its item.
    let mut calls: HashMap<String, usize> = HashMap::new();
    for line in BufReader::with_capacity(1 << 16, file).split(b'\n') {
        let line = line?;
        // A torn final line (still being written) is skipped.
        let Ok(mut entry) = serde_json::from_slice::<Value>(&line) else { continue };
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let raw = entry["message"].take();
        let ts = raw.get("timestamp").and_then(Value::as_f64).map_or(0, |t| t as i64);
        let Ok(message) = AgentMessage::deserialize(&raw) else { continue };
        let mut push = |kind: ItemKind, text: Option<String>, tool: Option<ToolCall>, partial: bool| {
            let seq = items.len() as u64 + 1;
            items.push(TimelineItem { seq, id: seq.to_string(), ts, kind, text, tool, partial, images: Vec::new() });
            items.len() - 1
        };
        match message {
            AgentMessage::User(user) if user.synthetic != Some(true) => {
                let text = user.content.map(content_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    push(ItemKind::User, Some(text), None, false);
                }
            }
            AgentMessage::Assistant(assistant) => {
                for content in assistant.content.unwrap_or_default() {
                    match content {
                        AssistantContent::Text(t) => {
                            if let Some(text) = t.text.filter(|t| !t.trim().is_empty()) {
                                push(ItemKind::Assistant, Some(text), None, false);
                            }
                        }
                        AssistantContent::Thinking(t) => {
                            if let Some(text) = t.thinking.filter(|t| !t.trim().is_empty()) {
                                push(ItemKind::Thinking, Some(text), None, false);
                            }
                        }
                        AssistantContent::ToolCall(call) => {
                            let args = call.arguments.unwrap_or_default();
                            let intent = call.intent.or_else(|| args.get("i").and_then(Value::as_str).map(str::to_owned));
                            let tool = ToolCall {
                                name: call.name.unwrap_or_default(),
                                intent,
                                args: truncate(&Value::Object(args).to_string(), MAX_ARGS),
                                status: ToolStatus::Running,
                                output: None,
                                duration_ms: None,
                                subagents: Vec::new(),
                            };
                            let at = push(ItemKind::Tool, None, Some(tool), true);
                            if let Some(id) = call.id {
                                calls.insert(id, at);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(error) = assistant.error_message {
                    push(ItemKind::Error, Some(error), None, false);
                }
            }
            AgentMessage::ToolResult(result) => {
                let Some(&at) = result.tool_call_id.as_ref().and_then(|id| calls.get(id)) else { continue };
                let item = &mut items[at];
                item.partial = false;
                if let Some(tool) = &mut item.tool {
                    tool.status = if result.is_error == Some(true) { ToolStatus::Error } else { ToolStatus::Ok };
                    // No duration: the session only has the assistant message's start time, which
                    // parallel calls share.
                    tool.output = Some(truncate(&tool_output(&raw), MAX_OUTPUT));
                }
            }
            AgentMessage::CompactionSummary(_) => {
                push(ItemKind::Notice, Some("Context compacted".to_owned()), None, false);
            }
            _ => {}
        }
    }
    // Tool items are keyed by their call id, as `TimelineImage.toolCall` of a subagent's images is.
    for (id, at) in calls {
        items[at].id = id;
    }
    if !live {
        for item in &mut items {
            crate::timeline::settle(item);
        }
    }
    let has_more = items.len() > LIMIT;
    if has_more {
        items.drain(..items.len() - LIMIT);
    }
    Ok(TimelinePage { items, has_more })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coocoo-transcript-{}", coocoo_proto::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Recorded subagent session lines (trimmed), plus a torn final line.
    const SESSION: &str = r#"{"type":"session","version":3,"id":"s1"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Count the files"}],"timestamp":1000}}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"I should list them"},{"type":"toolCall","id":"t1","name":"bash","arguments":{"command":"ls | wc -l","i":"Counting files"}}],"timestamp":2000}}
{"type":"custom","data":{}}
{"type":"message","message":{"role":"toolResult","toolCallId":"t1","toolName":"bash","content":[{"type":"text","text":"42"}],"isError":false,"timestamp":2500}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"42 files."},{"type":"toolCall","id":"t2","name":"read","arguments":{"path":"x"}}],"timestamp":3000}}
{"type":"message","message":{"role":"assist"#;

    #[test]
    fn maps_messages_and_pairs_tool_results() {
        let dir = temp_dir();
        let path = dir.join("Sub.jsonl");
        std::fs::write(&path, SESSION).unwrap();
        let page = read(&path, true).unwrap();
        let kinds: Vec<ItemKind> = page.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [ItemKind::User, ItemKind::Thinking, ItemKind::Tool, ItemKind::Assistant, ItemKind::Tool]);
        let bash = page.items[2].tool.as_ref().unwrap();
        assert_eq!((bash.intent.as_deref(), bash.status, bash.output.as_deref()), (Some("Counting files"), ToolStatus::Ok, Some("42")));
        assert_eq!(page.items[2].id, "t1", "tool items are keyed by their call id");
        // Still running while live; interrupted once the subagent is gone.
        assert_eq!(page.items[4].tool.as_ref().unwrap().status, ToolStatus::Running);
        let done = read(&path, false).unwrap();
        assert_eq!(done.items[4].tool.as_ref().unwrap().status, ToolStatus::Error);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_files_below_the_parent_session_dir() {
        let dir = temp_dir();
        let parent = dir.join("2026-parent.jsonl");
        std::fs::write(&parent, "").unwrap();
        std::fs::create_dir_all(dir.join("2026-parent/Nested")).unwrap();
        std::fs::write(dir.join("2026-parent/Sub.jsonl"), "").unwrap();
        std::fs::write(dir.join("2026-parent/Nested/Deep.jsonl"), "").unwrap();
        std::fs::write(dir.join("other.jsonl"), "").unwrap();
        std::fs::write(dir.join("2026-parent/notes.txt"), "").unwrap();
        assert!(resolve(&parent, &dir.join("2026-parent/Sub.jsonl")).is_ok());
        assert!(resolve(&parent, &dir.join("2026-parent/Nested/Deep.jsonl")).is_ok());
        for bad in ["other.jsonl", "2026-parent/../other.jsonl", "2026-parent/notes.txt", "2026-parent.jsonl"] {
            assert!(resolve(&parent, &dir.join(bad)).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// Images of tool results, inline or in omp's blob store, each read once as the file grows;
    /// a line still being written waits for the next scan.
    #[test]
    fn scan_picks_up_new_result_images() {
        use base64::Engine as _;
        let agent_dir = temp_dir();
        let session_dir = agent_dir.join("sessions/-proj/2026-parent");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(agent_dir.join("blobs")).unwrap();
        let png = crate::image::tests::png(800, 600);
        let hash = "ab".repeat(32);
        std::fs::write(agent_dir.join("blobs").join(&hash), &png).unwrap();
        let inline = base64::engine::general_purpose::STANDARD.encode(&png);
        let call = |id: &str, intent: &str| {
            format!(r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","id":"{id}","name":"browser","arguments":{{"i":"{intent}"}}}}]}}}}"#)
        };
        let result = |id: &str, data: &str| {
            format!(r#"{{"type":"message","message":{{"role":"toolResult","toolCallId":"{id}","toolName":"browser","content":[{{"type":"text","text":"ok"}},{{"type":"image","data":"{data}","mimeType":"image/png"}}]}}}}"#)
        };
        let path = session_dir.join("Sub.jsonl");
        let first = format!("{}\n{}\n{}\n", call("c1", "Login page"), result("c1", &format!("blob:sha256:{hash}")), call("c2", "Dashboard"));
        let torn = result("c2", &inline);
        std::fs::write(&path, format!("{first}{}", &torn[..40])).unwrap();
        let mut scan = ImageScan::default();
        let found = scan.scan(&path).unwrap();
        assert_eq!(found.iter().map(|f| (f.tool_call.as_str(), f.title.as_str(), f.image.2.len())).collect::<Vec<_>>(), [("c1", "Login page", png.len())]);
        assert!(scan.scan(&path).unwrap().is_empty(), "nothing new");
        let bad_blob = result("c3", "blob:sha256:../../etc/passwd");
        std::fs::write(&path, format!("{first}{torn}\n{bad_blob}\n")).unwrap();
        let found = scan.scan(&path).unwrap();
        assert_eq!(found.iter().map(|f| (f.tool_call.as_str(), f.title.as_str())).collect::<Vec<_>>(), [("c2", "Dashboard")]);
        assert_eq!(found[0].image.0, "image/png");
        std::fs::remove_dir_all(agent_dir).ok();
    }
}
