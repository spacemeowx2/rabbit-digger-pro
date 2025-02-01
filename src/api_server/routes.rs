use anyhow::Result;
use axum::{
    extract::Extension,
    http,
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    routing::{delete, get_service, post},
    Router,
};
use hyper::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    Method, Request, StatusCode,
};
use rd_interface::Arc;
use serde::Deserialize;
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};

use crate::storage::{FileStorage, FolderType};

use super::{
    handlers::{self, Ctx},
    v2::handlers as v2_handlers,
    ApiServer,
};

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
            router = router.fallback(get_service(ServeDir::new(webui)))
        }

        Ok(router)
    }

    async fn api(&self) -> Result<Router> {
        let ctx = Ctx {
            rd: self.rabbit_digger.clone(),
            cfg_mgr: self.config_manager.clone(),
            userdata: Arc::new(FileStorage::new(FolderType::Data, "userdata").await?),
        };

        let mut router = Router::new()
            .route(
                "/config",
                get(handlers::get_config).post(handlers::post_config),
            )
            .route("/get", get(handlers::get_registry))
            .route("/state", get(handlers::get_state))
            .route("/connection/:uuid", delete(handlers::delete_conn))
            .route(
                "/connection",
                get(handlers::get_connections).delete(handlers::delete_connections),
            )
            .route("/net/:net_name", post(handlers::post_select))
            .route("/net/:net_name/delay", get(handlers::get_delay))
            .route(
                "/userdata/*path",
                get(handlers::get_userdata)
                    .put(handlers::put_userdata)
                    .delete(handlers::delete_userdata),
            )
            .route("/userdata", get(handlers::list_userdata))
            .route("/stream/connection", get(handlers::get_connection))
            .route("/stream/logs", get(handlers::ws_log))
            .route("/v2/ws", get(v2_handlers::websocket_handler))
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
async fn auth<B>(req: Request<B>, next: Next<B>, token: String) -> impl IntoResponse {
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
