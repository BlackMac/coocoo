//! One browser profile: its Chromium process, CDP connection, tabs, screencasts and consoles.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use coocoo_proto::bus::encode_event;
use coocoo_proto::util::now_ms;
use coocoo_proto::{ApiError, ApiResult, BrowserConsoleEntry, BrowserProfileInfo, BrowserTab, ConnSink, events};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use crate::Shared;
use crate::cdp::{Cdp, CdpEvent};
use crate::chrome::{self, Discovery};
use crate::limiter::FpsLimiter;
use crate::stream::{console_entry, frame_envelope, is_base64};
use crate::tabs::{TabList, TargetInfo};

const CONSOLE_CAP: usize = 500;
const UPDATE_DEBOUNCE: Duration = Duration::from_millis(150);
/// How long to wait for a screencast frame after a viewport change before forcing one.
const FRAME_KICK_AFTER: Duration = Duration::from_millis(400);

/// A live Chromium process with its browser-level CDP connection.
pub(crate) struct Running {
    generation: u64,
    pub pid: u32,
    pub port: u16,
    pub cdp: Arc<Cdp>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    exited: watch::Receiver<bool>,
}

impl Running {
    fn kill(&self) {
        if let Some(tx) = self.kill.lock().take() {
            let _ = tx.send(());
        }
    }

    async fn wait_exit(&self, timeout: Duration) -> bool {
        let mut exited = self.exited.clone();
        tokio::time::timeout(timeout, exited.wait_for(|e| *e)).await.is_ok()
    }
}

/// Screencast parameters requested by one watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreencastParams {
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub quality: u32,
}

impl ScreencastParams {
    /// `Page.startScreencast` parameters.
    fn request(self) -> Value {
        let mut req = json!({ "format": "jpeg", "quality": self.quality, "everyNthFrame": 1 });
        if let Some(w) = self.max_width {
            req["maxWidth"] = json!(w);
        }
        if let Some(h) = self.max_height {
            req["maxHeight"] = json!(h);
        }
        req
    }

    /// Shared screencast satisfying every watcher: the largest requested bounds and quality.
    fn merge(self, other: Self) -> Self {
        let max = |a: Option<u32>, b: Option<u32>| match (a, b) {
            (Some(a), Some(b)) => Some(a.max(b)),
            // One watcher wants native size.
            _ => None,
        };
        Self {
            max_width: max(self.max_width, other.max_width),
            max_height: max(self.max_height, other.max_height),
            quality: self.quality.max(other.quality),
        }
    }
}

struct Frame {
    data: String,
    width: u32,
    height: u32,
}

/// Shared screencast + console subscription of one tab.
struct Watch {
    session_id: String,
    watchers: HashMap<u64, (ConnSink, ScreencastParams)>,
    applied: Option<ScreencastParams>,
    limiter: FpsLimiter,
    seq: u64,
    /// Newest frame held back by the fps cap.
    held: Option<Frame>,
    flush_scheduled: bool,
    /// Last delivered envelope, replayed to watchers that join later.
    last: Option<Arc<str>>,
}

impl Watch {
    fn merged_params(&self) -> Option<ScreencastParams> {
        self.watchers.values().map(|(_, p)| *p).reduce(ScreencastParams::merge)
    }
}

#[derive(Default)]
struct State {
    run: Option<Arc<Running>>,
    tabs: TabList,
    /// target id → flattened session id held by us.
    sessions: HashMap<String, String>,
    /// Sessions with `DOM`/`Overlay` enabled (design mode).
    design_sessions: HashSet<String>,
    watches: HashMap<String, Watch>,
    consoles: HashMap<String, VecDeque<BrowserConsoleEntry>>,
    last_activity: Option<Instant>,
}

impl State {
    fn current(&self) -> Option<Arc<Running>> {
        self.run.clone()
    }

    fn is_generation(&self, generation: u64) -> bool {
        self.run.as_ref().is_some_and(|r| r.generation == generation)
    }

    /// Forget everything tied to the (gone) process.
    fn reset(&mut self) -> Option<Arc<Running>> {
        self.tabs.clear();
        self.sessions.clear();
        self.design_sessions.clear();
        self.watches.clear();
        self.consoles.clear();
        self.run.take()
    }

