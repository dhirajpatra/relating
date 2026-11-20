use crate::error::AppError;
use crate::routes::aura::{build_backend, get_connection};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_core::{AuraConnectionId, ProjectId};
use relating_metadata::get_project as db_get_project;
use relating_shadow::{mask_params, replay_suite, ShadowQuery, ShadowReport, SuiteOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct ShadowRunRequest {
    pub connection_id: String,
    pub queries: Vec<ShadowQueryPayload>,
    #[serde(default = "default_true")]
    pub redact_params: bool,
    #[serde(default)]
    pub include_samples: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct ShadowQueryPayload {
    pub name: String,
    pub cypher: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct ShadowRunResponse {
    pub report: ShadowReport,
    /// Echo of submitted queries with params masked unless opted out.
    pub queries: Vec<ShadowQueryEcho>,
}

#[derive(Serialize)]
pub struct ShadowQueryEcho {
    pub name: String,
    pub cypher: String,
    pub params: serde_json::Value,
}

pub async fn run_shadow_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ShadowRunRequest>,
) -> Result<Json<ShadowRunResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let conn_id = AuraConnectionId::parse(&payload.connection_id).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid Connection UUID '{}': {}",
            payload.connection_id, e
        ))
    })?;
    let conn = get_connection(&state.pool, &project_id, &conn_id).await?;
    let backend = build_backend(&conn, None)?;

    let graph_path = state.project_graph_path(&project_id);
    let queries: Vec<ShadowQuery> = payload
        .queries
        .iter()
        .map(|spec| ShadowQuery {
            name: spec.name.clone(),
            cypher: spec.cypher.clone(),
            params: spec.params.clone(),
        })
        .collect();
    let options = SuiteOptions {
        row_limit: 200,
        include_samples: payload.include_samples,
    };
    let (comparisons, summary) = replay_suite(
        state.graph.as_ref(),
        &graph_path,
        &backend,
        &queries,
        &options,
    )
    .await?;

    let mut echo = Vec::with_capacity(payload.queries.len());
    for spec in &payload.queries {
        let params = if payload.redact_params {
            serde_json::to_value(mask_params(&spec.params)).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::to_value(&spec.params).unwrap_or(serde_json::Value::Null)
        };
        echo.push(ShadowQueryEcho {
            name: spec.name.clone(),
            cypher: spec.cypher.clone(),
            params,
        });
    }

    Ok(Json(ShadowRunResponse {
        report: ShadowReport {
            comparisons,
            summary,
        },
        queries: echo,
    }))
}
