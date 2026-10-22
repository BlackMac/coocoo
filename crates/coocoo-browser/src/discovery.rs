//! Per-profile loopback CDP discovery listener.
//!
//! puppeteer (omp's browser tool) resolves discovery as `new URL('/json/version', cdpUrl)`, which
//! drops any path, so each profile gets its own `http://127.0.0.1:<port>` whose root serves
//! Chrome's discovery JSON. Idle listeners only park in `accept`; Chromium launches on the first
//! discovery request.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Weak};
use std::time::Duration;

use coocoo_proto::{ApiError, ApiResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::AbortHandle;

use crate::Browser;

/// Largest accepted request head.
const MAX_HEAD: usize = 8 << 10;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A bound discovery listener.
pub(crate) struct Endpoint {
    pub port: u16,
    pub task: AbortHandle,
}

impl Endpoint {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Bind `127.0.0.1:0` and serve discovery for `profile` until aborted or the browser is dropped.
pub(crate) fn bind(browser: &Arc<Browser>, profile: &str) -> ApiResult<Endpoint> {
    let std = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .and_then(|l| l.set_nonblocking(true).map(|()| l))
        .map_err(|e| ApiError::internal(format!("cannot bind CDP discovery listener: {e}")))?;
    let port = std.local_addr().map_err(|e| ApiError::internal(format!("CDP discovery listener: {e}")))?.port();
    let listener =
        TcpListener::from_std(std).map_err(|e| ApiError::internal(format!("CDP discovery listener: {e}")))?;
    let task = tokio::spawn(accept_loop(listener, Arc::downgrade(browser), profile.to_owned())).abort_handle();
    Ok(Endpoint { port, task })
}

async fn accept_loop(listener: TcpListener, browser: Weak<Browser>, profile: String) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::debug!("CDP discovery accept ({profile}): {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Some(browser) = browser.upgrade() else { return };
        let profile = profile.clone();
        tokio::spawn(async move { serve(stream, &browser, &profile).await });
    }
}

async fn serve(mut stream: TcpStream, browser: &Arc<Browser>, profile: &str) {
    let head = match tokio::time::timeout(READ_TIMEOUT, read_head(&mut stream)).await {
        Ok(Some(head)) => head,
        _ => return,
    };
    let (status, body) = match respond(&head, browser, profile).await {
        Ok(body) => (200, body),
        Err((status, err)) => (status, serde_json::to_vec(&err).unwrap_or_default()),
    };
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

/// Read until the end of the request head; `None` on EOF, error or an oversized head.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(end);
            return String::from_utf8(buf).ok();
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    }
}

async fn respond(head: &str, browser: &Arc<Browser>, profile: &str) -> Result<Vec<u8>, (u16, ApiError)> {
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split(' ');
    let (method, target) = (request.next().unwrap_or_default(), request.next().unwrap_or_default());
    if method != "GET" {
        return Err((405, ApiError::invalid("only GET is supported")));
    }
    let host = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim());
    if !host.is_none_or(host_is_loopback) {
        return Err((403, ApiError::new("forbidden", "CDP discovery is loopback-only")));
    }
    let path = target.split(['?', '#']).next().unwrap_or_default().trim_matches('/');
    let path = match path {
        "json" | "json/list" | "json/version" | "json/protocol" => path,
        _ => return Err((404, ApiError::not_found(format!("CDP endpoint `/{path}`")))),
    };
    browser.cdp_discovery(profile, path).await.map_err(|err| {
        let status = match err.code.as_str() {
            "not_found" => 404,
            "unavailable" => 503,
            _ => 500,
        };
        (status, err)
    })
}

/// `127.0.0.1`, `localhost` or `[::1]`, with an optional port.
fn host_is_loopback(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((name, port)) if port.is_empty() || port.starts_with(':') => name,
            _ => return false,
        },
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts() {
        for (host, ok) in [
            ("127.0.0.1:9222", true),
            ("127.0.0.1", true),
            ("localhost", true),
            ("LOCALHOST:1", true),
            ("[::1]:9222", true),
            ("[::1]", true),
            ("evil.com", false),
            ("evil.com:9222", false),
            ("10.0.0.1", false),
            ("[::2]", false),
            ("127.0.0.1.evil.com", false),
        ] {
            assert_eq!(host_is_loopback(host), ok, "{host}");
        }
    }
}
