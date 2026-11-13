use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_ai::{create_ai_provider, AiHealthReport};
use relating_core::ProjectId;
use relating_metadata::get_project as db_get;

pub async fn test_project_ai_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<AiHealthReport>, AppError> {
    let id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;

    let provider = create_ai_provider(&project.ai_config)?;
    let report = provider.health_check().await?;

    Ok(Json(report))
}
