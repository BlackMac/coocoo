//! The usage log: `<data_dir>/events/YYYY-MM.jsonl` (UTC months), append-only.
//!
//! Events are buffered in memory and appended at most [`FLUSH_AFTER`] after the first one of a
//! batch (and on shutdown); nothing runs while nothing is recorded.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use coocoo_proto::insights::InsightEvent;
use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::time::utc_month;

const FLUSH_AFTER: Duration = Duration::from_secs(2);
/// Buffered events beyond this are dropped (the disk is gone or very slow).
const MAX_BUFFER: usize = 10_000;

pub struct EventLog {
    dir: PathBuf,
    enabled: AtomicBool,
    retention_days: Mutex<u32>,
    buffer: Mutex<Vec<InsightEvent>>,
    /// Serializes file writes and deletion.
    files: Mutex<()>,
    /// Oldest recorded event, once known (`Some(None)` = the log is empty).
    since: Mutex<Option<Option<i64>>>,
    handle: Handle,
}

impl EventLog {
    pub fn new(dir: PathBuf, enabled: bool, retention_days: u32, handle: Handle) -> Arc<Self> {
        let log = Arc::new(Self {
            dir,
            enabled: AtomicBool::new(enabled),
            retention_days: Mutex::new(retention_days),
            buffer: Mutex::new(Vec::new()),
            files: Mutex::new(()),
            since: Mutex::new(None),
            handle,
        });
        log.prune();
        log
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Switching off drops what is still buffered: nothing is written from then on.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        if !enabled {
            self.buffer.lock().clear();
        }
    }

    pub fn set_retention(&self, days: u32) {
        *self.retention_days.lock() = days;
        self.prune();
    }

    /// Queue an event (dropped while the log is off).
    pub fn record(self: &Arc<Self>, event: InsightEvent) {
        if !self.enabled() {
            return;
        }
        let first = {
            let mut buffer = self.buffer.lock();
            if buffer.len() >= MAX_BUFFER {
                return;
            }
            buffer.push(event);
            buffer.len() == 1
        };
        if first {
            let me = self.clone();
            self.handle.spawn(async move {
                tokio::time::sleep(FLUSH_AFTER).await;
                let _ = tokio::task::spawn_blocking(move || me.flush()).await;
            });
        }
    }

