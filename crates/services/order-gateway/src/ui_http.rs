//! Paper trader UI (`GET /ui`).

use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};

const INDEX_HTML: &str = include_str!("../static/ui/index.html");

/// `GET /ui` — same-origin paper trader (calls `/v1/*` with Bearer auth).
pub async fn get_ui() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(INDEX_HTML),
    )
        .into_response()
}
