use crate::error::AppError;
use crate::routes::aura::build_backend;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_advisor::{
    analyze_query, analyze_structure, estimate_sizing, estimate_sizing_with_vectors,
    measure_target, QueryFinding, SizingEstimate, StructureNote, TargetMeasurements,
    VectorSizingInput,
};
use relating_core::{AuraConnectionId, ProjectId};
use relating_graph::GraphBackend;
use relating_metadata::{get_aura_connection as db_get_connection, get_project as db_get_project};
use serde::{Deserialize, Serialize};

pub const MAX_ADVISOR_QUERIES: usize = 50;
pub const MAX_ADVISOR_CYPHER_LEN: usize = 20000;

#[derive(Deserialize)]
pub struct AdvisorQuery {
    pub name: String,
    pub cypher: String,
}

#[derive(Deserialize)]
pub struct AnalyzeRequest {
    #[serde(default)]
    pub queries: Vec<AdvisorQuery>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub measure: bool,
    /// Optional embedding workloads to size: each entry needs `dimensions`
    /// and `vector_count` (`profile_name` defaults to "default"). When
    /// present, vector bytes are folded into storage and dimension/tier fit
    /// is checked before any batch write.
    #[serde(default)]
    pub vectors: Vec<VectorSizingInput>,
    /// Optional target Aura tier for vector RAM checks (preset id, `<N>gb`,
    /// `<N>mb`, or `custom:<mb>`). Only used when `vectors` is non-empty.
    #[serde(default)]
    pub aura_tier: Option<String>,
}

#[derive(Serialize)]
pub struct QueryReport {
    pub name: String,
    pub findings: Vec<QueryFinding>,
}

#[derive(Serialize)]
pub struct AnalyzeResponse {
    pub queries: Vec<QueryReport>,
    pub structure: Vec<StructureNote>,
    pub sizing: SizingEstimate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurements: Option<TargetMeasurements>,
}

pub async fn analyze_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.queries.len() > MAX_ADVISOR_QUERIES {
        return Err(AppError::BadRequest(format!(
            "At most {} queries per analysis",
            MAX_ADVISOR_QUERIES
        )));
    }

    let mut queries = Vec::with_capacity(payload.queries.len());
    for query in &payload.queries {
        if query.cypher.trim().is_empty() {
            return Err(AppError::BadRequest(format!(
                "Query '{}' is empty",
                query.name
            )));
        }
        if query.cypher.len() > MAX_ADVISOR_CYPHER_LEN {
            return Err(AppError::BadRequest(format!(
                "Query '{}' exceeds {} characters",
                query.name, MAX_ADVISOR_CYPHER_LEN
            )));
        }
        queries.push(QueryReport {
            name: query.name.clone(),
            findings: analyze_query(&query.name, &query.cypher),
        });
    }

    let graph_path = state.project_graph_path(&project_id);
    let structure = analyze_structure(state.graph.as_ref(), &graph_path)?;

    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();
    let sizing = if payload.vectors.is_empty() {
        estimate_sizing(schema.node_count, schema.relationship_count)
    } else {
        let tier = payload
            .aura_tier
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(relating_migrate::parse_tier)
            .transpose()?;
        estimate_sizing_with_vectors(
            schema.node_count,
            schema.relationship_count,
            &payload.vectors,
            tier.as_ref(),
        )
    };

    let measurements = if payload.measure {
        let conn_id_str = payload.connection_id.ok_or_else(|| {
            AppError::BadRequest("measure=true requires a connection_id".to_string())
        })?;
        let conn_id = AuraConnectionId::parse(&conn_id_str).map_err(|e| {
            AppError::BadRequest(format!("Invalid Connection UUID '{}': {}", conn_id_str, e))
        })?;
        let conn = db_get_connection(&state.pool, &conn_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Connection '{}' not found", conn_id)))?;
        let backend = build_backend(&conn, None)?;
        let pairs: Vec<(String, String)> = payload
            .queries
            .iter()
            .map(|q| (q.name.clone(), q.cypher.clone()))
            .collect();
        Some(measure_target(&backend, &pairs).await?)
    } else {
        None
    };

    Ok(Json(AnalyzeResponse {
        queries,
        structure,
        sizing,
        measurements,
    }))
}