    /// Append everything buffered (blocking).
    pub fn flush(&self) {
        let events = std::mem::take(&mut *self.buffer.lock());
        if events.is_empty() {
            return;
        }
        let _files = self.files.lock();
        if let Err(err) = std::fs::create_dir_all(&self.dir) {
            tracing::warn!(%err, "cannot create the usage log directory");
            return;
        }
        let mut start = 0;
        while start < events.len() {
            let month = utc_month(events[start].ts);
            let end = events[start..].iter().position(|e| utc_month(e.ts) != month).map_or(events.len(), |n| start + n);
            let mut text = String::new();
            for event in &events[start..end] {
                if let Ok(line) = serde_json::to_string(event) {
                    text.push_str(&line);
                    text.push('\n');
                }
            }
            let path = self.dir.join(format!("{month}.jsonl"));
            let written = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(text.as_bytes()));
            if let Err(err) = written {
                tracing::warn!(file = %path.display(), %err, "usage log write failed");
            }
            start = end;
        }
        let first = events.iter().map(|e| e.ts).min();
        let mut since = self.since.lock();
        if let Some(None) = *since {
            *since = Some(first);
        }
    }

    /// Events with `from <= ts < to`, oldest first (including buffered ones).
    pub fn read(&self, from: i64, to: i64) -> Vec<InsightEvent> {
        let mut events = Vec::new();
        {
            let _files = self.files.lock();
            for (month, path) in self.months() {
                if !month_overlaps(&month, from, to) {
                    continue;
                }
                let Ok(file) = std::fs::File::open(&path) else { continue };
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    match serde_json::from_str::<InsightEvent>(&line) {
                        Ok(e) if e.ts >= from && e.ts < to => events.push(e),
                        Ok(_) => {}
                        Err(err) => tracing::debug!(%err, "skipping unreadable usage log line"),
                    }
                }
            }
        }
        events.extend(self.buffer.lock().iter().filter(|e| e.ts >= from && e.ts < to).cloned());
        events.sort_by_key(|e| e.ts);
        events
    }

    /// The oldest recorded event (Unix ms); `None` while the log is empty.
    pub fn since(&self) -> Option<i64> {
        if let Some(known) = *self.since.lock() {
            return known;
        }
        let oldest = {
            let _files = self.files.lock();
            self.months().into_iter().find_map(|(_, path)| first_ts(&path))
        };
        let oldest = oldest.or_else(|| self.buffer.lock().iter().map(|e| e.ts).min());
        *self.since.lock() = Some(oldest);
        oldest
    }

    /// Size of the log files in bytes.
    pub fn bytes(&self) -> u64 {
        self.months().iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
    }

    /// Delete the whole log.
    pub fn clear(&self) -> std::io::Result<()> {
        self.buffer.lock().clear();
        let _files = self.files.lock();
        *self.since.lock() = Some(None);
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Drop month files entirely older than the retention period.
    fn prune(&self) {
        let days = i64::from(*self.retention_days.lock());
        if days == 0 {
            return;
        }
        let cutoff = utc_month(coocoo_proto::util::now_ms() - days * 86_400_000);
        let _files = self.files.lock();
        let mut removed = false;
        for (month, path) in self.months() {
            if month < cutoff {
                removed |= std::fs::remove_file(&path).is_ok();
            }
        }
        if removed {
            *self.since.lock() = None;
        }
    }

    /// `(YYYY-MM, path)` of every log file, oldest first.
    fn months(&self) -> Vec<(String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut files: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let month = name.strip_suffix(".jsonl")?;
                (month.len() == 7 && month.as_bytes()[4] == b'-').then(|| (month.to_owned(), e.path()))
            })
            .collect();
        files.sort();
        files
    }
}

fn first_ts(path: &Path) -> Option<i64> {
    let file = std::fs::File::open(path).ok()?;
    BufReader::new(file).lines().map_while(Result::ok).find_map(|l| serde_json::from_str::<InsightEvent>(&l).ok()).map(|e| e.ts)
}

/// Whether UTC month `YYYY-MM` overlaps `[from, to)`.
fn month_overlaps(month: &str, from: i64, to: i64) -> bool {
    let first = utc_month(from.max(0));
    let last = utc_month((to - 1).max(0));
    month >= first.as_str() && month <= last.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(ts: i64, kind: &str) -> InsightEvent {
        InsightEvent { ts, ..InsightEvent::new(kind) }
    }

    #[tokio::test]
    async fn writes_by_month_reads_ranges_and_clears() {
        let dir = std::env::temp_dir().join(format!("coocoo-log-{}", coocoo_proto::util::new_id()));
        let log = EventLog::new(dir.clone(), true, 0, Handle::current());
        // 2026-09-30 23:00 UTC and 2026-10-01 01:00 UTC land in different files.
        log.record(event(1_790_809_200_000, "a"));
        log.record(event(1_790_816_400_000, "b"));
        assert_eq!(log.read(0, i64::MAX).len(), 2, "buffered events are readable");
        log.flush();
        assert!(dir.join("2026-09.jsonl").exists() && dir.join("2026-10.jsonl").exists());
        assert_eq!(log.read(1_790_812_800_000, i64::MAX).iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(log.since(), Some(1_790_809_200_000));

        log.set_enabled(false);
        log.record(event(1_790_816_400_001, "c"));
        log.flush();
        assert_eq!(log.read(0, i64::MAX).len(), 2, "nothing is recorded while off");

        log.clear().unwrap();
        assert!(!dir.exists());
        assert_eq!((log.read(0, i64::MAX).len(), log.since()), (0, None));
    }
}
