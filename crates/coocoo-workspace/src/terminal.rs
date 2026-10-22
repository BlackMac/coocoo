//! Terminals: login shells on pseudo-terminals (`terminal.*`), streamed to the connections
//! attached to them.
//!
//! A terminal outlives its viewers: detaching (closing the panel, a dropped connection) leaves the
//! shell running, and attaching again replays the recent output. It ends when the shell exits or
//! on `terminal.kill`; when the daemon exits, closing the PTY master hangs the shell up.

use std::collections::HashMap;
use std::sync::Arc;

use coocoo_proto::error::{ok, params};
use coocoo_proto::{ApiError, ApiResult, ConnSink, Emitter, TerminalInfo};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};

/// Recent output kept per terminal for re-attaching viewers.
#[cfg(unix)]
const SCROLLBACK: usize = 256 * 1024;
/// Largest `terminal.input` payload (a big paste); keeps one request from stalling the PTY.
const MAX_INPUT: usize = 256 * 1024;
/// A viewer whose connection does not take output for this long is detached: one stalled
/// phone must not freeze the shell for everyone else.
#[cfg(unix)]
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct Terminals {
    #[cfg_attr(not(unix), allow(dead_code))]
    emitter: Emitter,
    #[cfg_attr(not(unix), allow(dead_code))]
    terms: Mutex<HashMap<String, Arc<imp::Term>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenParams {
    cwd: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachParams {
    id: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

#[derive(Deserialize)]
struct IdParams {
    id: String,
}

#[derive(Deserialize)]
struct InputParams {
    id: String,
    data: String,
}

#[derive(Deserialize)]
struct ResizeParams {
    id: String,
    cols: u16,
    rows: u16,
}

fn size(cols: Option<u16>, rows: Option<u16>) -> (u16, u16) {
    (cols.unwrap_or(80).clamp(2, 1000), rows.unwrap_or(24).clamp(1, 500))
}

impl Terminals {
    pub fn new(emitter: Emitter) -> Arc<Self> {
        Arc::new(Self { emitter, terms: Mutex::new(HashMap::new()) })
    }

    /// `terminal.*`. Output of attached terminals goes to `conn`.
    pub async fn dispatch(self: &Arc<Self>, method: &str, p: Value, conn: &ConnSink) -> ApiResult<Value> {
        match method {
            "terminal.list" => {
                let mut list: Vec<TerminalInfo> = self.terms.lock().values().map(|t| t.info()).collect();
                list.sort_by_key(|t| t.created_at);
                ok(list)
            }
            "terminal.open" => {
                let OpenParams { cwd, cols, rows } = params(p)?;
                if !std::path::Path::new(&cwd).is_dir() {
                    return Err(ApiError::invalid(format!("{cwd} is not a directory")));
                }
                let (cols, rows) = size(cols, rows);
                ok(self.open(cwd, cols, rows)?)
            }
            "terminal.attach" => {
                let AttachParams { id, cols, rows } = params(p)?;
                let term = self.get(&id)?;
                if cols.is_some() || rows.is_some() {
                    let (cols, rows) = size(cols, rows);
                    term.resize(cols, rows)?;
                }
                Ok(json!({ "data": term.attach(conn) }))
            }
            "terminal.detach" => {
                let IdParams { id } = params(p)?;
                if let Some(term) = self.terms.lock().get(&id) {
                    term.detach(conn.id);
                }
                Ok(json!({}))
            }
            "terminal.input" => {
                let InputParams { id, data } = params(p)?;
                if data.len() > MAX_INPUT {
                    return Err(ApiError::invalid(format!("input larger than {MAX_INPUT} bytes")));
                }
                self.get(&id)?.write(data.as_bytes()).await?;
                Ok(json!({}))
            }
            "terminal.resize" => {
                let ResizeParams { id, cols, rows } = params(p)?;
                let (cols, rows) = size(Some(cols), Some(rows));
                self.get(&id)?.resize(cols, rows)?;
                Ok(json!({}))
            }
            "terminal.kill" => {
                let IdParams { id } = params(p)?;
                self.get(&id)?.hang_up();
                Ok(json!({}))
            }
            _ => Err(ApiError::unknown_method(method)),
        }
    }

    /// Detach the connection from every terminal (the shells keep running).
    pub fn conn_closed(&self, conn_id: u64) {
        for term in self.terms.lock().values() {
            term.detach(conn_id);
        }
    }

    fn get(&self, id: &str) -> ApiResult<Arc<imp::Term>> {
        self.terms.lock().get(id).cloned().ok_or_else(|| ApiError::not_found(format_args!("terminal {id}")))
    }
}

#[cfg(unix)]
mod imp {
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::Stdio;
    use std::sync::{Arc, Weak};

    use coocoo_proto::util::{new_id, now_ms};
    use coocoo_proto::{ApiError, ApiResult, ConnSink, TerminalInfo, events};
    use parking_lot::Mutex;
    use serde_json::json;
    use tokio::io::unix::AsyncFd;

    use super::{SCROLLBACK, SEND_TIMEOUT, Terminals};

    pub(crate) struct Term {
        info: Mutex<TerminalInfo>,
        master: AsyncFd<OwnedFd>,
        /// Shell pid = its process group and session (`setsid`).
        pid: i32,
        /// Serializes writers so concurrent `terminal.input` calls do not interleave.
        write_lock: tokio::sync::Mutex<()>,
        state: Mutex<State>,
    }

    #[derive(Default)]
    struct State {
        scrollback: VecDeque<u8>,
        /// Whether the scrollback dropped its oldest bytes (it may start mid-line).
        trimmed: bool,
        /// Incomplete UTF-8 sequence at the end of the last read.
        carry: Vec<u8>,
        watchers: HashMap<u64, ConnSink>,
    }

    impl Term {
        pub(crate) fn info(&self) -> TerminalInfo {
            self.info.lock().clone()
        }

        /// Subscribe `conn` and return the recent output, atomically: nothing is lost or sent
        /// twice between the snapshot and the first `terminal.output`.
        pub(crate) fn attach(&self, conn: &ConnSink) -> String {
            let mut st = self.state.lock();
            st.watchers.insert(conn.id, conn.clone());
            let (a, b) = st.scrollback.as_slices();
            let mut bytes = Vec::with_capacity(a.len() + b.len());
            bytes.extend_from_slice(a);
            bytes.extend_from_slice(b);
            // A trimmed buffer may start inside a line or escape sequence: start at the next line.
            let start = if st.trimmed { bytes.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1) } else { 0 };
            String::from_utf8_lossy(&bytes[start..]).into_owned()
        }

        pub(crate) fn detach(&self, conn_id: u64) {
            self.state.lock().watchers.remove(&conn_id);
        }

        pub(crate) async fn write(&self, mut data: &[u8]) -> ApiResult<()> {
            let _guard = self.write_lock.lock().await;
            while !data.is_empty() {
                let mut ready = self.master.writable().await?;
                match ready.try_io(|fd| {
                    let n = unsafe { libc::write(fd.as_raw_fd(), data.as_ptr().cast(), data.len()) };
                    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
                }) {
                    Ok(Ok(n)) => data = &data[n..],
                    Ok(Err(err)) => return Err(ApiError::conflict(format!("terminal closed: {err}"))),
                    Err(_would_block) => {}
                }
            }
            Ok(())
        }

        pub(crate) fn resize(&self, cols: u16, rows: u16) -> ApiResult<()> {
            let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
            if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) } != 0 {
                return Err(ApiError::internal(format!("resize: {}", io::Error::last_os_error())));
            }
            let mut info = self.info.lock();
            (info.cols, info.rows) = (cols, rows);
            Ok(())
        }

        /// SIGHUP to the shell's process group, as when a terminal window closes.
        pub(crate) fn hang_up(&self) {
            // pid 0 would signal the daemon's own process group.
            if self.pid > 0 {
                unsafe { libc::kill(-self.pid, libc::SIGHUP) };
            }
        }

        /// Append output to the scrollback and send it to every viewer.
        async fn output(&self, terminals: &Terminals, bytes: &[u8]) {
            let (text, watchers) = {
                let mut st = self.state.lock();
                st.scrollback.extend(bytes);
                if st.scrollback.len() > SCROLLBACK {
                    let excess = st.scrollback.len() - SCROLLBACK;
                    st.scrollback.drain(..excess);
                    st.trimmed = true;
                }
                if st.watchers.is_empty() {
                    st.carry.clear();
                    return;
                }
                let mut carry = std::mem::take(&mut st.carry);
                let text = decode(&mut carry, bytes);
                st.carry = carry;
                (text, st.watchers.values().cloned().collect::<Vec<_>>())
            };
            if text.is_empty() {
                return;
            }
            let id = self.info.lock().id.clone();
            let Some(event) = coocoo_proto::bus::encode_event(
                events::TERMINAL_OUTPUT,
                terminals.emitter.host_id(),
                &json!({ "id": id, "data": text }),
            ) else {
                return;
            };
            for sink in watchers {
                let delivered = tokio::time::timeout(SEND_TIMEOUT, sink.send(event.clone())).await.unwrap_or(false);
                if !delivered {
                    self.detach(sink.id);
                }
            }
        }
    }

    /// Decode `bytes` after the incomplete sequence left in `carry`; keeps a new incomplete tail
    /// in `carry` and replaces invalid bytes.
    fn decode(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
        carry.extend_from_slice(bytes);
        let mut out = String::with_capacity(carry.len());
        let mut rest: &[u8] = carry;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    rest = &[];
                    break;
                }
                Err(err) => {
                    let (valid, after) = rest.split_at(err.valid_up_to());
                    out.push_str(unsafe { std::str::from_utf8_unchecked(valid) });
                    match err.error_len() {
                        Some(n) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[n..];
                        }
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        let tail = rest.to_vec();
        *carry = tail;
        out
    }

    fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
        if ret == -1 { Err(io::Error::last_os_error()) } else { Ok(ret) }
    }

    fn shell() -> String {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".to_owned())
    }

    impl Terminals {
        pub(super) fn open(self: &Arc<Self>, cwd: String, cols: u16, rows: u16) -> ApiResult<TerminalInfo> {
            let (master, slave) = openpty(cols, rows).map_err(|e| ApiError::internal(format!("cannot open a pty: {e}")))?;
            // SAFETY: the `OwnedFd` stays owned by the `AsyncFd` and is never replaced.
            let master = unsafe { AsyncFd::register(master) }.map_err(|e| ApiError::internal(format!("pty: {e}")))?;
            let shell = shell();
            let mut command = tokio::process::Command::new(&shell);
            command
                .arg("-l")
                .current_dir(&cwd)
                .env("TERM", "xterm-256color")
                .env("COLORTERM", "truecolor")
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            // New session with the pty as controlling terminal, so job control and ^C work.
            unsafe {
                command.pre_exec(|| {
                    cvt(libc::setsid())?;
                    cvt(libc::ioctl(0, libc::TIOCSCTTY as _, 0))?;
                    Ok(())
                });
            }
            let mut child = command.spawn().map_err(|e| ApiError::internal(format!("cannot start {shell}: {e}")))?;
            let pid = child.id().map_or(0, |p| p as i32);
            let info = TerminalInfo { id: new_id(), cwd, shell, cols, rows, created_at: now_ms() };
            let term = Arc::new(Term {
                info: Mutex::new(info.clone()),
                master,
                pid,
                write_lock: tokio::sync::Mutex::new(()),
                state: Mutex::new(State::default()),
            });
            self.terms.lock().insert(info.id.clone(), term.clone());
            // The slave copies were moved into the child's stdio and are closed here, so the
            // master reads EOF/EIO once the shell and everything it started have exited.
            drop(command);
            let me = Arc::downgrade(self);
            tokio::spawn(async move {
                pump(&me, &term).await;
                let code = child.wait().await.ok().and_then(|s| s.code());
                let Some(terminals) = me.upgrade() else { return };
                let id = term.info.lock().id.clone();
                terminals.terms.lock().remove(&id);
                let watchers: Vec<ConnSink> = term.state.lock().watchers.drain().map(|(_, s)| s).collect();
                for sink in watchers {
                    sink.try_send_event(events::TERMINAL_EXIT, terminals.emitter.host_id(), &json!({ "id": id, "code": code }));
                }
            });
            Ok(info)
        }
    }

    /// Read the master until the shell side is gone.
    async fn pump(terminals: &Weak<Terminals>, term: &Term) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let Ok(mut ready) = term.master.readable().await else { return };
            match ready.try_io(|fd| {
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
            }) {
                // EOF, or EIO: every slave descriptor is closed.
                Ok(Ok(0)) | Ok(Err(_)) => return,
                Ok(Ok(n)) => {
                    let Some(terminals) = terminals.upgrade() else { return };
                    term.output(&terminals, &buf[..n]).await;
                }
                Err(_would_block) => {}
            }
        }
    }

    /// A pty pair; the master is non-blocking, both ends close on exec.
    // macOS declares `winp` as `*mut winsize`, Linux as `*const`.
    #[allow(clippy::unnecessary_mut_passed)]
    fn openpty(cols: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
        let (mut master, mut slave) = (-1, -1);
        let mut ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        cvt(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) })?;
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        for fd in [&master, &slave] {
            let flags = cvt(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) })?;
            cvt(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) })?;
        }
        let flags = cvt(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) })?;
        cvt(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) })?;
        Ok((master, slave))
    }

    #[cfg(test)]
    mod tests {
        use super::decode;

        #[test]
        fn utf8_split_across_reads_is_reassembled() {
            let text = "grün → ✓";
            let bytes = text.as_bytes();
            let mut carry = Vec::new();
            let mut out = String::new();
            for chunk in bytes.chunks(1) {
                out.push_str(&decode(&mut carry, chunk));
            }
            assert_eq!(out, text);
            assert!(carry.is_empty());
        }

        #[test]
        fn invalid_bytes_are_replaced_not_carried() {
            let mut carry = Vec::new();
            assert_eq!(decode(&mut carry, b"a\xffb"), "a\u{fffd}b");
            assert!(carry.is_empty());
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::sync::Arc;

    use coocoo_proto::{ApiError, ApiResult, ConnSink, TerminalInfo};

    use super::Terminals;

    /// Never constructed: terminals need a Unix pty.
    pub(crate) enum Term {}

    impl Term {
        pub(crate) fn info(&self) -> TerminalInfo {
            match *self {}
        }
        pub(crate) fn attach(&self, _conn: &ConnSink) -> String {
            match *self {}
        }
        pub(crate) fn detach(&self, _conn_id: u64) {
            match *self {}
        }
        pub(crate) async fn write(&self, _data: &[u8]) -> ApiResult<()> {
            match *self {}
        }
        pub(crate) fn resize(&self, _cols: u16, _rows: u16) -> ApiResult<()> {
            match *self {}
        }
        pub(crate) fn hang_up(&self) {
            match *self {}
        }
    }

    impl Terminals {
        pub(super) fn open(self: &Arc<Self>, _cwd: String, _cols: u16, _rows: u16) -> ApiResult<TerminalInfo> {
            Err(ApiError::unavailable("terminals are only available on Linux and macOS hosts"))
        }
    }
}
