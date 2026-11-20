use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_core::{AuraConnection, AuraConnectionId, ProjectId};
use relating_metadata::{
    delete_aura_connection as db_delete, get_aura_connection_resolving_env,
    get_project as db_get_project, insert_aura_connection,
    list_aura_connections_including_env as db_list,
};
use relating_migrate::{BoltBackend, BoltConfig, BoltUri, Neo4jBackend, TargetInfo};
use serde::{Deserialize, Serialize};

pub const DEFAULT_AUTH_ENV_VAR: &str = "AURA_PASSWORD";

#[derive(Deserialize)]
pub struct CreateConnectionRequest {
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub auth_env_var: Option<String>,
}

#[derive(Serialize)]
pub struct ConnectionTestReport {
    pub reachable: bool,
    pub server_agent: Option<String>,
    pub bolt_version: Option<String>,
    pub database: Option<String>,
    pub message: String,
}

/// Fetch a connection for a project, resolving the `.env` connection when
/// no stored row matches. All Aura-backed routes go through here so an
/// `AURA_URI` in the environment is usable everywhere a stored id is.
pub async fn get_connection(
    pool: &relating_metadata::DbPool,
    project_id: &ProjectId,
    conn_id: &AuraConnectionId,
) -> Result<AuraConnection, AppError> {
    get_aura_connection_resolving_env(pool, project_id, conn_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Connection '{}' not found", conn_id)))
}

pub fn build_backend(
    conn: &AuraConnection,
    password_override: Option<String>,
) -> Result<BoltBackend, AppError> {
    let uri = BoltUri::parse(&conn.uri)?;
    let password = match password_override {
        Some(p) => p,
        None => relating_core::resolve_secret(&conn.auth_env_var).map_err(AppError::BadRequest)?,
    };
    let mut config = BoltConfig::new(uri, conn.username.clone(), password);
    config.database = conn.database.clone();
    Ok(BoltBackend::new(config))
}

pub async fn create_connection_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CreateConnectionRequest>,
) -> Result<(StatusCode, Json<AuraConnection>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "Connection name cannot be empty".to_string(),
        ));
    }

    // Validate the URI up front (scheme, host, no embedded credentials).
    // Parse problems are client errors; runtime reachability stays a 502.
    BoltUri::parse(payload.uri.trim())
        .map_err(|e| AppError::BadRequest(format!("Invalid Aura URI: {}", e)))?;

    let conn = AuraConnection {
        id: AuraConnectionId::new(),
        project_id,
        name: name.to_string(),
        uri: payload.uri.trim().to_string(),
        username: payload.username.unwrap_or_else(|| "neo4j".to_string()),
        database: payload.database,
        auth_env_var: payload
            .auth_env_var
            .unwrap_or_else(|| DEFAULT_AUTH_ENV_VAR.to_string()),
        created_at: Utc::now(),
    };

    insert_aura_connection(&state.pool, &conn)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                AppError::BadRequest(format!("A connection named '{}' already exists", name))
            } else {
                AppError::Core(e)
            }
        })?;

    Ok((StatusCode::CREATED, Json(conn)))
}

pub async fn list_connections_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<AuraConnection>>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let conns = db_list(&state.pool, &project_id).await?;
    Ok(Json(conns))
}

pub async fn delete_connection_handler(
    State(state): State<AppState>,
    Path((id_str, conn_id_str)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let conn_id = AuraConnectionId::parse(&conn_id_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Connection UUID '{}': {}", conn_id_str, e))
    })?;

    let deleted = db_delete(&state.pool, &conn_id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "Connection '{}' not found",
            conn_id
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_connection_handler(
    State(state): State<AppState>,
    Path((id_str, conn_id_str)): Path<(String, String)>,
) -> Result<Json<ConnectionTestReport>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let conn_id = AuraConnectionId::parse(&conn_id_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Connection UUID '{}': {}", conn_id_str, e))
    })?;

    let conn = get_connection(&state.pool, &project_id, &conn_id).await?;

    let backend = build_backend(&conn, None)?;
    let info: TargetInfo = backend.test_connection().await?;

    Ok(Json(ConnectionTestReport {
        reachable: true,
        server_agent: Some(info.server_agent.clone()),
        bolt_version: Some(format!("{}.{}", info.bolt_major, info.bolt_minor)),
        database: info.database.clone(),
        message: format!(
            "Connected to {} over Bolt {}.{}",
            info.server_agent, info.bolt_major, info.bolt_minor
        ),
    }))
}
