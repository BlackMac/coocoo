//! Agent ledger: `<data_dir>/insights/ledger.jsonl`, one [`LedgerEntry`] per change of an
//! agent's identity (created, renamed, new session file, deleted). Deleting an agent removes its
//! data directory; the ledger keeps what the statistics need to still count it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use coocoo_proto::insights::LedgerEntry;
use parking_lot::Mutex;

/// What the ledger knows about one agent (all its entries folded).
#[derive(Debug, Clone)]
pub struct LedgerAgent {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub cwd: String,
    pub worktree: Option<String>,
    /// Every session file it ever had, oldest first.
    pub session_files: Vec<String>,
    pub parent_id: Option<String>,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl LedgerAgent {
    fn from_entry(e: &LedgerEntry) -> Self {
        Self {
            id: e.agent_id.clone(),
            title: e.title.clone(),
            project_id: e.project_id.clone(),
            cwd: e.cwd.clone(),
            worktree: e.worktree.clone(),
            session_files: e.session_file.iter().cloned().collect(),
            parent_id: e.parent_id.clone(),
            created_at: e.created_at,
            deleted_at: e.deleted_at,
        }
    }

    /// Fold `e` in; returns whether anything changed.
    fn apply(&mut self, e: &LedgerEntry) -> bool {
        let mut changed = false;
        let mut set = |field: &mut String, value: &String| {
            if field != value {
                *field = value.clone();
                changed = true;
            }
        };
        set(&mut self.title, &e.title);
        set(&mut self.cwd, &e.cwd);
        if e.project_id.is_some() && self.project_id != e.project_id {
            self.project_id = e.project_id.clone();
            changed = true;
        }
        if e.worktree.is_some() && self.worktree != e.worktree {
            self.worktree = e.worktree.clone();
            changed = true;
        }
        if let Some(file) = &e.session_file
            && !self.session_files.contains(file)
        {
            self.session_files.push(file.clone());
            changed = true;
        }
        if e.deleted_at.is_some() && self.deleted_at.is_none() {
            self.deleted_at = e.deleted_at;
            changed = true;
        }
        changed
    }
}

pub struct Ledger {
    path: PathBuf,
    agents: Mutex<HashMap<String, LedgerAgent>>,
}

impl Ledger {
    pub fn open(path: PathBuf) -> Self {
        let mut agents: HashMap<String, LedgerAgent> = HashMap::new();
        if let Ok(file) = std::fs::File::open(&path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                let Ok(entry) = serde_json::from_str::<LedgerEntry>(&line) else { continue };
                match agents.get_mut(&entry.agent_id) {
                    Some(agent) => {
                        agent.apply(&entry);
                    }
                    None => {
                        agents.insert(entry.agent_id.clone(), LedgerAgent::from_entry(&entry));
                    }
                }
            }
        }
        Self { path, agents: Mutex::new(agents) }
    }

    /// Whether `entry` would change what the ledger knows (cheap, no I/O).
    pub fn changes(&self, entry: &LedgerEntry) -> bool {
        match self.agents.lock().get(&entry.agent_id) {
            Some(agent) => agent.clone().apply(entry),
            None => true,
        }
    }

    /// Fold an entry in and append it when it changes something (blocking, small).
    pub fn record(&self, entry: LedgerEntry) {
        let mut agents = self.agents.lock();
        let changed = match agents.get_mut(&entry.agent_id) {
            Some(agent) => agent.apply(&entry),
            None => {
                agents.insert(entry.agent_id.clone(), LedgerAgent::from_entry(&entry));
                true
            }
        };
        if !changed {
            return;
        }
        let Ok(mut line) = serde_json::to_string(&entry) else { return };
        line.push('\n');
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let written = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(err) = written {
            tracing::warn!(file = %self.path.display(), %err, "agent ledger write failed");
        }
    }

    pub fn agents(&self) -> Vec<LedgerAgent> {
        self.agents.lock().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, title: &str, session: Option<&str>) -> LedgerEntry {
        LedgerEntry {
            ts: 1,
            agent_id: id.into(),
            title: title.into(),
            project_id: Some("p".into()),
            cwd: "/w".into(),
            worktree: None,
            session_file: session.map(Into::into),
            parent_id: None,
            created_at: 1,
            deleted_at: None,
        }
    }

    #[test]
    fn deleted_agents_keep_every_session_file() {
        let path = std::env::temp_dir().join(format!("coocoo-ledger-{}", coocoo_proto::util::new_id())).join("ledger.jsonl");
        let ledger = Ledger::open(path.clone());
        ledger.record(entry("a", "First", Some("/s/1.jsonl")));
        ledger.record(entry("a", "First", Some("/s/1.jsonl")));
        ledger.record(entry("a", "Renamed", Some("/s/2.jsonl")));
        ledger.record(LedgerEntry { deleted_at: Some(9), ..entry("a", "Renamed", None) });
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3, "unchanged entries are not appended");

        let reopened = Ledger::open(path);
        let a = &reopened.agents()[0];
        assert_eq!((a.title.as_str(), a.session_files.len(), a.deleted_at), ("Renamed", 2, Some(9)));
    }
}
