//! `agents.subagentTimeline`: a subagent's transcript, read from its omp session file.
//!
//! omp keeps each subagent's session next to its parent's (`<parent>.jsonl` →
//! `<parent>/<Subagent>.jsonl`, nested ones deeper). Reading the file works whether or not the
//! parent's process still runs, and only files inside the agent's own session directory are
//! served.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use coocoo_proto::util::truncate;
use coocoo_proto::{ApiError, ApiResult, ItemKind, TimelineItem, TimelinePage, ToolCall, ToolStatus};
use omp_rpc::{AgentMessage, AssistantContent};
use serde::Deserialize;
use serde_json::Value;

use crate::mapper::{MAX_ARGS, MAX_OUTPUT, content_text, tool_output};

/// Newest items returned; long subagent runs are cut at the front.
const LIMIT: usize = 500;

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
            items.push(TimelineItem { seq, id: seq.to_string(), ts, kind, text, tool, partial });
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
}
