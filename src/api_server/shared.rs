use std::sync::Arc;

use axum::{
    response::{IntoResponse, Response},
    BoxError, Json,
};
use hyper::StatusCode;
use rabbit_digger::RabbitDigger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    config::{ConfigManager, ImportSource},
    storage::FileStorage,
};

#[derive(Clone)]
pub(super) struct Ctx {
    pub(super) rd: RabbitDigger,
    pub(super) cfg_mgr: ConfigManager,
    pub(super) userdata: Arc<FileStorage>,
    pub(super) source_sender: Option<Arc<tokio::sync::mpsc::Sender<ImportSource>>>,
    pub(super) log_file_path: Option<std::path::PathBuf>,
}

pub(super) enum ApiError {
    NotFound,
    EngineNotRunning,
    Anyhow(anyhow::Error),
    Other(BoxError),
}

impl ApiError {
    pub(super) fn other<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        ApiError::Other(Box::new(err))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(inner: anyhow::Error) -> Self {
        if inner.to_string().contains("Not running") {
            return ApiError::EngineNotRunning;
        }
        ApiError::Anyhow(inner)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            ApiError::EngineNotRunning => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Engine not running".to_string(),
            ),
            ApiError::Anyhow(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
            ApiError::Other(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ConnectionQuery {
    #[serde(default)]
    pub patch: bool,
    #[serde(default)]
    pub without_connections: bool,
}

#[derive(Debug, Serialize)]
pub struct DelayResponse {
    pub(super) connect: u64,
    pub(super) response: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaybePatch {
    Full(Value),
    Patch(json_patch::Patch),
}
