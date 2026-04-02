use axum::{
    extract::Request,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use include_dir::{include_dir, Dir};

static WEBUI_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/webui/dist");

fn mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

async fn serve_embedded(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // Try exact file match
    if let Some(file) = WEBUI_DIR.get_file(path) {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime_from_path(path))],
            file.contents(),
        )
            .into_response();
    }

    // SPA fallback: serve index.html for any unmatched route
    if let Some(file) = WEBUI_DIR.get_file("index.html") {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            file.contents(),
        )
            .into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

pub fn embedded_webui_router() -> Router {
    Router::new().fallback(get(serve_embedded))
}
