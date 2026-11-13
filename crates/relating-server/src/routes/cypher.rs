use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_core::ProjectId;
use relating_graph::{GraphBackend, GraphQueryResult, GraphWriteResult};
use relating_metadata::get_project as db_get;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct CypherReadRequest {
    pub query: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct CypherWriteRequest {
    pub query: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

pub async fn cypher_read_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CypherReadRequest>,
) -> Result<Json<GraphQueryResult>, AppError> {
    let id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    // Verify project exists
    let _ = db_get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;

    let graph_path = state.project_graph_path(&id);
    let limit = payload.limit.or(Some(100)); // Default read limit of 100 for safety

    let result = state
        .graph
        .execute_read(&graph_path, &payload.query, payload.params, limit)?;

    Ok(Json(result))
}

pub async fn cypher_write_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CypherWriteRequest>,
) -> Result<Json<GraphWriteResult>, AppError> {
    let id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    // Verify project exists
    let _ = db_get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;

    let graph_path = state.project_graph_path(&id);
    let result = state
        .graph
        .execute_write(&graph_path, &payload.query, payload.params)?;

    Ok(Json(result))
}
