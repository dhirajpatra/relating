use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use relating_core::{
    Actor, ActorContext, AiProviderConfig, CreateProjectRequest, Project, ProjectId,
    ProjectSummary, Role,
};
use relating_graph::{GraphBackend, GraphSchemaInfo};
use relating_metadata::{
    delete_project as db_delete, get_project as db_get, insert_project, list_projects as db_list,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ProjectDetailResponse {
    pub project: Project,
    pub schema: GraphSchemaInfo,
}

pub async fn create_project_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<Project>), AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "Project name cannot be empty".to_string(),
        ));
    }

    let project_id = ProjectId::new();
    let now = Utc::now();
    let ai_config = payload.ai_config.unwrap_or_else(AiProviderConfig::mock);

    let mut project = Project {
        id: project_id.clone(),
        name: name.to_string(),
        description: payload.description.unwrap_or_default(),
        created_at: now,
        updated_at: now,
        ai_config,
    };

    // 1. Insert into SQLite metadata
    insert_project(&state.pool, &project).await?;

    // 2. OIDC creators become admins of their own project, so the grant
    // that let them create (global editor) is not needed for every visit.
    if let Actor::User { id, .. } = &actor.actor {
        relating_metadata::create_grant(
            &state.pool,
            id,
            Some(&project.id.to_string()),
            Role::Admin,
            &actor.actor.display(),
        )
        .await?;
    }

    // 3. Ensure project-scoped graph store exists on disk
    let graph_path = state.project_graph_path(&project.id);
    state.graph.ensure_store(&graph_path)?;

    // Provider keys are write-only: accepted on create, never returned.
    project.ai_config = project.ai_config.redacted();
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn list_projects_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProjectSummary>>, AppError> {
    let projects = db_list(&state.pool).await?;
    let mut summaries = Vec::with_capacity(projects.len());

    for proj in projects {
        let graph_path = state.project_graph_path(&proj.id);
        let schema = state.graph.get_schema(&graph_path).unwrap_or_default();

        summaries.push(ProjectSummary {
            id: proj.id,
            name: proj.name,
            description: proj.description,
            created_at: proj.created_at,
            updated_at: proj.updated_at,
            node_count: schema.node_count,
            relationship_count: schema.relationship_count,
            provider_type: proj.ai_config.provider_type,
        });
    }

    Ok(Json(summaries))
}

pub async fn get_project_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<ProjectDetailResponse>, AppError> {
    let id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let mut project = db_get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;
    project.ai_config = project.ai_config.redacted();

    let graph_path = state.project_graph_path(&id);
    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();

    Ok(Json(ProjectDetailResponse { project, schema }))
}

pub async fn delete_project_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let deleted = db_delete(&state.pool, &id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "Project with ID '{}' not found",
            id
        )));
    }

    // Clean up project files on disk
    let project_dir = state.project_dir(&id);
    if project_dir.exists() {
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    Ok(StatusCode::NO_CONTENT)
}
