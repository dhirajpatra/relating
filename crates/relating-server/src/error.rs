use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use relating_core::RelatingError;
use relating_graph::GraphError;
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),

    #[error("Core error: {0}")]
    Core(#[from] RelatingError),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Internal server error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            AppError::Core(RelatingError::ProjectNotFound(msg)) => {
                (StatusCode::NOT_FOUND, "PROJECT_NOT_FOUND", msg.clone())
            }
            AppError::Core(RelatingError::ValidationError(msg)) => {
                (StatusCode::BAD_REQUEST, "VALIDATION_ERROR", msg.clone())
            }
            AppError::Graph(GraphError::StoreNotFound(msg)) => {
                (StatusCode::NOT_FOUND, "STORE_NOT_FOUND", msg.clone())
            }
            AppError::Graph(GraphError::Syntax(msg)) => {
                (StatusCode::BAD_REQUEST, "CYPHER_SYNTAX_ERROR", msg.clone())
            }
            AppError::Graph(GraphError::Semantic(msg)) => {
                (StatusCode::BAD_REQUEST, "CYPHER_SEMANTIC_ERROR", msg.clone())
            }
            AppError::Graph(GraphError::ReadOnlyViolation(msg)) => {
                (StatusCode::FORBIDDEN, "READ_ONLY_VIOLATION", msg.clone())
            }
            AppError::Ai(err) => (
                StatusCode::BAD_GATEWAY,
                "AI_PROVIDER_ERROR",
                err.to_string(),
            ),
            other => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                other.to_string(),
            ),
        };

        let body = Json(json!({
            "error": {
                "code": code,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}
