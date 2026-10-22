//! Static web UI (`ui/dist`) for browser/phone access, embedded with `embed-ui`.
//!
//! SPA routing: unknown extension-less paths serve `index.html`. Vite's hashed `assets/*` are
//! cached for a year; everything else revalidates via `ETag`.

use axum::http::{HeaderMap, Uri};
#[cfg(feature = "embed-ui")]
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

const NOT_BUILT: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Coocoo</title>\
<style>body{font:15px system-ui,sans-serif;margin:15vh auto;max-width:34em;color:#222;padding:0 1em}\
code{background:#f2f2f2;padding:.1em .3em;border-radius:3px}</style></head><body>\
<h1>Coocoo daemon is running</h1><p>The web UI is not part of this build. Build it with \
<code>npm --prefix ui ci &amp;&amp; npm --prefix ui run build</code> and rebuild the daemon \
with the <code>embed-ui</code> feature.</p></body></html>";

#[cfg(feature = "embed-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../ui/dist"]
#[allow_missing = true]
struct Assets;

#[cfg(feature = "embed-ui")]
pub(crate) async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(path) {
        return respond(path, file, &headers);
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match Assets::get("index.html") {
        Some(file) => respond("index.html", file, &headers),
        None => Html(NOT_BUILT).into_response(),
    }
}

#[cfg(feature = "embed-ui")]
fn respond(path: &str, file: rust_embed::EmbeddedFile, headers: &HeaderMap) -> Response {
    use std::borrow::Cow;
    use std::fmt::Write;

    let hash = file.metadata.sha256_hash();
    let mut etag = String::with_capacity(2 + 32);
    etag.push('"');
    for byte in &hash[..16] {
        let _ = write!(etag, "{byte:02x}");
    }
    etag.push('"');
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag), (header::CACHE_CONTROL, cache.to_owned())]).into_response();
    }
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let body = match file.data {
        Cow::Borrowed(bytes) => axum::body::Body::from(bytes),
        Cow::Owned(bytes) => axum::body::Body::from(bytes),
    };
    (
        [(header::CONTENT_TYPE, mime.essence_str().to_owned()), (header::ETAG, etag), (header::CACHE_CONTROL, cache.to_owned())],
        body,
    )
        .into_response()
}

#[cfg(not(feature = "embed-ui"))]
pub(crate) async fn serve(_uri: Uri, _headers: HeaderMap) -> Response {
    Html(NOT_BUILT).into_response()
}
