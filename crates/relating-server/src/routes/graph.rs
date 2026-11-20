use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use relating_core::ProjectId;
use relating_graph::{CytoscapeGraph, GraphBackend};
use relating_metadata::get_project as db_get_project;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct GraphQueryLimit {
    pub limit: Option<u32>,
}

pub async fn get_graph_elements_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Query(params): Query<GraphQueryLimit>,
) -> Result<Json<CytoscapeGraph>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    // Verify project exists
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let graph_path = state.project_graph_path(&project_id);
    let elements = state
        .graph
        .get_cytoscape_elements(&graph_path, params.limit)?;

    Ok(Json(elements))
}
