//! Per-agent timeline storage.
//!
//! `timeline.jsonl` is append-only: every upsert appends the full item, and readers keep the
//! last line per item. Item ids are the decimal `seq` allocated when the item was created, so an
//! upsert always keeps its first `seq`. Memory holds only a short tail of the newest items
//! (including streaming text that is not persisted until the block ends).

use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use coocoo_proto::{ItemKind, TimelineItem, TimelinePage, ToolStatus};

/// Items kept in memory per agent.
const TAIL_LEN: usize = 64;

pub(crate) struct Timeline {
    path: PathBuf,
    file: Option<File>,
    /// `None` until the file was scanned once (lazily, on the first write).
    next_seq: Option<u64>,
    tail: VecDeque<TimelineItem>,
}

impl Timeline {
    pub fn new(path: PathBuf) -> Self {
        Self { path, file: None, next_seq: None, tail: VecDeque::new() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn ensure_loaded(&mut self) {
        if self.next_seq.is_some() {
            return;
        }
        let items = read_all(&self.path);
        self.next_seq = Some(items.keys().next_back().map_or(1, |seq| seq + 1));
        let skip = items.len().saturating_sub(TAIL_LEN);
        self.tail = items.into_values().skip(skip).collect();
    }

    /// Allocate the next `seq` (monotonic per agent, persisted implicitly by the items).
    pub fn alloc(&mut self) -> u64 {
        self.ensure_loaded();
        let seq = self.next_seq.unwrap_or(1);
        self.next_seq = Some(seq + 1);
        seq
    }

    /// Persist an item (insert or replace) and update the in-memory tail.
    pub fn upsert(&mut self, item: &TimelineItem) -> io::Result<()> {
        self.ensure_loaded();
        self.remember(item.clone());
        let mut line = serde_json::to_vec(item).map_err(io::Error::other)?;
        line.push(b'\n');
        if self.file.is_none() {
            self.file = Some(open_append(&self.path)?);
        }
        let file = self.file.as_mut().expect("opened above");
        if let Err(err) = file.write_all(&line) {
            self.file = None;
            return Err(err);
        }
        Ok(())
    }

    /// Replace a tail item without persisting it (live subagent progress; the next upsert does).
    pub fn overlay(&mut self, item: TimelineItem) {
        self.ensure_loaded();
        self.remember(item);
    }

    /// Append streaming text to a tail item without persisting it (the final upsert does).
    pub fn append_text(&mut self, item_id: &str, text: &str) {
        if let Some(item) = self.tail.iter_mut().rev().find(|i| i.id == item_id) {
            item.text.get_or_insert_with(String::new).push_str(text);
        }
    }

    pub fn tail(&self) -> Vec<TimelineItem> {
        self.tail.iter().cloned().collect()
    }

    /// Time of the newest item of `kind` still in the in-memory tail.
    pub fn last_ts(&mut self, kind: ItemKind) -> Option<i64> {
        self.ensure_loaded();
        self.tail.iter().rev().find(|i| i.kind == kind).map(|i| i.ts)
    }

    /// Release the file handle (agent went offline).
    pub fn close(&mut self) {
        self.file = None;
    }

    fn remember(&mut self, item: TimelineItem) {
        if let Some(slot) = self.tail.iter_mut().rev().find(|i| i.seq == item.seq) {
            *slot = item;
            return;
        }
        match self.tail.back() {
            Some(last) if item.seq < last.seq => {
                // Older item that already fell out of the tail stays on disk only.
                if self.tail.front().is_some_and(|first| item.seq > first.seq) {
                    let at = self.tail.partition_point(|i| i.seq < item.seq);
                    self.tail.insert(at, item);
                }
            }
            _ => self.tail.push_back(item),
        }
        while self.tail.len() > TAIL_LEN {
            self.tail.pop_front();
        }
    }
}

/// Open for appending; a torn final line (crash mid-write) is terminated first so the next
/// item starts on its own line.
fn open_append(path: &Path) -> io::Result<File> {
    use std::io::{Read, Seek, SeekFrom};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).read(true).append(true).open(path)?;
    if file.metadata()?.len() > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
        }
    }
    Ok(file)
}

/// Every item in the file, last line per `seq` wins. Malformed lines (a torn final write) are
/// skipped.
fn read_all(path: &Path) -> BTreeMap<u64, TimelineItem> {
    let mut items = BTreeMap::new();
    let Ok(file) = File::open(path) else { return items };
    for line in BufReader::with_capacity(1 << 16, file).split(b'\n') {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        if let Ok(item) = serde_json::from_slice::<TimelineItem>(&line) {
            items.insert(item.seq, item);
        }
    }
    items
}

/// One page of items with `seq < before`, newest `limit` items, oldest first.
///
/// `tail` overlays the on-disk state with the in-memory versions (streaming text). When the
/// agent has no live process, leftover partial items (daemon crash mid-turn) are presented as
/// finished so the UI does not show them streaming forever.
pub(crate) fn page(path: &Path, tail: Vec<TimelineItem>, before: Option<u64>, limit: usize, live: bool) -> TimelinePage {
    let mut items = read_all(path);
    for item in tail {
        items.insert(item.seq, item);
    }
    if let Some(before) = before {
        items.split_off(&before);
    }
    let has_more = items.len() > limit;
    let skip = items.len().saturating_sub(limit);
    let mut items: Vec<TimelineItem> = items.into_values().skip(skip).collect();
    if !live {
        for item in &mut items {
            settle(item);
        }
    }
    TimelinePage { items, has_more }
}

