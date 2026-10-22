//! Translates omp RPC notifications into timeline items and agent state effects.
//!
//! Pure with respect to I/O: every output goes through a [`Sink`], so the mapping is unit
//! tested with recorded frames. Streaming text deltas are coalesced into at most one
//! `agent.delta` per [`DELTA_WINDOW`]; subagent progress into one unpersisted tool item update
//! per [`PROGRESS_WINDOW`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use coocoo_proto::util::{now_ms, truncate};
use coocoo_proto::{ItemKind, Subagent, TimelineItem, ToolCall, ToolStatus};
use omp_rpc::{
    AgentMessage, AssistantContent, AssistantMessageEvent, ExtensionUiRequest, MessageContent, NotifyType, PromptStatus,
    RpcAgentEvent, RpcNotification, StopReason, SubagentLifecyclePayload, SubagentLifecycleStatus, SubagentProgressPayload,
    UserContent,
};
use serde_json::{Map, Value};

/// Coalescing window for streaming text.
pub(crate) const DELTA_WINDOW: Duration = Duration::from_millis(50);
/// Subagents report progress on every tool step; clients see at most one update per window.
pub(crate) const PROGRESS_WINDOW: Duration = Duration::from_secs(1);
pub(crate) const MAX_ARGS: usize = 4 * 1024;
pub(crate) const MAX_OUTPUT: usize = 8 * 1024;
const MAX_ACTIVITY: usize = 160;
/// Finished tool calls kept for subagent frames that arrive after the call returned.
const RECENT_TOOLS: usize = 16;

fn has_running(item: &TimelineItem) -> bool {
    item.tool.as_ref().is_some_and(|t| t.subagents.iter().any(|s| s.status == "running"))
}

/// A state change derived from an omp frame.
#[derive(Debug, Clone)]
pub(crate) enum Effect {
    /// Insert or replace a timeline item (persisted + `agent.timeline`).
    Upsert(TimelineItem),
    /// Replace a tool item's live subagent progress (`agent.timeline`, memory only; the next
    /// `Upsert` of the item persists the state at that point).
    Progress(TimelineItem),
    /// Streaming text for a partial item (`agent.delta`, memory only).
    Delta { item_id: String, text: String },
    /// A run started (`agent_start`).
    AgentStart,
    /// One prompt completed (`prompt_result`).
    PromptResult { status: PromptStatus, settled: bool, error: Option<String> },
    /// Nothing can wake the session any more (`session_settled`).
    Settled,
    /// A dialog the user has to answer.
    Request { id: String, method: String, payload: Value },
    /// A dialog was cancelled by omp.
    Resolve { id: String },
    SubagentStarted(String),
    SubagentEnded(String),
    Queue { steering: Vec<String>, follow_up: Vec<String> },
    Thinking(String),
    /// Full visible text of the latest assistant message.
    Output(String),
    /// Session state changed in a way only `get_state` reports (todos, model).
    Refresh,
    TodosCleared,
}

/// Tests compare effects structurally; `TimelineItem` itself has no `PartialEq`.
#[cfg(test)]
impl PartialEq for Effect {
    fn eq(&self, other: &Self) -> bool {
        format!("{self:?}") == format!("{other:?}")
    }
}

/// Receives the mapper's output.
pub(crate) trait Sink {
    /// Next timeline `seq` for this agent.
    fn seq(&mut self) -> u64;
    fn effect(&mut self, effect: Effect);
}

struct Block {
    item: TimelineItem,
    /// The item was upserted at least once (it is created on its first non-empty text).
    emitted: bool,
    /// The block ended (`text_end`/`thinking_end`); `message_end` only re-sends changed text.
    done: bool,
}

struct RunningTool {
    item: TimelineItem,
    started: Instant,
}

#[derive(Default)]
pub(crate) struct Mapper {
    /// Streaming assistant/thinking blocks by `(messageId, contentIndex)`.
    blocks: HashMap<(String, i64), Block>,
    tools: HashMap<String, RunningTool>,
    /// Recently finished tool calls, oldest first: async `task` calls return before their
    /// subagents report, and detached subagents outlive the call. Calls with running subagents
    /// are never evicted.
    finished: Vec<(String, TimelineItem)>,
    pending_delta: Option<(String, String)>,
    delta_since: Option<Instant>,
    /// Tool calls with subagent progress not yet sent, and when the first of it arrived.
    progress_dirty: Vec<String>,
    progress_since: Option<Instant>,
    compaction: Option<TimelineItem>,
    retry: Option<TimelineItem>,
    /// An error item was already shown for the current run.
    error_shown: bool,
}

fn new_item(sink: &mut dyn Sink, kind: ItemKind, text: Option<String>, partial: bool) -> TimelineItem {
    let seq = sink.seq();
    TimelineItem { seq, id: seq.to_string(), ts: now_ms(), kind, text, tool: None, partial }
}

