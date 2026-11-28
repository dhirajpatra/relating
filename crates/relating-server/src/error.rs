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

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),

    #[error("Core error: {0}")]
    Core(#[from] RelatingError),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Ingest error: {0}")]
    Ingest(#[from] relating_ingest::IngestError),

    #[error("Retrieval error: {0}")]
    Retrieval(#[from] relating_retrieval::RetrievalError),

    #[error("Migration error: {0}")]
    Migrate(#[from] relating_migrate::MigrateError),

    #[error("Schema error: {0}")]
    Schema(#[from] relating_schema::SchemaError),

    #[error("Portability error: {0}")]
    Port(#[from] relating_portability::PortError),

    #[error("Shadow error: {0}")]
    Shadow(#[from] relating_shadow::ShadowError),

    #[error("Debug error: {0}")]
    Debug(#[from] relating_debug::DebugError),

    #[error("Synth error: {0}")]
    Synth(#[from] relating_synth::SynthError),

    #[error("Advisor error: {0}")]
    Advisor(#[from] relating_advisor::AdvisorError),

    #[error("Connector error: {0}")]
    Connectors(#[from] relating_connectors::ConnectorError),

    #[error("Internal server error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "FORBIDDEN", msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone()),
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
            AppError::Graph(GraphError::Semantic(msg)) => (
                StatusCode::BAD_REQUEST,
                "CYPHER_SEMANTIC_ERROR",
                msg.clone(),
            ),
            AppError::Graph(GraphError::ReadOnlyViolation(msg)) => {
                (StatusCode::FORBIDDEN, "READ_ONLY_VIOLATION", msg.clone())
            }
            AppError::Ai(err) => (
                StatusCode::BAD_GATEWAY,
                "AI_PROVIDER_ERROR",
                err.to_string(),
            ),
            AppError::Ingest(err) => (StatusCode::BAD_REQUEST, "INGEST_ERROR", err.to_string()),
            AppError::Retrieval(relating_retrieval::RetrievalError::Ai(err)) => (
                StatusCode::BAD_GATEWAY,
                "AI_PROVIDER_ERROR",
                err.to_string(),
            ),
            AppError::Retrieval(err) => {
                (StatusCode::BAD_REQUEST, "RETRIEVAL_ERROR", err.to_string())
            }
            AppError::Migrate(relating_migrate::MigrateError::Auth(msg)) => {
                (StatusCode::BAD_GATEWAY, "AURA_AUTH_FAILED", msg.clone())
            }
            AppError::Migrate(relating_migrate::MigrateError::Connection(msg))
            | AppError::Migrate(relating_migrate::MigrateError::Timeout(msg)) => {
                (StatusCode::BAD_GATEWAY, "AURA_UNREACHABLE", msg.clone())
            }
            AppError::Migrate(err) => (StatusCode::BAD_REQUEST, "MIGRATION_ERROR", err.to_string()),
            AppError::Schema(err) => (StatusCode::BAD_REQUEST, "SCHEMA_ERROR", err.to_string()),
            AppError::Port(err) => (
                StatusCode::BAD_REQUEST,
                "PORTABILITY_ERROR",
                err.to_string(),
            ),
            AppError::Shadow(err) => (StatusCode::BAD_REQUEST, "SHADOW_ERROR", err.to_string()),
            AppError::Debug(err) => (StatusCode::BAD_REQUEST, "DEBUG_ERROR", err.to_string()),
            AppError::Synth(err) => (StatusCode::BAD_REQUEST, "SYNTH_ERROR", err.to_string()),
            AppError::Advisor(err) => (StatusCode::BAD_REQUEST, "ADVISOR_ERROR", err.to_string()),
            AppError::Connectors(
                relating_connectors::ConnectorError::Config(_)
                | relating_connectors::ConnectorError::Mapping(_),
            ) => (
                StatusCode::BAD_REQUEST,
                "CONNECTOR_CONFIG",
                self.to_string(),
            ),
            AppError::Connectors(err) => (
                StatusCode::BAD_GATEWAY,
                "CONNECTOR_UPSTREAM",
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