    /// Stop a tab's screencast and drop our session on it.
    fn teardown_watch(&mut self, target_id: &str) {
        let Some(watch) = self.watches.remove(target_id) else { return };
        self.design_sessions.remove(&watch.session_id);
        if self.sessions.get(target_id) == Some(&watch.session_id) {
            self.sessions.remove(target_id);
        }
        if let Some(run) = &self.run {
            run.cdp.send("Page.stopScreencast", json!({}), Some(&watch.session_id));
            run.cdp.send("Target.detachFromTarget", json!({ "sessionId": watch.session_id }), None);
        }
    }

    fn remove_watcher(&mut self, target_id: &str, conn_id: u64) {
        let Some(watch) = self.watches.get_mut(target_id) else { return };
        watch.watchers.remove(&conn_id);
        if watch.watchers.is_empty() {
            self.teardown_watch(target_id);
        }
    }

    fn target_of_session(&self, session_id: &str) -> Option<String> {
        self.sessions.iter().find(|(_, s)| *s == session_id).map(|(t, _)| t.clone())
    }
}

pub(crate) struct Profile {
    pub id: String,
    shared: Arc<Shared>,
    user_data_dir: PathBuf,
    start_lock: tokio::sync::Mutex<()>,
    attach_lock: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    update_scheduled: AtomicBool,
}

impl Profile {
    pub fn new(id: String, shared: Arc<Shared>) -> Arc<Self> {
        let user_data_dir = shared.root.join(&id);
        Arc::new(Self {
            id,
            shared,
            user_data_dir,
            start_lock: tokio::sync::Mutex::new(()),
            attach_lock: tokio::sync::Mutex::new(()),
            state: Mutex::new(State::default()),
            update_scheduled: AtomicBool::new(false),
        })
    }