fn notice(sink: &mut dyn Sink, kind: ItemKind, text: String) {
    let item = new_item(sink, kind, Some(text), false);
    sink.effect(Effect::Upsert(item));
}

/// omp notices/notifications: `info` is transient status chatter (e.g. tool mounts at startup)
/// and stays out of the timeline; warnings become notices, errors error items.
fn leveled_notice(sink: &mut dyn Sink, level: NotifyType, text: String) {
    match level {
        NotifyType::Info => tracing::debug!(message = %text, "omp info notice"),
        NotifyType::Warning => notice(sink, ItemKind::Notice, text),
        NotifyType::Error => notice(sink, ItemKind::Error, text),
    }
}

impl Mapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// When buffered streaming text or subagent progress must be flushed.
    pub fn deadline(&self) -> Option<Instant> {
        let delta = self.delta_since.map(|since| since + DELTA_WINDOW);
        let progress = self.progress_since.map(|since| since + PROGRESS_WINDOW);
        match (delta, progress) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Emit buffered streaming text, and subagent progress once its window has passed.
    pub fn flush(&mut self, sink: &mut dyn Sink) {
        self.delta_since = None;
        if let Some((item_id, text)) = self.pending_delta.take()
            && !text.is_empty()
        {
            sink.effect(Effect::Delta { item_id, text });
        }
        if self.progress_since.is_some_and(|since| since.elapsed() >= PROGRESS_WINDOW) {
            self.progress_since = None;
            for call in std::mem::take(&mut self.progress_dirty) {
                if let Some(item) = self.owner(&call) {
                    sink.effect(Effect::Progress(item.clone()));
                }
            }
        }
    }

    /// The process is gone: finish every streaming item.
    pub fn finish(&mut self, sink: &mut dyn Sink) {
        self.flush(sink);
        let mut blocks: Vec<Block> = self.blocks.drain().map(|(_, b)| b).filter(|b| b.emitted && !b.done).collect();
        blocks.sort_by_key(|b| b.item.seq);
        for mut block in blocks {
            block.item.partial = false;
            sink.effect(Effect::Upsert(block.item));
        }
        let mut tools: Vec<RunningTool> = self.tools.drain().map(|(_, t)| t).collect();
        tools.sort_by_key(|t| t.item.seq);
        for mut tool in tools {
            crate::timeline::settle(&mut tool.item);
            sink.effect(Effect::Upsert(tool.item));
        }
        let finished = std::mem::take(&mut self.finished);
        for (_, mut item) in finished {
            if has_running(&item) {
                crate::timeline::settle(&mut item);
                sink.effect(Effect::Upsert(item));
            }
        }
        self.progress_dirty.clear();
        self.progress_since = None;
        for mut item in [self.compaction.take(), self.retry.take()].into_iter().flatten() {
            item.partial = false;
            sink.effect(Effect::Upsert(item));
        }
    }

    pub fn handle(&mut self, notification: RpcNotification, sink: &mut dyn Sink) {
        match notification {
            RpcNotification::RpcAgentEvent(event) => self.agent_event(event, sink),
            RpcNotification::PromptResult(result) => {
                self.flush(sink);
                let error = result.error.map(|e| e.message);
                match result.status {
                    PromptStatus::Error if !self.error_shown => {
                        let text = error.clone().unwrap_or_else(|| "The turn failed".to_owned());
                        notice(sink, ItemKind::Error, text);
                        self.error_shown = true;
                    }
                    PromptStatus::Aborted => notice(sink, ItemKind::Notice, "Aborted".to_owned()),
                    _ => {}
                }
                sink.effect(Effect::PromptResult { status: result.status, settled: result.session_settled, error });
            }
            RpcNotification::SessionSettled(_) => {
                self.flush(sink);
                sink.effect(Effect::Settled);
            }
            RpcNotification::ExtensionUiRequest(request) => self.ui_request(request, sink),
            RpcNotification::ExtensionError(err) => notice(
                sink,
                ItemKind::Error,
                format!("Extension error ({}) in {}: {}", err.event, err.extension_path, err.error),
            ),
            RpcNotification::SubagentLifecycle(event) => {
                let id = event.payload.id.clone();
                sink.effect(match event.payload.status {
                    SubagentLifecycleStatus::Started => Effect::SubagentStarted(id),
                    _ => Effect::SubagentEnded(id),
                });
                self.subagent_lifecycle(event.payload, sink);
            }
            RpcNotification::SubagentProgress(event) => self.subagent_progress(event.payload),
            RpcNotification::RpcFrameError(err) => {
                tracing::warn!(error = %err.error, original = ?err.original_type, "omp dropped an oversized frame");
            }
            _ => {}
        }
    }

    /// The tool item a subagent belongs to: a running tool call, or a recently finished one.
    fn owner(&mut self, call_id: &str) -> Option<&mut TimelineItem> {
        match self.tools.get_mut(call_id) {
            Some(tool) => Some(&mut tool.item),
            None => self.finished.iter_mut().rev().find(|(id, _)| id == call_id).map(|(_, item)| item),
        }
    }

    /// Started/finished subagents change the tool item persistently.
    fn subagent_lifecycle(&mut self, p: SubagentLifecyclePayload, sink: &mut dyn Sink) {
        let Some(call) = p.parent_tool_call_id else { return };
        let Some(item) = self.owner(&call) else { return };
        let Some(tool) = item.tool.as_mut() else { return };
        let done = p.status != SubagentLifecycleStatus::Started;
        let status = if done { p.status.as_str() } else { "running" }.to_owned();
        let sub = match tool.subagents.iter().position(|s| s.id == p.id) {
            Some(i) => &mut tool.subagents[i],
            None => {
                tool.subagents.push(new_subagent(p.id, p.agent, now_ms()));
                tool.subagents.last_mut().expect("pushed")
            }
        };
        sub.status = status;
        if p.description.is_some() {
            sub.description = p.description;
        }
        if p.session_file.is_some() {
            sub.session_file = p.session_file;
        }
        if done {
            sub.ended_at = Some(now_ms());
            sub.activity = None;
        }
        let item = item.clone();
        self.progress_dirty.retain(|c| *c != call);
        sink.effect(Effect::Upsert(item));
    }

    /// Progress (current tool, counts, cost) only updates memory; [`Self::flush`] sends it.
    fn subagent_progress(&mut self, p: SubagentProgressPayload) {
        let Some(call) = p.parent_tool_call_id else { return };
        let progress = &p.progress;
        let Some(id) = progress.get("id").and_then(Value::as_str) else { return };
        let Some(item) = self.owner(&call) else { return };
        let Some(tool) = item.tool.as_mut() else { return };
        let sub = match tool.subagents.iter().position(|s| s.id == id) {
            Some(i) => &mut tool.subagents[i],
            None => {
                tool.subagents.push(new_subagent(id.to_owned(), p.agent, now_ms()));
                tool.subagents.last_mut().expect("pushed")
            }
        };
        if sub.status != "running" {
            return;
        }
        if sub.description.is_none() {
            sub.description = progress.get("description").and_then(Value::as_str).map(str::to_owned);
        }
        if sub.session_file.is_none() {
            sub.session_file = p.session_file;
        }
        sub.activity = activity(progress);
        sub.tool_count = progress.get("toolCount").and_then(Value::as_u64).map(|n| n as u32);
        sub.cost_usd = progress.get("cost").and_then(Value::as_f64);
        sub.model = progress.get("resolvedModelIdentity").or_else(|| progress.get("resolvedModel")).and_then(Value::as_str).map(str::to_owned);
        if !self.progress_dirty.contains(&call) {
            self.progress_dirty.push(call);
        }
        self.progress_since.get_or_insert_with(Instant::now);
    }

    fn ui_request(&mut self, request: ExtensionUiRequest, sink: &mut dyn Sink) {
        let (id, method) = match &request {
            ExtensionUiRequest::Select(r) => (r.id.clone(), "select"),
            ExtensionUiRequest::Confirm(r) => (r.id.clone(), "confirm"),
            ExtensionUiRequest::Input(r) => (r.id.clone(), "input"),
            ExtensionUiRequest::Editor(r) => (r.id.clone(), "editor"),
            ExtensionUiRequest::Ask(r) => (r.id.clone(), "ask"),
            ExtensionUiRequest::Cancel(r) => {
                sink.effect(Effect::Resolve { id: r.target_id.clone() });
                return;
            }
            ExtensionUiRequest::Notify(r) => {
                leveled_notice(sink, r.notify_type.unwrap_or(NotifyType::Info), r.message.clone());
                return;
            }
            ExtensionUiRequest::OpenUrl(r) => {
                let mut text = format!("Open {}", r.launch_url.as_deref().unwrap_or(&r.url));
                if let Some(instructions) = &r.instructions {
                    text.push_str(" — ");
                    text.push_str(instructions);
                }
                notice(sink, ItemKind::Notice, text);
                return;
            }
            // Terminal presentation surfaces (status line, widgets, title, editor text).
            ExtensionUiRequest::SetStatus(_)
            | ExtensionUiRequest::SetWidget(_)
            | ExtensionUiRequest::SetTitle(_)
            | ExtensionUiRequest::SetEditorText(_) => return,
        };
        let payload = serde_json::to_value(&request).unwrap_or(Value::Null);
        sink.effect(Effect::Request { id, method: method.to_owned(), payload });
    }

    fn agent_event(&mut self, event: RpcAgentEvent, sink: &mut dyn Sink) {
        match event {
            RpcAgentEvent::AgentStart(_) => {
                self.error_shown = false;
                // Finished blocks of earlier runs whose message_end never arrived.
                self.blocks.retain(|_, b| !b.done);
                sink.effect(Effect::AgentStart);
            }
            RpcAgentEvent::MessageUpdate(update) => {
                let message_id = update.message_id.unwrap_or_default();
                match update.assistant_message_event {
                    AssistantMessageEvent::TextDelta(e) => {
                        self.delta(sink, &message_id, e.content_index.unwrap_or(0), ItemKind::Assistant, e.delta.as_deref())
                    }
                    AssistantMessageEvent::TextEnd(e) => {
                        self.end_block(sink, &message_id, e.content_index.unwrap_or(0), ItemKind::Assistant, e.content)
                    }
                    AssistantMessageEvent::ThinkingDelta(e) => {
                        self.delta(sink, &message_id, e.content_index.unwrap_or(0), ItemKind::Thinking, e.delta.as_deref())
                    }
                    AssistantMessageEvent::ThinkingEnd(e) => {
                        self.end_block(sink, &message_id, e.content_index.unwrap_or(0), ItemKind::Thinking, e.content)
                    }
                    _ => {}
                }
            }
            RpcAgentEvent::MessageEnd(end) => self.message_end(sink, end.message_id.unwrap_or_default(), end.message),
            RpcAgentEvent::ToolExecutionStart(start) => {
                self.flush(sink);
                let intent = start.intent.or_else(|| {
                    start.args.as_ref().and_then(|a| a.get("i")).and_then(Value::as_str).map(str::to_owned)
                });
                let args = start.args.as_ref().map_or_else(String::new, |a| truncate(&a.to_string(), MAX_ARGS));
                let mut item = new_item(sink, ItemKind::Tool, None, true);
                item.tool = Some(ToolCall {
                    name: start.tool_name,
                    intent,
                    args,
                    status: ToolStatus::Running,
                    output: None,
                    duration_ms: None,
                    subagents: Vec::new(),
                });
                sink.effect(Effect::Upsert(item.clone()));
                self.tools.insert(start.tool_call_id, RunningTool { item, started: Instant::now() });
            }
            RpcAgentEvent::ToolExecutionEnd(end) => {
                self.flush(sink);
                let (mut item, duration) = match self.tools.remove(&end.tool_call_id) {
                    Some(tool) => (tool.item, Some(tool.started.elapsed().as_millis() as u64)),
                    None => {
                        let mut item = new_item(sink, ItemKind::Tool, None, false);
                        item.tool = Some(ToolCall {
                            name: end.tool_name.clone(),
                            intent: None,
                            args: String::new(),
                            status: ToolStatus::Running,
                            output: None,
                            duration_ms: None,
                            subagents: Vec::new(),
                        });
                        (item, None)
                    }
                };
                item.partial = false;
                if let Some(tool) = &mut item.tool {
                    tool.status = if end.is_error == Some(true) { ToolStatus::Error } else { ToolStatus::Ok };
                    tool.output = end.result.as_ref().map(|r| truncate(&tool_output(r), MAX_OUTPUT));
                    tool.duration_ms = duration;
                }
                self.finished.push((end.tool_call_id.clone(), item.clone()));
                if self.finished.len() > RECENT_TOOLS
                    && let Some(oldest) = self.finished.iter().position(|(_, item)| !has_running(item))
                {
                    self.finished.remove(oldest);
                }
                sink.effect(Effect::Upsert(item));
                if end.tool_name == "todo" {
                    sink.effect(Effect::Refresh);
                }
            }
            RpcAgentEvent::AutoCompactionStart(_) => {
                let item = match self.compaction.take() {
                    Some(item) => item,
                    None => new_item(sink, ItemKind::Notice, None, true),
                };
                let item = TimelineItem { text: Some("Compacting context…".to_owned()), partial: true, ..item };
                sink.effect(Effect::Upsert(item.clone()));
                self.compaction = Some(item);
            }
            RpcAgentEvent::AutoCompactionEnd(end) => {
                let mut item = match self.compaction.take() {
                    Some(item) => item,
                    None => new_item(sink, ItemKind::Notice, None, false),
                };
                item.partial = false;
                item.text = Some(if end.aborted {
                    "Compaction aborted".to_owned()
                } else if end.skipped == Some(true) {
                    "Compaction skipped".to_owned()
                } else if let Some(err) = end.error_message {
                    item.kind = ItemKind::Error;
                    format!("Compaction failed: {err}")
                } else if let Some(result) = end.result {
                    format!("Context compacted ({} tokens summarized)", result.tokens_before)
                } else {
                    "Context compacted".to_owned()
                });
                sink.effect(Effect::Upsert(item));
                sink.effect(Effect::Refresh);
            }
            RpcAgentEvent::AutoRetryStart(start) => {
                let item = match self.retry.take() {
                    Some(item) => item,
                    None => new_item(sink, ItemKind::Notice, None, true),
                };
                let text = format!(
                    "Retrying in {}s (attempt {}/{}): {}",
                    (start.delay_ms + 999) / 1000,
                    start.attempt,
                    start.max_attempts,
                    start.error_message
                );
                let item = TimelineItem { text: Some(text), partial: true, ..item };
                sink.effect(Effect::Upsert(item.clone()));
                self.retry = Some(item);
            }
            RpcAgentEvent::AutoRetryEnd(end) => {
                let mut item = match self.retry.take() {
                    Some(item) => item,
                    None => new_item(sink, ItemKind::Notice, None, false),
                };
                item.partial = false;
                if end.success {
                    item.text = Some(format!("Recovered after {} attempt(s)", end.attempt));
                } else {
                    item.kind = ItemKind::Error;
                    item.text = Some(format!(
                        "Retries exhausted: {}",
                        end.final_error.as_deref().unwrap_or("request failed")
                    ));
                    self.error_shown = true;
                }
                sink.effect(Effect::Upsert(item));
            }
            RpcAgentEvent::RetryFallbackApplied(e) => {
                notice(sink, ItemKind::Notice, format!("Switched model {} → {}", e.from, e.to));
                sink.effect(Effect::Refresh);
            }
            RpcAgentEvent::ModelChanged(_) => sink.effect(Effect::Refresh),
            RpcAgentEvent::ThinkingLevelChanged(e) => {
                if let Some(level) = e.thinking_level {
                    sink.effect(Effect::Thinking(level.as_str().to_owned()));
                }
            }
            RpcAgentEvent::Notice(e) => leveled_notice(sink, e.level, e.message),
            RpcAgentEvent::QueueUpdate(q) => sink.effect(Effect::Queue { steering: q.steering, follow_up: q.follow_up }),
            RpcAgentEvent::TodoAutoClear(_) => sink.effect(Effect::TodosCleared),
            RpcAgentEvent::IrcMessage(irc) => {
                let text = irc.message.content.map(content_text).unwrap_or_default();
                if !text.is_empty() {
                    notice(sink, ItemKind::Notice, text);
                }
            }
            _ => {}
        }
    }

    fn delta(&mut self, sink: &mut dyn Sink, message_id: &str, index: i64, kind: ItemKind, delta: Option<&str>) {
        let Some(delta) = delta.filter(|d| !d.is_empty()) else { return };
        let key = (message_id.to_owned(), index);
        let block = self.blocks.entry(key).or_insert_with(|| Block {
            item: new_item(sink, kind, Some(String::new()), true),
            emitted: false,
            done: false,
        });
        block.item.text.get_or_insert_with(String::new).push_str(delta);
        if !block.emitted {
            block.emitted = true;
            let item = block.item.clone();
            if self.pending_delta.as_ref().is_some_and(|(id, _)| *id != item.id) {
                self.flush(sink);
            }
            sink.effect(Effect::Upsert(item));
            return;
        }
        let item_id = block.item.id.clone();
        match &mut self.pending_delta {
            Some((id, text)) if *id == item_id => text.push_str(delta),
            _ => {
                self.flush(sink);
                self.pending_delta = Some((item_id, delta.to_owned()));
                self.delta_since = Some(Instant::now());
            }
        }
    }

    fn end_block(&mut self, sink: &mut dyn Sink, message_id: &str, index: i64, kind: ItemKind, content: Option<String>) {
        self.flush(sink);
        let key = (message_id.to_owned(), index);
        let block = self.blocks.remove(&key);
        if let Some(done) = self.finish_block(sink, block, kind, content) {
            self.blocks.insert(key, done);
        }
    }

    /// Finalize a block with its complete text. Returns the finished block (kept until
    /// `message_end` so the final message does not duplicate it).
    fn finish_block(&mut self, sink: &mut dyn Sink, block: Option<Block>, kind: ItemKind, content: Option<String>) -> Option<Block> {
        let text = content.or_else(|| block.as_ref().and_then(|b| b.item.text.clone()));
        if block.as_ref().is_some_and(|b| b.done && b.item.text == text) {
            return block;
        }
        let emitted = block.as_ref().is_some_and(|b| b.emitted);
        if !emitted && text.as_deref().is_none_or(|t| t.trim().is_empty()) {
            return block;
        }
        let mut item = match block {
            Some(block) => block.item,
            None => new_item(sink, kind, None, false),
        };
        item.text = text;
        item.partial = false;
        sink.effect(Effect::Upsert(item.clone()));
        Some(Block { item, emitted: true, done: true })
    }

    fn message_end(&mut self, sink: &mut dyn Sink, message_id: String, message: AgentMessage) {
        self.flush(sink);
        match message {
            AgentMessage::User(user) => {
                if user.synthetic == Some(true) {
                    return;
                }
                let text = user.content.map(content_text).unwrap_or_default();
                if !text.is_empty() {
                    notice(sink, ItemKind::User, text);
                }
            }
            AgentMessage::Assistant(assistant) => {
                let mut visible = String::new();
                for (index, content) in assistant.content.unwrap_or_default().into_iter().enumerate() {
                    let block = self.blocks.remove(&(message_id.clone(), index as i64));
                    match content {
                        AssistantContent::Text(text) => {
                            let text = text.text.unwrap_or_default();
                            if !text.trim().is_empty() {
                                if !visible.is_empty() {
                                    visible.push_str("\n\n");
                                }
                                visible.push_str(&text);
                            }
                            self.finish_block(sink, block, ItemKind::Assistant, Some(text));
                        }
                        AssistantContent::Thinking(thinking) => {
                            self.finish_block(sink, block, ItemKind::Thinking, thinking.thinking);
                        }
                        _ => {
                            self.finish_block(sink, block, ItemKind::Assistant, None);
                        }
                    }
                }
                // Blocks the final message no longer lists (should not happen) are closed too.
                let stale: Vec<(String, i64)> = self.blocks.keys().filter(|(id, _)| *id == message_id).cloned().collect();
                for key in stale {
                    let block = self.blocks.remove(&key);
                    self.finish_block(sink, block, ItemKind::Assistant, None);
                }
                if !visible.is_empty() {
                    sink.effect(Effect::Output(visible));
                }
                if assistant.stop_reason == Some(StopReason::Error) {
                    let text = assistant.error_message.unwrap_or_else(|| "The model request failed".to_owned());
                    notice(sink, ItemKind::Error, text);
                    self.error_shown = true;
                }
            }
            _ => {}
        }
    }
}