/// Mark a streaming item as finished because its process is gone; its subagents went with it.
pub(crate) fn settle(item: &mut TimelineItem) {
    if let Some(tool) = &mut item.tool {
        for sub in tool.subagents.iter_mut().filter(|s| s.status == "running") {
            sub.status = "aborted".to_owned();
            sub.activity = None;
        }
    }
    if !item.partial {
        return;
    }
    item.partial = false;
    if let Some(tool) = &mut item.tool
        && tool.status == ToolStatus::Running
    {
        tool.status = ToolStatus::Error;
        tool.output.get_or_insert_with(|| "interrupted".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use coocoo_proto::{ItemKind, ToolCall};

    use super::*;

    fn item(seq: u64, kind: ItemKind, text: &str, partial: bool) -> TimelineItem {
        TimelineItem { seq, id: seq.to_string(), ts: seq as i64, kind, text: Some(text.to_owned()), tool: None, partial, images: Vec::new() }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coocoo-agents-test-{}-{name}", coocoo_proto::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("timeline.jsonl")
    }

    #[test]
    fn upserts_keep_first_seq_and_last_version_wins() {
        let path = temp_path("upsert");
        let mut tl = Timeline::new(path.clone());
        let a = tl.alloc();
        let b = tl.alloc();
        assert_eq!((a, b), (1, 2));
        tl.upsert(&item(a, ItemKind::Assistant, "", true)).unwrap();
        tl.upsert(&item(b, ItemKind::User, "hi", false)).unwrap();
        tl.upsert(&item(a, ItemKind::Assistant, "hello world", false)).unwrap();

        let page = page(&path, Vec::new(), None, 200, true);
        assert!(!page.has_more);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].seq, 1);
        assert_eq!(page.items[0].text.as_deref(), Some("hello world"));
        assert!(!page.items[0].partial);
        assert_eq!(page.items[1].text.as_deref(), Some("hi"));

        // A fresh store (daemon restart) continues the sequence after the highest seq.
        let mut reopened = Timeline::new(path.clone());
        assert_eq!(reopened.alloc(), 3);
        assert_eq!(reopened.tail().len(), 2);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn paginates_backwards_from_cursor() {
        let path = temp_path("page");
        let mut tl = Timeline::new(path.clone());
        for n in 0..10 {
            let seq = tl.alloc();
            tl.upsert(&item(seq, ItemKind::Notice, &format!("n{n}"), false)).unwrap();
        }
        let first = page(&path, tl.tail(), None, 4, true);
        assert_eq!(first.items.iter().map(|i| i.seq).collect::<Vec<_>>(), vec![7, 8, 9, 10]);
        assert!(first.has_more);
        let second = page(&path, tl.tail(), Some(7), 4, true);
        assert_eq!(second.items.iter().map(|i| i.seq).collect::<Vec<_>>(), vec![3, 4, 5, 6]);
        assert!(second.has_more);
        let last = page(&path, tl.tail(), Some(3), 4, true);
        assert_eq!(last.items.iter().map(|i| i.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert!(!last.has_more);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn streaming_text_overlays_disk_and_settles_when_offline() {
        let path = temp_path("overlay");
        let mut tl = Timeline::new(path.clone());
        let seq = tl.alloc();
        tl.upsert(&item(seq, ItemKind::Assistant, "Hel", true)).unwrap();
        tl.append_text(&seq.to_string(), "lo");
        let tool_seq = tl.alloc();
        let mut tool = item(tool_seq, ItemKind::Tool, "", true);
        tool.text = None;
        tool.tool = Some(ToolCall {
            name: "bash".into(),
            intent: None,
            args: "{}".into(),
            status: ToolStatus::Running,
            output: None,
            duration_ms: None,
            subagents: Vec::new(),
        });
        tl.upsert(&tool).unwrap();

        let live = page(&path, tl.tail(), None, 200, true);
        assert_eq!(live.items[0].text.as_deref(), Some("Hello"));
        assert!(live.items[0].partial);

        let offline = page(&path, Vec::new(), None, 200, false);
        assert_eq!(offline.items[0].text.as_deref(), Some("Hel"));
        assert!(!offline.items[0].partial);
        let tool = offline.items[1].tool.as_ref().unwrap();
        assert_eq!(tool.status, ToolStatus::Error);
        assert_eq!(tool.output.as_deref(), Some("interrupted"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn torn_lines_are_skipped() {
        let path = temp_path("torn");
        std::fs::write(
            &path,
            "{\"seq\":1,\"id\":\"1\",\"ts\":1,\"kind\":\"user\",\"text\":\"a\"}\n{\"seq\":2,\"id\":\"2\",\"ts\"",
        )
        .unwrap();
        let page = page(&path, Vec::new(), None, 10, false);
        assert_eq!(page.items.len(), 1);
        let mut tl = Timeline::new(path.clone());
        assert_eq!(tl.alloc(), 2);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