    pub fn info(&self) -> BrowserProfileInfo {
        let cdp_url = self.shared.cdp_url(&self.id);
        let st = self.state.lock();
        BrowserProfileInfo {
            id: self.id.clone(),
            running: st.run.is_some(),
            pid: st.run.as_ref().map(|r| r.pid),
            tabs: st.tabs.snapshot(),
            cdp_url,
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.state.lock().run.as_ref().map(|r| r.pid)
    }

    pub fn touch(&self) {
        self.state.lock().last_activity = Some(Instant::now());
    }

    fn emit_update(&self) {
        self.shared.emitter.emit(events::BROWSER_UPDATED, &self.info());
    }

    /// Coalesce bursts of target events into one `browser.updated`.
    fn schedule_update(self: &Arc<Self>) {
        if self.update_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(UPDATE_DEBOUNCE).await;
            if let Some(p) = weak.upgrade() {
                p.update_scheduled.store(false, Ordering::Release);
                p.emit_update();
            }
        });
    }

    pub fn current(&self) -> Option<Arc<Running>> {
        self.state.lock().current()
    }

    /// The running process, or `conflict` when the profile is stopped.
    pub fn running(&self) -> ApiResult<Arc<Running>> {
        self.current().ok_or_else(|| ApiError::conflict(format!("browser profile `{}` is not running", self.id)))
    }

    // ── lifecycle ────────────────────────────────────────────────────────────────────────────

    pub async fn ensure_running(self: &Arc<Self>) -> ApiResult<Arc<Running>> {
        if let Some(run) = self.current() {
            return Ok(run);
        }
        let _guard = self.start_lock.lock().await;
        if let Some(run) = self.current() {
            return Ok(run);
        }
        if self.shared.shutting_down.load(Ordering::Acquire) {
            return Err(ApiError::unavailable("daemon is shutting down"));
        }
        let run = self.launch().await?;
        self.emit_update();
        Ok(run)
    }

    async fn launch(self: &Arc<Self>) -> ApiResult<Arc<Running>> {
        let config = &self.shared.config;
        let exe = Discovery::from_env(config.chrome_path.clone()).find()?;
        let launched = chrome::launch(&exe, &self.user_data_dir, config.headless).await?;
        let generation = self.shared.next_generation();
        let pid = launched.pid;
        tracing::info!(profile = %self.id, pid, port = launched.port, exe = %exe.display(), "Chromium started");

        let (kill_tx, kill_rx) = oneshot::channel();
        let (exit_tx, exit_rx) = watch::channel(false);
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let status = chrome::supervise(launched.child, pid, kill_rx).await;
            let _ = exit_tx.send(true);
            if let Some(p) = weak.upgrade() {
                p.on_exit(generation, status);
            }
        });

        let on_event = {
            let weak = Arc::downgrade(self);
            move |cdp: &Arc<Cdp>, ev: CdpEvent| {
                if let Some(p) = weak.upgrade() {
                    p.on_event(generation, cdp, ev);
                }
            }
        };
        let on_close = {
            let weak = Arc::downgrade(self);
            move || {
                // Socket gone while the process may live on: kill it; the supervisor cleans up.
                if let Some(p) = weak.upgrade()
                    && let Some(run) = p.current().filter(|r| r.generation == generation)
                {
                    run.kill();
                }
            }
        };
        let cdp = match Cdp::connect(&launched.ws_url, on_event, on_close).await {
            Ok(cdp) => cdp,
            Err(err) => {
                let _ = kill_tx.send(());
                return Err(err);
            }
        };
        let run = Arc::new(Running {
            generation,
            pid,
            port: launched.port,
            cdp,
            kill: Mutex::new(Some(kill_tx)),
            exited: exit_rx,
        });
        {
            let mut st = self.state.lock();
            st.reset();
            st.run = Some(run.clone());
            st.last_activity = Some(Instant::now());
        }
        // Reports every existing target before the response, then keeps the tab list current.
        if let Err(err) = run.cdp.call("Target.setDiscoverTargets", json!({ "discover": true }), None).await {
            self.stop_locked().await;
            return Err(err);
        }
        let idle = Duration::from_secs(self.shared.config.idle_shutdown_secs);
        if !idle.is_zero() {
            tokio::spawn(idle_loop(Arc::downgrade(self), generation, idle));
        }
        Ok(run)
    }

    pub async fn stop(&self) {
        let _guard = self.start_lock.lock().await;
        self.stop_locked().await;
    }

    async fn stop_locked(&self) {
        let Some(run) = self.state.lock().reset() else { return };
        // Graceful close lets Chromium flush cookies/storage of the profile.
        let _ = run.cdp.call_with_timeout("Browser.close", json!({}), None, Duration::from_secs(2)).await;
        if !run.wait_exit(Duration::from_secs(3)).await {
            run.kill();
            run.wait_exit(Duration::from_secs(6)).await;
        }
        tracing::info!(profile = %self.id, pid = run.pid, "Chromium stopped");
        self.emit_update();
    }

    fn on_exit(&self, generation: u64, status: Option<std::process::ExitStatus>) {
        let gone = {
            let mut st = self.state.lock();
            if !st.is_generation(generation) {
                return;
            }
            st.reset()
        };
        if let Some(run) = gone {
            tracing::warn!(profile = %self.id, pid = run.pid, ?status, "Chromium exited unexpectedly");
            self.emit_update();
        }
    }

    // ── CDP events ───────────────────────────────────────────────────────────────────────────

    fn on_event(self: &Arc<Self>, generation: u64, cdp: &Arc<Cdp>, mut ev: CdpEvent) {
        let mut st = self.state.lock();
        if !st.is_generation(generation) {
            return;
        }
        match ev.method.as_str() {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let Ok(info) = serde_json::from_value::<TargetInfo>(ev.params["targetInfo"].take()) else { return };
                if info.kind == "page" {
                    st.last_activity = Some(Instant::now());
                }
                if st.tabs.upsert(info) {
                    drop(st);
                    self.schedule_update();
                }
            }
            "Target.targetDestroyed" => {
                let Some(target_id) = ev.params["targetId"].as_str() else { return };
                st.last_activity = Some(Instant::now());
                st.watches.remove(target_id);
                if let Some(session) = st.sessions.remove(target_id) {
                    st.design_sessions.remove(&session);
                }
                st.consoles.remove(target_id);
                if st.tabs.remove(target_id) {
                    drop(st);
                    self.schedule_update();
                }
            }
            "Target.targetCrashed" => {
                let Some(target_id) = ev.params["targetId"].as_str().map(str::to_owned) else { return };
                let status = ev.params["status"].as_str().unwrap_or("crashed");
                self.push_console(&mut st, &target_id, "error", format!("Page renderer {status}"));
            }
            "Target.detachedFromTarget" => {
                let Some(session) = ev.params["sessionId"].as_str() else { return };
                st.design_sessions.remove(session);
                if let Some(target_id) = st.target_of_session(session) {
                    st.sessions.remove(&target_id);
                    if st.watches.get(&target_id).is_some_and(|w| w.session_id == session) {
                        st.watches.remove(&target_id);
                    }
                }
            }
            "Page.screencastFrame" => {
                let Some(session) = ev.session_id.as_deref() else { return };
                let ack = ev.params["sessionId"].clone();
                cdp.send("Page.screencastFrameAck", json!({ "sessionId": ack }), Some(session));
                let Some(target_id) = st.target_of_session(session) else { return };
                let Value::String(data) = ev.params["data"].take() else { return };
                let meta = &ev.params["metadata"];
                let frame = Frame {
                    data,
                    width: meta["deviceWidth"].as_f64().unwrap_or(0.0).round() as u32,
                    height: meta["deviceHeight"].as_f64().unwrap_or(0.0).round() as u32,
                };
                self.on_frame(&mut st, &target_id, frame);
            }
            method => {
                let Some(session) = ev.session_id.as_deref() else { return };
                let Some((level, text)) = console_entry(method, &ev.params) else { return };
                let Some(target_id) = st.target_of_session(session) else { return };
                self.push_console(&mut st, &target_id, level, text);
            }
        }
    }

    fn push_console(&self, st: &mut State, target_id: &str, level: &str, text: String) {
        let entry = BrowserConsoleEntry {
            profile: self.id.clone(),
            target_id: target_id.to_owned(),
            level: level.to_owned(),
            text,
            ts: now_ms(),
        };
        if let Some(watch) = st.watches.get(target_id)
            && let Some(json) = encode_event(events::BROWSER_CONSOLE, self.shared.emitter.host_id(), &entry)
        {
            for (sink, _) in watch.watchers.values() {
                sink.try_send(json.clone());
            }
        }
        let ring = st.consoles.entry(target_id.to_owned()).or_default();
        if ring.len() == CONSOLE_CAP {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    fn on_frame(self: &Arc<Self>, st: &mut State, target_id: &str, frame: Frame) {
        if !is_base64(&frame.data) {
            tracing::debug!("dropping screencast frame with non-base64 payload");
            return;
        }
        let Some(watch) = st.watches.get_mut(target_id) else { return };
        let now = Instant::now();
        match watch.limiter.check(now) {
            None => self.deliver(st, target_id, frame),
            Some(wait) => {
                watch.held = Some(frame);
                if !watch.flush_scheduled {
                    watch.flush_scheduled = true;
                    let (weak, target_id, session) =
                        (Arc::downgrade(self), target_id.to_owned(), watch.session_id.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(wait).await;
                        let Some(p) = weak.upgrade() else { return };
                        let mut st = p.state.lock();
                        let Some(watch) = st.watches.get_mut(&target_id).filter(|w| w.session_id == session) else {
                            return;
                        };
                        watch.flush_scheduled = false;
                        if let Some(frame) = watch.held.take() {
                            watch.limiter.mark_sent(Instant::now());
                            p.deliver(&mut st, &target_id, frame);
                        }
                    });
                }
            }
        }
    }

    /// Fan one frame out to every watcher of the tab; prune watchers whose connection is gone.
    fn deliver(&self, st: &mut State, target_id: &str, frame: Frame) {
        let Some(watch) = st.watches.get_mut(target_id) else { return };
        watch.seq += 1;
        let json = frame_envelope(
            self.shared.emitter.host_id(),
            &self.id,
            target_id,
            &frame.data,
            frame.width,
            frame.height,
            watch.seq,
        );
        // A full queue just drops this frame for that client (slow link → lower frame rate).
        watch.watchers.retain(|_, (sink, _)| sink.try_send(json.clone()) || !sink.is_closed());
        watch.last = Some(json);
        if watch.watchers.is_empty() {
            st.teardown_watch(target_id);
        }
    }

    // ── per-tab operations ───────────────────────────────────────────────────────────────────

    /// Our flattened CDP session on a tab, attached on first use.
    pub async fn session(self: &Arc<Self>, target_id: &str) -> ApiResult<(Arc<Running>, String)> {
        let run = self.running()?;
        let lookup = |st: &State| -> ApiResult<Option<String>> {
            if !st.tabs.contains(target_id) {
                return Err(ApiError::not_found(format!("tab {target_id}")));
            }
            Ok(st.sessions.get(target_id).cloned())
        };
        self.touch();
        if let Some(session) = lookup(&self.state.lock())? {
            return Ok((run, session));
        }
        let _guard = self.attach_lock.lock().await;
        if let Some(session) = lookup(&self.state.lock())? {
            return Ok((run, session));
        }
        let res =
            run.cdp.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), None).await?;
        let session = res["sessionId"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Target.attachToTarget returned no sessionId"))?
            .to_owned();
        let mut st = self.state.lock();
        if !st.is_generation(run.generation) {
            return Err(ApiError::unavailable("browser restarted"));
        }
        st.sessions.insert(target_id.to_owned(), session.clone());
        Ok((run, session))
    }

    pub async fn watch(self: &Arc<Self>, conn: &ConnSink, target_id: &str, params: ScreencastParams) -> ApiResult<()> {
        let (run, session) = self.session(target_id).await?;
        let (fresh, restart) = {
            let mut st = self.state.lock();
            let max_fps = self.shared.config.max_fps;
            let watch = st.watches.entry(target_id.to_owned()).or_insert_with(|| Watch {
                session_id: session.clone(),
                watchers: HashMap::new(),
                applied: None,
                limiter: FpsLimiter::new(max_fps),
                seq: 0,
                held: None,
                flush_scheduled: false,
                last: None,
            });
            watch.watchers.insert(conn.id, (conn.clone(), params));
            if let Some(last) = &watch.last {
                conn.try_send(last.clone());
            }
            let fresh = watch.applied.is_none();
            let merged = watch.merged_params();
            let restart = (merged != watch.applied).then_some(merged).flatten();
            if restart.is_some() {
                watch.applied = restart;
            }
            if fresh {
                // `Runtime.enable` / `Log.enable` replay buffered messages; start the ring clean.
                st.consoles.remove(target_id);
            }
            (fresh, restart)
        };
        let s = Some(session.as_str());
        if fresh {
            run.cdp.call("Page.enable", json!({}), s).await?;
            run.cdp.call("Runtime.enable", json!({}), s).await?;
            run.cdp.call("Log.enable", json!({}), s).await?;
        }
        if let Some(p) = restart {
            run.cdp.call("Page.startScreencast", p.request(), s).await?;
        }
        Ok(())
    }

    /// Make sure watchers see the tab after a viewport change.
    ///
    /// Chromium occasionally produces no screencast frame for a viewport-only change of a
    /// static page (more often under CPU load). Re-issuing `Page.startScreencast` with the
    /// current parameters forces one; it is only sent when no frame arrived in time.
    pub fn expect_frame(self: &Arc<Self>, target_id: &str) {
        let seq = {
            let st = self.state.lock();
            match st.watches.get(target_id) {
                Some(w) => w.seq,
                None => return,
            }
        };
        let (weak, target_id) = (Arc::downgrade(self), target_id.to_owned());
        tokio::spawn(async move {
            for wait in [FRAME_KICK_AFTER, FRAME_KICK_AFTER * 2] {
                tokio::time::sleep(wait).await;
                let Some(p) = weak.upgrade() else { return };
                let st = p.state.lock();
                let Some(w) = st.watches.get(&target_id) else { return };
                if w.seq != seq || w.held.is_some() {
                    return;
                }
                let (Some(run), Some(params)) = (st.run.as_ref(), w.applied) else { return };
                run.cdp.send("Page.startScreencast", params.request(), Some(&w.session_id));
            }
        });
    }

    pub fn unwatch(&self, conn_id: u64, target_id: &str) {
        self.state.lock().remove_watcher(target_id, conn_id);
    }

    pub fn conn_closed(&self, conn_id: u64) {
        let mut st = self.state.lock();
        let targets: Vec<String> =
            st.watches.iter().filter(|(_, w)| w.watchers.contains_key(&conn_id)).map(|(t, _)| t.clone()).collect();
        for target_id in targets {
            st.remove_watcher(&target_id, conn_id);
        }
    }

    pub fn console(&self, target_id: &str) -> Vec<BrowserConsoleEntry> {
        self.state.lock().consoles.get(target_id).map(|ring| ring.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn tab(&self, target_id: &str) -> Option<BrowserTab> {
        self.state.lock().tabs.get(target_id)
    }

    pub fn tab_count(&self) -> usize {
        self.state.lock().tabs.len()
    }

    pub fn sole_blank_tab(&self) -> Option<String> {
        self.state.lock().tabs.sole_blank_tab().map(str::to_owned)
    }

    /// Mark a tab active (activation, open). Seeds the tab when the target event is still in flight.
    pub fn activate_tab(self: &Arc<Self>, target_id: &str, url: Option<&str>) {
        {
            let mut st = self.state.lock();
            if !st.tabs.contains(target_id)
                && let Some(url) = url
            {
                st.tabs.upsert(TargetInfo {
                    target_id: target_id.to_owned(),
                    kind: "page".to_owned(),
                    title: String::new(),
                    url: url.to_owned(),
                });
            }
            st.tabs.touch(target_id);
        }
        self.schedule_update();
    }

    /// Enable `DOM` + `Overlay` on the session once (design mode).
    pub async fn ensure_design(&self, run: &Running, session: &str) -> ApiResult<()> {
        if self.state.lock().design_sessions.contains(session) {
            return Ok(());
        }
        run.cdp.call("DOM.enable", json!({}), Some(session)).await?;
        run.cdp.call("Overlay.enable", json!({}), Some(session)).await?;
        self.state.lock().design_sessions.insert(session.to_owned());
        Ok(())
    }

    pub fn design_enabled(&self, session: &str) -> bool {
        self.state.lock().design_sessions.contains(session)
    }

    // ── idle shutdown ────────────────────────────────────────────────────────────────────────

    /// Stop the profile when it is still idle; returns `false` when something kept it alive.
    async fn stop_if_idle(&self, generation: u64, idle: Duration) -> bool {
        let _guard = self.start_lock.lock().await;
        let (run, sessions) = {
            let mut st = self.state.lock();
            if !st.is_generation(generation) {
                return true;
            }
            let quiet = st.last_activity.is_none_or(|t| t.elapsed() >= idle);
            if !st.watches.is_empty() || !quiet {
                return false;
            }
            st.design_sessions.clear();
            (st.current(), std::mem::take(&mut st.sessions))
        };
        let Some(run) = run else { return true };
        // Our leftover control sessions would count as clients below; they are idle anyway.
        for session in sessions.values() {
            let _ = run.cdp.call("Target.detachFromTarget", json!({ "sessionId": session }), None).await;
        }
        // An attached page means another CDP client (an omp agent) is still driving the browser.
        if let Ok(res) = run.cdp.call("Target.getTargets", json!({}), None).await
            && res["targetInfos"]
                .as_array()
                .is_some_and(|t| t.iter().any(|i| i["type"] == "page" && i["attached"] == true))
        {
            self.touch();
            return false;
        }
        tracing::info!(profile = %self.id, "stopping idle Chromium");
        self.stop_locked().await;
        true
    }
}

/// One low-frequency timer per running profile: wakes when the idle deadline could have passed.
async fn idle_loop(weak: Weak<Profile>, generation: u64, idle: Duration) {
    loop {
        let deadline = {
            let Some(p) = weak.upgrade() else { return };
            let st = p.state.lock();
            if !st.is_generation(generation) {
                return;
            }
            st.last_activity.unwrap_or_else(Instant::now) + idle
        };
        tokio::time::sleep_until(deadline).await;
        let Some(p) = weak.upgrade() else { return };
        {
            let mut st = p.state.lock();
            if !st.is_generation(generation) {
                return;
            }
            if !st.watches.is_empty() {
                st.last_activity = Some(Instant::now());
                continue;
            }
            if st.last_activity.is_some_and(|t| t.elapsed() < idle) {
                continue;
            }
        }
        if p.stop_if_idle(generation, idle).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screencast_params_merge() {
        let a = ScreencastParams { max_width: Some(800), max_height: Some(600), quality: 50 };
        let b = ScreencastParams { max_width: Some(1280), max_height: Some(400), quality: 70 };
        assert_eq!(a.merge(b), ScreencastParams { max_width: Some(1280), max_height: Some(600), quality: 70 });
        let native = ScreencastParams { max_width: None, max_height: None, quality: 10 };
        assert_eq!(a.merge(native), ScreencastParams { max_width: None, max_height: None, quality: 50 });
    }
}
