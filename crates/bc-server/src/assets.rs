//! The Trunk bundle, embedded at compile time.

use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header;
use axum::response::IntoResponse as _;
use axum::response::Response;

use self::bundle::Assets;

/// Holds only the derive, so the lint expectation covers nothing else.
#[expect(
    clippy::same_name_method,
    reason = "the derive gives `Assets` both an inherent and an `Embed` `get`/`iter`"
)]
#[expect(
    clippy::inline_modules,
    reason = "the module exists only to scope the expectation above to the derive"
)]
mod bundle {
    /// Trunk's `dist-web/`; debug builds read it from disk on each request.
    ///
    /// `allow_missing` lets the workspace build before the bundle exists; the
    /// shell then answers 503 with build instructions.
    #[derive(rust_embed::Embed)]
    #[folder = "../bc-ui/dist-web"]
    #[allow_missing = true]
    pub(super) struct Assets;
}

/// Serves a bundled file, or `index.html` for an extensionless route.
pub(crate) async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        return (
            [(header::CONTENT_TYPE, file.metadata.mimetype().to_owned())],
            file.data,
        )
            .into_response();
    }
    // A path with an extension names a file; a missing one is a real 404.
    let is_route = path
        .rsplit('/')
        .next()
        .is_none_or(|last| !last.contains('.'));
    if !is_route {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get("index.html") {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.data,
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "frontend not built: run `mise run build:server`",
        )
            .into_response(),
    }
}
