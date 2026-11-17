use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_ai::create_ai_provider;
use relating_core::{ProjectId, RetrievalStrategy, RetrievalTrace};
use relating_metadata::get_project as db_get_project;
use relating_retrieval::{run_retrieval, DEFAULT_TOP_K};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RetrievalRequest {
    pub query: String,
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

pub async fn run_retrieval_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<RetrievalRequest>,
) -> Result<Json<RetrievalTrace>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("Query cannot be empty".to_string()));
    }

    let provider = create_ai_provider(&project.ai_config)?;
    let trace = run_retrieval(
        &state.pool,
        &provider,
        &project,
        payload.query.trim(),
        payload.strategy.unwrap_or_default(),
        payload.top_k.unwrap_or(DEFAULT_TOP_K),
        payload.profile_name.as_deref(),
    )
    .await?;

    Ok(Json(trace))
}
