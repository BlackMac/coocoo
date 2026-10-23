//! omp's statistics database (`~/.omp/stats.db`), read-only.
//!
//! omp fills it lazily from its session files, so a query first runs `omp stats --summary`
//! (at most once per [`SYNC_EVERY`], only when asked). Only the columns Coocoo needs are read; a
//! database without them is reported as an unknown schema instead of guessed at.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, params};

const SYNC_EVERY: Duration = Duration::from_secs(60);
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);

/// Columns read per table; checked before every query.
const NEEDED: &[(&str, &[&str])] = &[
    (
        "messages",
        &[
            "session_file",
            "model",
            "provider",
            "agent_type",
            "timestamp",
            "duration",
            "ttft",
            "stop_reason",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "cost_input",
            "cost_output",
            "cost_cache_read",
            "cost_cache_write",
            "cost_total",
        ],
    ),
    ("tool_calls", &["session_file", "tool_name", "model", "timestamp", "is_error"]),
    (
        "user_messages",
        &["session_file", "timestamp", "model", "yelling", "profanity", "anguish", "negation", "repetition", "blame", "prose"],
    ),
];

pub struct OmpStats {
    pub path: PathBuf,
    omp: PathBuf,
    /// Last sync: when (monotonic + wall clock) and its error.
    synced: tokio::sync::Mutex<Option<(Instant, i64, Option<String>)>>,
}

/// Result of a sync attempt.
pub struct Synced {
    pub at: Option<i64>,
    pub error: Option<String>,
}

impl OmpStats {
    pub fn new(omp: PathBuf) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Self { path: home.join(".omp").join("stats.db"), omp, synced: tokio::sync::Mutex::new(None) }
    }

    /// Bring the database up to date unless that happened in the last minute. Concurrent
    /// callers share one run.
    pub async fn sync(&self, force: bool) -> Synced {
        let mut last = self.synced.lock().await;
        if let Some((at, wall, error)) = last.as_ref()
            && !force
            && at.elapsed() < SYNC_EVERY
        {
            return Synced { at: Some(*wall), error: error.clone() };
        }
        let mut cmd = tokio::process::Command::new(&self.omp);
        cmd.args(["stats", "--summary"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let error = match tokio::time::timeout(SYNC_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => None,
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Some(format!("`omp stats` failed: {}", stderr.lines().last().unwrap_or("").trim()))
            }
            Ok(Err(err)) => Some(format!("cannot run `{}`: {err}", self.omp.display())),
            Err(_) => Some(format!("`omp stats` did not finish within {} s", SYNC_TIMEOUT.as_secs())),
        };
        let wall = coocoo_proto::util::now_ms();
        *last = Some((Instant::now(), wall, error.clone()));
        Synced { at: Some(wall), error }
    }

    /// Open the database read-only and check its schema.
    pub fn open(&self) -> Result<Db, String> {
        Db::open(&self.path)
    }
}

/// One assistant response.
pub struct MessageRow {
    pub session_file: String,
    pub model: String,
    pub provider: String,
    pub agent_type: String,
    pub ts: i64,
    /// ms (omp stores fractional milliseconds).
    pub duration: Option<f64>,
    pub ttft: Option<f64>,
    pub stop_reason: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_read: f64,
    pub cost_cache_write: f64,
    pub cost_total: f64,
}

pub struct ToolCallRow {
    pub session_file: String,
    pub name: String,
    pub model: String,
    pub ts: i64,
    pub error: bool,
}

pub struct UserRow {
    pub session_file: String,
    pub ts: i64,
    pub model: Option<String>,
    pub signals: crate::report::Signals,
    /// Only for flagged prompts (shortened).
    pub text: Option<String>,
}

pub struct Db {
    conn: Connection,
    /// `meta.rollup_version`.
    pub schema: Option<String>,
}

/// Prompt text kept for a flagged prompt.
const FLAGGED_CHARS: i64 = 400;

