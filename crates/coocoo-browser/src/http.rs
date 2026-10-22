//! Minimal HTTP/1.1 GET for Chromium's loopback DevTools discovery endpoints.

use std::time::Duration;

use coocoo_proto::{ApiError, ApiResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// `json/protocol` is ~1 MiB; anything far beyond that is not DevTools.
const MAX_RESPONSE: usize = 16 << 20;

/// GET `http://127.0.0.1:<port><path>` and return the body of a 200 response.
pub(crate) async fn get(port: u16, path: &str) -> ApiResult<Vec<u8>> {
    let fut = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        // Chromium rejects Host headers that are not an IP or `localhost` (DNS-rebinding guard)
        // and derives the advertised WebSocket URLs from it.
        let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        // Chromium keeps the socket open despite `Connection: close`: stop at the framed end.
        let mut raw = Vec::with_capacity(8192);
        while !is_complete(&raw) {
            if raw.len() > MAX_RESPONSE {
                return Err(std::io::Error::other("response too large"));
            }
            if stream.read_buf(&mut raw).await? == 0 {
                break;
            }
        }
        Ok::<_, std::io::Error>(raw)
    };
    let raw = tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .map_err(|_| ApiError::unavailable("Chromium DevTools endpoint timed out"))?
        .map_err(|e| ApiError::unavailable(format!("Chromium DevTools endpoint: {e}")))?;
    parse_response(raw)
}

/// Whether `raw` holds a whole response (by `Content-Length` or the chunked terminator).
fn is_complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    let Ok(head) = std::str::from_utf8(&raw[..split]) else { return true };
    let body = raw.len() - split - 4;
    for line in head.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else { continue };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_or(true, |len| body >= len);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") && value.trim().eq_ignore_ascii_case("chunked") {
            return raw.ends_with(b"\r\n0\r\n\r\n") || (body == 5 && raw.ends_with(b"0\r\n\r\n"));
        }
    }
    false
}

fn parse_response(raw: Vec<u8>) -> ApiResult<Vec<u8>> {
    let bad = |what: &str| ApiError::unavailable(format!("Chromium DevTools endpoint: {what}"));
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| bad("truncated response"))?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| bad("invalid headers"))?;
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split_whitespace().nth(1)).ok_or_else(|| bad("no status line"))?
        .to_owned();
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        }
    }
    let mut body = raw;
    body.drain(..split + 4);
    if chunked {
        body = dechunk(&body).ok_or_else(|| bad("malformed chunked body"))?;
    } else if let Some(len) = content_length {
        if body.len() < len {
            return Err(bad("truncated body"));
        }
        body.truncate(len);
    }
    if status == "404" {
        return Err(ApiError::not_found("DevTools endpoint"));
    }
    if status != "200" {
        return Err(bad(&format!("HTTP {status}: {}", String::from_utf8_lossy(&body))));
    }
    Ok(body)
}

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&data[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_length_and_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n{}junk".to_vec();
        assert_eq!(parse_response(raw).unwrap(), b"{}");
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n[1,\r\n2\r\n2]\r\n0\r\n\r\n".to_vec();
        assert_eq!(parse_response(raw).unwrap(), b"[1,2]");
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec();
        assert_eq!(parse_response(raw).unwrap_err().code, "not_found");
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n".to_vec()).is_err());
    }

    #[test]
    fn detects_framed_end() {
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n"));
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}"));
        assert!(is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{\"\"}"));
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n"));
        assert!(is_complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"));
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\n\r\n{}"));
    }
}
