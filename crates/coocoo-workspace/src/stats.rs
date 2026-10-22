//! Host and per-process-tree resource sampling (`stats.watch`, `stats.get`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use coocoo_proto::error::{ok, params};
use coocoo_proto::util::now_ms;
use coocoo_proto::{ApiError, ApiResult, ConnSink, HostStats, ProcStat, events};
use serde::Deserialize;
use serde_json::{Value, json};
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::task::JoinHandle;

use crate::Stats;

const INTERVAL: Duration = Duration::from_secs(2);
/// A CPU baseline older than this is re-primed: usage would otherwise be a long-term average.
const STALE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct WatchState {
    conns: HashSet<u64>,
    task: Option<JoinHandle<()>>,
}

/// Lazily created `System`; dropped when nobody watches to keep the daemon small.
#[derive(Default)]
pub(crate) struct Sampler {
    sys: Option<System>,
    last: Option<Instant>,
}

impl Sampler {
    fn refresh(&mut self) -> &System {
        let sys = self.sys.get_or_insert_with(System::new);
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory().without_tasks(),
        );
        self.last = Some(Instant::now());
        sys
    }
}

#[derive(Deserialize)]
struct WatchParams {
    enabled: bool,
}

impl Stats {
    pub(crate) async fn handle(self: &Arc<Self>, method: &str, p: Value, conn: &ConnSink) -> ApiResult<Value> {
        match method {
            "stats.watch" => {
                let p: WatchParams = params(p)?;
                self.set_watch(conn.id, p.enabled, Some(self));
                ok(json!({}))
            }
            "stats.get" => {
                let stats = self.sample().await?;
                if self.watch.lock().conns.is_empty() {
                    *self.sampler.lock() = Sampler::default();
                }
                ok(stats)
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    /// Add/remove a watching connection; starts the sampling task on the first watcher and
    /// stops it (releasing the process table) when the last one leaves.
    pub(crate) fn set_watch(&self, conn_id: u64, enabled: bool, this: Option<&Arc<Self>>) {
        let mut watch = self.watch.lock();
        if enabled {
            watch.conns.insert(conn_id);
            if watch.task.is_none()
                && let Some(this) = this
            {
                watch.task = Some(tokio::spawn(watch_loop(Arc::downgrade(this))));
            }
        } else if watch.conns.remove(&conn_id) && watch.conns.is_empty() {
            if let Some(task) = watch.task.take() {
                task.abort();
            }
            *self.sampler.lock() = Sampler::default();
        }
    }

    /// One sample with a valid CPU baseline (primes and waits the minimum interval if needed).
    async fn sample(&self) -> ApiResult<HostStats> {
        let since = self.sampler.lock().last.map(|t| t.elapsed());
        match since {
            Some(elapsed) if elapsed <= STALE => {
                if let Some(rest) = MINIMUM_CPU_UPDATE_INTERVAL.checked_sub(elapsed) {
                    tokio::time::sleep(rest).await;
                }
            }
            _ => {
                let sampler = Arc::clone(&self.sampler);
                tokio::task::spawn_blocking(move || {
                    sampler.lock().refresh();
                })
                .await
                .map_err(ApiError::internal)?;
                tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL).await;
            }
        }
        let mut roots = (self.pids)();
        roots.push(("daemon".to_owned(), std::process::id()));
        let host_id = self.emitter.host_id().to_owned();
        let sampler = Arc::clone(&self.sampler);
        tokio::task::spawn_blocking(move || build(sampler.lock().refresh(), host_id, roots))
            .await
            .map_err(ApiError::internal)
    }
}

async fn watch_loop(stats: Weak<Stats>) {
    loop {
        let Some(this) = stats.upgrade() else { return };
        match this.sample().await {
            Ok(sample) => this.emitter.emit(events::HOST_STATS, &sample),
            Err(err) => tracing::debug!("stats sample failed: {err}"),
        }
        drop(this);
        tokio::time::sleep(INTERVAL).await;
    }
}

fn build(sys: &System, host_id: String, roots: Vec<(String, u32)>) -> HostStats {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, proc_) in sys.processes() {
        if let Some(parent) = proc_.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let procs = roots
        .into_iter()
        .filter_map(|(key, pid)| {
            let root = Pid::from_u32(pid);
            sys.process(root)?;
            let (mut rss, mut cpu) = (0u64, 0f32);
            let mut seen = HashSet::new();
            let mut stack = vec![root];
            while let Some(pid) = stack.pop() {
                if !seen.insert(pid) {
                    continue;
                }
                if let Some(p) = sys.process(pid) {
                    rss += p.memory();
                    cpu += p.cpu_usage();
                }
                if let Some(kids) = children.get(&pid) {
                    stack.extend_from_slice(kids);
                }
            }
            Some(ProcStat { key, pid, rss, cpu })
        })
        .collect();
    let load = System::load_average();
    HostStats {
        host_id,
        ts: now_ms(),
        cpu: sys.global_cpu_usage(),
        cpu_count: sys.cpus().len() as u32,
        mem_used: sys.used_memory(),
        mem_total: sys.total_memory(),
        load: [load.one, load.five, load.fifteen],
        procs,
    }
}