fn new_subagent(id: String, agent: String, started_at: i64) -> Subagent {
    Subagent {
        id,
        agent,
        description: None,
        status: "running".to_owned(),
        started_at,
        ended_at: None,
        activity: None,
        tool_count: None,
        cost_usd: None,
        model: None,
        session_file: None,
    }
}

/// One line on what a subagent is doing, from omp's `AgentProgress`: the running tool with its
/// intent (or its argument preview), else the last intent it stated.
fn activity(progress: &Map<String, Value>) -> Option<String> {
    let text = |key: &str| progress.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let line = match text("currentTool") {
        Some(tool) => match text("currentToolIntent").or_else(|| text("currentToolArgs")) {
            Some(detail) => format!("{tool} · {detail}"),
            None => tool.to_owned(),
        },
        None => text("lastIntent")?.to_owned(),
    };
    Some(truncate(&line.replace('\n', " "), MAX_ACTIVITY))
}

/// Visible text of user/custom content; images are summarized.
pub(crate) fn content_text(content: MessageContent) -> String {
    match content {
        MessageContent::String(text) => text,
        MessageContent::UserContentList(parts) => {
            let mut text = String::new();
            let mut images = 0;
            for part in parts {
                match part {
                    UserContent::Text(t) => {
                        if let Some(t) = t.text {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&t);
                        }
                    }
                    UserContent::Image(_) => images += 1,
                }
            }
            if images > 0 {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&format!("[{images} image{}]", if images == 1 { "" } else { "s" }));
            }
            text
        }
    }
}

