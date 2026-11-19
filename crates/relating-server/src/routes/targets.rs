use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_core::ProjectId;
use relating_graph::QueryTarget;
use relating_metadata::{
    get_project as db_get_project, list_aura_connections as db_list_connections,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct QueryTargetInfo {
    pub backend: String,
    pub connection_string: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct QueryTargetsResponse {
    pub project_id: String,
    pub local: QueryTargetInfo,
    pub aura: Vec<QueryTargetInfo>,
    pub usage: String,
}

/// List the stable query targets for a project.
///
/// Application code keeps one shaped Cypher API and flips backends by
/// swapping the connection string: `relate://local` runs on the project's
/// local Relate store, `aura://<connection-id>` proxies the identical
/// statement to Aura. No query rewrite is needed after migration.
pub async fn list_query_targets_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<QueryTargetsResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let connections = db_list_connections(&state.pool, &project_id).await?;
    let aura = connections
        .into_iter()
        .map(|conn| {
            let target = QueryTarget::Aura {
                connection_id: conn.id.to_string(),
            };
            QueryTargetInfo {
                backend: "aura".to_string(),
                connection_string: target.connection_string(),
                connection_id: Some(conn.id.to_string()),
                name: Some(conn.name),
            }
        })
        .collect();

    Ok(Json(QueryTargetsResponse {
        project_id: project_id.to_string(),
        local: QueryTargetInfo {
            backend: "relate".to_string(),
            connection_string: QueryTarget::Local.connection_string(),
            connection_id: None,
            name: Some("local".to_string()),
        },
        aura,
        usage: "POST the same {query, params} to /cypher/read (or /cypher/write) with 'connection_id' set to an Aura connection id to run it on Aura instead of the local store.".to_string(),
    }))
}
