use crate::error::AppError;
use crate::routes::aura::build_backend;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_core::{AuraConnectionId, ProjectId};
use relating_graph::{GraphBackend, GraphQueryResult, GraphWriteResult};
use relating_metadata::{get_aura_connection as db_get_connection, get_project as db_get};
use relating_migrate::{json_to_bolt, proxied_read, summarize_write_meta, Neo4jBackend};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
pub struct CypherReadRequest {
    pub query: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Optional Aura connection id. When absent the query runs against the
    /// project's local Relate store (historical behavior). When present the
    /// same Cypher text and bound parameters are proxied to Aura and the
    /// result is returned in the identical shape, so application code flips
    /// backends by changing only this target — never the query.
    #[serde(default)]
    pub connection_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CypherWriteRequest {
    pub query: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    /// Optional Aura connection id for remote writes. Requires
    /// `confirm_remote_write` to be true; local writes are unaffected.
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub confirm_remote_write: bool,
}

async fn proxy_connection(
    state: &AppState,
    project_id: &ProjectId,
    connection_id: &str,
) -> Result<relating_migrate::BoltBackend, AppError> {
    let conn_id = AuraConnectionId::parse(connection_id).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid Connection UUID '{}': {}",
            connection_id, e
        ))
    })?;
    let conn = db_get_connection(&state.pool, &conn_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Connection '{}' not found", conn_id)))?;
    if conn.project_id != *project_id {
        return Err(AppError::NotFound(format!(
            "Connection '{}' not found",
            conn_id
        )));
    }
    build_backend(&conn, None)
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

    if let Some(connection_id) = payload.connection_id.as_deref() {
        let backend = proxy_connection(&state, &id, connection_id).await?;
        let limit = payload.limit.or(Some(100)); // Same default read bound as local
        let result = proxied_read(&backend, &payload.query, payload.params, limit).await?;
        return Ok(Json(result));
    }

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

    if let Some(connection_id) = payload.connection_id.as_deref() {
        if !payload.confirm_remote_write {
            return Err(AppError::BadRequest(
                "Remote writes require explicit confirmation: set 'confirm_remote_write' to true"
                    .to_string(),
            ));
        }
        let backend = proxy_connection(&state, &id, connection_id).await?;
        let params: BTreeMap<String, relating_migrate::BoltValue> = payload
            .params
            .iter()
            .map(|(k, v)| (k.clone(), json_to_bolt(v)))
            .collect();
        let start = std::time::Instant::now();
        let meta = backend.run_write(&payload.query, params).await?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(Json(summarize_write_meta(&meta, elapsed_ms)));
    }

    let graph_path = state.project_graph_path(&id);
    let result = state
        .graph
        .execute_write(&graph_path, &payload.query, payload.params)?;

    Ok(Json(result))
}