impl Db {
    fn open(path: &Path) -> Result<Db, String> {
        if !path.exists() {
            return Err(format!("{} does not exist (omp has not recorded any usage on this host)", path.display()));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let _ = conn.busy_timeout(Duration::from_secs(10));
        let schema: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'rollup_version'", [], |r| r.get(0))
            .ok();
        for (table, columns) in NEEDED {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).map_err(|e| e.to_string())?;
            let have: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?.flatten().collect();
            if let Some(missing) = columns.iter().find(|c| !have.iter().any(|h| h == *c)) {
                return Err(format!(
                    "unknown stats.db schema (rollup_version {}): {table}.{missing} is missing — Coocoo needs an update for this omp version",
                    schema.as_deref().unwrap_or("?")
                ));
            }
        }
        Ok(Db { conn, schema })
    }

    pub fn messages(&self, from: i64, to: i64, mut keep: impl FnMut(&str) -> bool) -> Result<Vec<MessageRow>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT session_file, model, provider, agent_type, timestamp, duration, ttft, stop_reason, \
                 input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, \
                 cost_input, cost_output, cost_cache_read, cost_cache_write, cost_total \
                 FROM messages WHERE timestamp >= ?1 AND timestamp < ?2",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![from, to]).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let session_file: String = r.get(0).map_err(|e| e.to_string())?;
            if !keep(&session_file) {
                continue;
            }
            let num = |i: usize| -> u64 { r.get::<_, Option<i64>>(i).ok().flatten().unwrap_or(0).max(0) as u64 };
            let cost = |i: usize| -> f64 { r.get::<_, Option<f64>>(i).ok().flatten().unwrap_or(0.0) };
            out.push(MessageRow {
                session_file,
                model: r.get(1).unwrap_or_default(),
                provider: r.get(2).unwrap_or_default(),
                agent_type: r.get(3).unwrap_or_default(),
                ts: r.get(4).unwrap_or_default(),
                duration: r.get::<_, Option<f64>>(5).ok().flatten(),
                ttft: r.get::<_, Option<f64>>(6).ok().flatten(),
                stop_reason: r.get(7).unwrap_or_default(),
                input: num(8),
                output: num(9),
                cache_read: num(10),
                cache_write: num(11),
                cost_input: cost(12),
                cost_output: cost(13),
                cost_cache_read: cost(14),
                cost_cache_write: cost(15),
                cost_total: cost(16),
            });
        }
        Ok(out)
    }

    pub fn tool_calls(&self, from: i64, to: i64, mut keep: impl FnMut(&str) -> bool) -> Result<Vec<ToolCallRow>, String> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT session_file, tool_name, model, timestamp, is_error FROM tool_calls WHERE timestamp >= ?1 AND timestamp < ?2")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![from, to]).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let session_file: String = r.get(0).map_err(|e| e.to_string())?;
            if !keep(&session_file) {
                continue;
            }
            out.push(ToolCallRow {
                session_file,
                name: r.get(1).unwrap_or_default(),
                model: r.get(2).unwrap_or_default(),
                ts: r.get(3).unwrap_or_default(),
                error: r.get::<_, Option<i64>>(4).ok().flatten().unwrap_or(0) != 0,
            });
        }
        Ok(out)
    }

    pub fn user_messages(&self, from: i64, to: i64, mut keep: impl FnMut(&str) -> bool) -> Result<Vec<UserRow>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT session_file, timestamp, model, yelling, profanity, anguish, negation, repetition, blame, \
                 CASE WHEN yelling + profanity + anguish + negation + repetition + blame > 0 THEN substr(prose, 1, ?3) END \
                 FROM user_messages WHERE timestamp >= ?1 AND timestamp < ?2",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![from, to, FLAGGED_CHARS]).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let session_file: String = r.get(0).map_err(|e| e.to_string())?;
            if !keep(&session_file) {
                continue;
            }
            let n = |i: usize| -> u64 { r.get::<_, Option<i64>>(i).ok().flatten().unwrap_or(0).max(0) as u64 };
            out.push(UserRow {
                session_file,
                ts: r.get(1).unwrap_or_default(),
                model: r.get(2).ok().flatten(),
                signals: crate::report::Signals { yelling: n(3), profanity: n(4), anguish: n(5), negation: n(6), repetition: n(7), blame: n(8) },
                text: r.get(9).ok().flatten(),
            });
        }
        Ok(out)
    }
}
