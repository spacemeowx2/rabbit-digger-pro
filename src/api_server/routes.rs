use anyhow::Result;
use axum::{
    body::Body,
    extract::Extension,
    http,
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    routing::get_service,
    Router,
};
use hyper::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    Method, Request, StatusCode,
};
use serde::Deserialize;
use std::sync::Arc;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use super::{handlers::Ctx, rpc, ApiServer};

impl ApiServer {
    pub async fn routes(&self) -> Result<Router> {
        let mut router = Router::new()
            .nest("/api", self.api().await?)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_headers([AUTHORIZATION, CONTENT_TYPE])
                    .allow_methods([Method::GET, Method::POST, Method::POST, Method::DELETE]),
            )
            .layer(TraceLayer::new_for_http());

        if let Some(webui) = &self.web_ui {
            // Runtime override: serve from filesystem path
            let spa_index = std::path::Path::new(webui).join("index.html");
            router = router.fallback_service(get_service(
                ServeDir::new(webui)
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new(spa_index)),
            ))
        } else {
            #[cfg(feature = "webui_embed")]
            {
                router = router.merge(super::embedded_webui::embedded_webui_router());
            }
        }

        Ok(router)
    }

    async fn api(&self) -> Result<Router> {
        let ctx = Ctx {
            rd: self.rabbit_digger.clone(),
            cfg_mgr: self.config_manager.clone(),
            userdata: std::sync::Arc::new(
                crate::storage::FileStorage::new(crate::storage::FolderType::Data, "userdata")
                    .await?,
            ),
            source_sender: self.source_sender.as_ref().map(|s| Arc::new(s.clone())),
            log_file_path: self.log_file_path.clone(),
        };

        let mut router = Router::new()
            .route("/rpc", get(rpc::ws_rpc))
            .layer(Extension(ctx));

        if let Some(token) = &self.access_token {
            let token = token.clone();
            router = router.route_layer(middleware::from_fn(move |req, next| {
                let token = token.clone();
                auth(req, next, token)
            }))
        }

        Ok(router)
    }
}

#[derive(Deserialize)]
struct AuthQuery {
    token: String,
}
async fn auth(req: Request<Body>, next: Next, token: String) -> impl IntoResponse {
    let auth_header = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let query = req.uri().query().unwrap_or_default();
    let value = serde_urlencoded::from_str(query)
        .ok()
        .map(|i: AuthQuery| i.token);

    match auth_header.or(value.as_ref().map(AsRef::as_ref)) {
        Some(auth_header) if auth_header == token => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