/// Text of a tool result (`{content: [{type:"text", text}], details}`), or its JSON.
pub(crate) fn tool_output(result: &Value) -> String {
    if let Some(text) = result.as_str() {
        return text.to_owned();
    }
    if let Some(parts) = result.get("content").and_then(Value::as_array) {
        let mut out = String::new();
        for part in parts {
            let piece = match part.get("type").and_then(Value::as_str) {
                Some("text") => part.get("text").and_then(Value::as_str).unwrap_or_default(),
                Some("image") => "[image]",
                _ => continue,
            };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(piece);
        }
        return out;
    }
    result.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        next: u64,
        effects: Vec<Effect>,
    }

    impl Sink for Recorder {
        fn seq(&mut self) -> u64 {
            self.next += 1;
            self.next
        }
        fn effect(&mut self, effect: Effect) {
            self.effects.push(effect);
        }
    }

    impl Recorder {
        fn upserts(&self) -> Vec<&TimelineItem> {
            self.effects.iter().filter_map(|e| if let Effect::Upsert(i) = e { Some(i) } else { None }).collect()
        }
        /// Final version of every item, in seq order.
        fn items(&self) -> Vec<TimelineItem> {
            let mut map = std::collections::BTreeMap::new();
            for item in self.upserts() {
                map.insert(item.seq, item.clone());
            }
            map.into_values().collect()
        }
    }

    fn feed(mapper: &mut Mapper, rec: &mut Recorder, frames: &str) {
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let value: Value = serde_json::from_str(line).unwrap();
            if value.get("type").and_then(Value::as_str) == Some("response") {
                continue;
            }
            mapper.handle(RpcNotification::from_value(value).unwrap(), rec);
        }
        mapper.flush(rec);
    }

    /// Frames recorded from `omp --mode rpc` (v18.12.3, `messageUpdates: "delta"`): a prompt
    /// that runs `echo hi` through bash and answers "pong".
    const RECORDED: &str = include_str!("testdata/bash_pong.jsonl");

    #[test]
    fn recorded_turn_maps_to_user_tool_assistant() {
        let (mut mapper, mut rec) = (Mapper::new(), Recorder::default());
        feed(&mut mapper, &mut rec, RECORDED);
        let items = rec.items();
        let kinds: Vec<ItemKind> = items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![ItemKind::User, ItemKind::Tool, ItemKind::Assistant], "{items:#?}");
        assert!(items[0].text.as_deref().unwrap().contains("echo hi"));
        let tool = items[1].tool.as_ref().unwrap();
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.intent.as_deref(), Some("Running echo command"));
        assert_eq!(tool.args, r#"{"command":"echo hi"}"#);
        assert_eq!(tool.status, ToolStatus::Ok);
        assert_eq!(tool.output.as_deref(), Some("hi\n"));
        assert!(tool.duration_ms.is_some());
        assert_eq!(items[2].text.as_deref(), Some("pong"));
        assert!(items.iter().all(|i| !i.partial));

        let lifecycle: Vec<&Effect> = rec
            .effects
            .iter()
            .filter(|e| matches!(e, Effect::AgentStart | Effect::PromptResult { .. } | Effect::Settled | Effect::Output(_)))
            .collect();
        assert_eq!(
            lifecycle,
            vec![
                &Effect::AgentStart,
                &Effect::Output("pong".into()),
                &Effect::PromptResult { status: PromptStatus::Completed, settled: true, error: None },
                &Effect::Settled,
            ]
        );
        // `setWidget` presentation frames are ignored.
        assert!(!rec.effects.iter().any(|e| matches!(e, Effect::Request { .. })));
    }

    #[test]
    fn streaming_deltas_are_coalesced() {
        let (mut mapper, mut rec) = (Mapper::new(), Recorder::default());
        let frames = r#"
{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_start","contentIndex":0}}
{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hel"}}
{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"lo "}}
{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"wor"}}
{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"ld"}}
"#;
        for line in frames.lines().filter(|l| !l.is_empty()) {
            mapper.handle(RpcNotification::from_value(serde_json::from_str(line).unwrap()).unwrap(), &mut rec);
        }
        // First text creates the partial item; the rest is buffered until the window closes.
        assert_eq!(rec.upserts().len(), 1);
        assert_eq!(rec.upserts()[0].text.as_deref(), Some("Hel"));
        assert!(rec.upserts()[0].partial);
        assert!(mapper.deadline().is_some());
        assert!(!rec.effects.iter().any(|e| matches!(e, Effect::Delta { .. })));
        mapper.flush(&mut rec);
        assert_eq!(rec.effects.last(), Some(&Effect::Delta { item_id: "1".into(), text: "lo world".into() }));
        assert!(mapper.deadline().is_none());

        let end = r#"{"type":"message_update","messageId":"m1","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_end","contentIndex":0,"content":"Hello world"}}"#;
        mapper.handle(RpcNotification::from_value(serde_json::from_str(end).unwrap()).unwrap(), &mut rec);
        let last = rec.upserts().last().cloned().unwrap().clone();
        assert_eq!((last.id.as_str(), last.text.as_deref(), last.partial), ("1", Some("Hello world"), false));
    }

    #[test]
    fn dialogs_errors_and_crash_cleanup() {
        let (mut mapper, mut rec) = (Mapper::new(), Recorder::default());
        let frames = r#"
{"type":"agent_start"}
{"type":"extension_ui_request","id":"ui1","method":"confirm","title":"Run?","message":"rm -rf build"}
{"type":"extension_ui_request","id":"ui2","method":"cancel","targetId":"ui1"}
{"type":"extension_ui_request","id":"ui3","method":"notify","message":"heads up","notifyType":"warning"}
{"type":"notice","level":"info","message":"xd://: mounted mcp__example"}
{"type":"tool_execution_start","toolCallId":"t1","toolName":"bash","args":{"command":"sleep 100","i":"Sleeping"}}
{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"overloaded"}
{"type":"auto_retry_end","success":false,"attempt":3,"finalError":"overloaded"}
{"type":"prompt_result","id":"req_1","agentInvoked":true,"status":"error","sessionSettled":true,"error":{"message":"overloaded","retryable":true}}
{"type":"subagent_lifecycle","payload":{"id":"Sub1","agent":"task","agentSource":"bundled","status":"started","index":0}}
"#;
        feed(&mut mapper, &mut rec, frames);
        let request = rec.effects.iter().find_map(|e| match e {
            Effect::Request { id, method, payload } => Some((id.clone(), method.clone(), payload.clone())),
            _ => None,
        });
        let (id, method, payload) = request.unwrap();
        assert_eq!((id.as_str(), method.as_str()), ("ui1", "confirm"));
        assert_eq!(payload["type"], "extension_ui_request");
        assert_eq!(payload["message"], "rm -rf build");
        assert!(rec.effects.contains(&Effect::Resolve { id: "ui1".into() }));
        assert!(rec.effects.contains(&Effect::SubagentStarted("Sub1".into())));
        assert!(rec.effects.contains(&Effect::PromptResult {
            status: PromptStatus::Error,
            settled: true,
            error: Some("overloaded".into())
        }));

        mapper.finish(&mut rec);
        let items = rec.items();
        let texts: Vec<(ItemKind, Option<&str>)> = items.iter().map(|i| (i.kind, i.text.as_deref())).collect();
        assert_eq!(
            texts,
            vec![
                (ItemKind::Notice, Some("heads up")),
                (ItemKind::Tool, None),
                (ItemKind::Error, Some("Retries exhausted: overloaded")),
            ],
            "the failed prompt must not add a second error item"
        );
        let tool = items[1].tool.as_ref().unwrap();
        assert_eq!(tool.intent.as_deref(), Some("Sleeping"));
        assert_eq!(tool.status, ToolStatus::Error);
        assert!(!items[1].partial);
    }

    #[test]
    fn subagents_live_on_their_task_call() {
        let (mut mapper, mut rec) = (Mapper::new(), Recorder::default());
        let progress = |tool: &str| {
            format!(
                r#"{{"type":"subagent_progress","payload":{{"index":0,"agent":"task","agentSource":"bundled","task":"t","parentToolCallId":"c1","sessionFile":"/s/A.jsonl","progress":{{"id":"A","status":"running","currentTool":"{tool}","currentToolIntent":"Reading docs","toolCount":3,"cost":0.02,"resolvedModel":"anthropic/x"}}}}}}"#
            )
        };
        let frames = format!(
            r#"
{{"type":"tool_execution_start","toolCallId":"c1","toolName":"task","args":{{"tasks":[]}}}}
{{"type":"subagent_lifecycle","payload":{{"id":"A","agent":"task","agentSource":"bundled","status":"started","index":0,"description":"Docs","parentToolCallId":"c1"}}}}
{{"type":"subagent_lifecycle","payload":{{"id":"B","agent":"scout","agentSource":"bundled","status":"started","index":1,"parentToolCallId":"c1","detached":true}}}}
{}
{}
"#,
            progress("grep"),
            progress("read")
        );
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            mapper.handle(RpcNotification::from_value(serde_json::from_str(line).unwrap()).unwrap(), &mut rec);
        }
        // Progress is held back for the window, then sent once with the latest state, unpersisted.
        assert!(!rec.effects.iter().any(|e| matches!(e, Effect::Progress(_))));
        mapper.progress_since = Some(Instant::now() - PROGRESS_WINDOW);
        mapper.flush(&mut rec);
        let live: Vec<&TimelineItem> = rec.effects.iter().filter_map(|e| if let Effect::Progress(i) = e { Some(i) } else { None }).collect();
        assert_eq!(live.len(), 1);
        let a = &live[0].tool.as_ref().unwrap().subagents[0];
        assert_eq!(
            (a.description.as_deref(), a.activity.as_deref(), a.tool_count, a.model.as_deref(), a.session_file.as_deref()),
            (Some("Docs"), Some("read · Reading docs"), Some(3), Some("anthropic/x"), Some("/s/A.jsonl"))
        );

        // The call returns while B (detached) still runs, and an async C only reports afterwards;
        // both still update the call's item.
        let rest = r#"
{"type":"subagent_lifecycle","payload":{"id":"A","agent":"task","agentSource":"bundled","status":"completed","index":0,"parentToolCallId":"c1"}}
{"type":"tool_execution_end","toolCallId":"c1","toolName":"task","result":{"content":[{"type":"text","text":"ok"}]},"isError":false}
{"type":"subagent_lifecycle","payload":{"id":"C","agent":"task","agentSource":"bundled","status":"started","index":2,"parentToolCallId":"c1"}}
{"type":"subagent_lifecycle","payload":{"id":"B","agent":"scout","agentSource":"bundled","status":"failed","index":1,"parentToolCallId":"c1"}}
"#;
        feed(&mut mapper, &mut rec, rest);
        let items = rec.items();
        let tool = items[0].tool.as_ref().unwrap();
        let states: Vec<(&str, &str, bool, bool)> =
            tool.subagents.iter().map(|s| (s.id.as_str(), s.status.as_str(), s.ended_at.is_some(), s.activity.is_some())).collect();
        assert_eq!(states, [("A", "completed", true, false), ("B", "failed", true, false), ("C", "running", false, false)]);
        assert_eq!(tool.status, ToolStatus::Ok);
        assert!(has_running(&items[0]), "C still runs");
        mapper.finish(&mut rec);
        let c = rec.items()[0].tool.as_ref().unwrap().subagents[2].clone();
        assert_eq!(c.status, "aborted", "the process went away");
    }
}
