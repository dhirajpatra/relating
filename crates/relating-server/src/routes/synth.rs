use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_core::{Project, ProjectId};
use relating_metadata::{get_project as db_get_project, insert_project as db_insert_project};
use relating_synth::{
    generate_dataset, materialize_dataset, profile_graph, EdgeCase, GraphProfile,
    MaterializeOutcome, SynthConfig, SynthDataset, SynthStats,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct GenerateRequest {
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub edge_cases: Vec<EdgeCase>,
    #[serde(default)]
    pub max_nodes: Option<usize>,
    #[serde(default)]
    pub materialize: bool,
    #[serde(default)]
    pub target_name: Option<String>,
    #[serde(default)]
    pub include_dataset: bool,
}

fn default_scale() -> f64 {
    1.0
}

#[derive(Serialize)]
pub struct GenerateResponse {
    pub profile_summary: ProfileSummary,
    pub stats: SynthStats,
    pub notes: Vec<String>,
    pub sample_nodes: Vec<serde_json::Value>,
    pub sample_rels: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<SynthDataset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialized: Option<MaterializeOutcome>,
}

#[derive(Serialize)]
pub struct ProfileSummary {
    pub labels: Vec<String>,
    pub rel_types: Vec<String>,
    pub total_nodes: u64,
    pub total_relationships: u64,
}

pub async fn profile_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<GraphProfile>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let graph_path = state.project_graph_path(&project_id);
    let profile = profile_graph(state.graph.as_ref(), &graph_path)?;
    Ok(Json(profile))
}

pub async fn generate_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<GenerateRequest>,
) -> Result<(StatusCode, Json<GenerateResponse>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let graph_path = state.project_graph_path(&project_id);
    let profile = profile_graph(state.graph.as_ref(), &graph_path)?;

    let config = SynthConfig {
        scale: payload.scale,
        seed: payload.seed,
        edge_cases: payload.edge_cases,
        max_nodes: payload.max_nodes.unwrap_or(50_000),
    };
    let dataset = generate_dataset(&profile, &config)?;

    let sample_nodes: Vec<serde_json::Value> = dataset
        .nodes
        .iter()
        .take(5)
        .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
        .collect();
    let sample_rels: Vec<serde_json::Value> = dataset
        .rels
        .iter()
        .take(5)
        .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
        .collect();

    // Materialize into an isolated sibling project, never into the source.
    let (target_project_id, materialized) = if payload.materialize {
        let now = Utc::now();
        let target = Project {
            id: ProjectId::new(),
            name: payload
                .target_name
                .unwrap_or_else(|| format!("{}-synth-{}", project.name, payload.seed)),
            description: format!(
                "Synthetic fixture generated from '{}' (seed {})",
                project.name, payload.seed
            ),
            created_at: now,
            updated_at: now,
            ai_config: project.ai_config.clone(),
        };
        db_insert_project(&state.pool, &target).await?;
        let target_graph_path = state.project_graph_path(&target.id);
        let outcome = materialize_dataset(state.graph.as_ref(), &target_graph_path, &dataset)?;
        (Some(target.id.to_string()), Some(outcome))
    } else {
        (None, None)
    };

    Ok((
        StatusCode::CREATED,
        Json(GenerateResponse {
            profile_summary: ProfileSummary {
                labels: profile.labels.iter().map(|l| l.label.clone()).collect(),
                rel_types: profile
                    .rel_types
                    .iter()
                    .map(|r| r.rel_type.clone())
                    .collect(),
                total_nodes: profile.total_nodes,
                total_relationships: profile.total_relationships,
            },
            stats: dataset.stats.clone(),
            notes: dataset.notes.clone(),
            sample_nodes,
            sample_rels,
            dataset: if payload.include_dataset {
                Some(dataset)
            } else {
                None
            },
            target_project_id,
            materialized,
        }),
    ))
}
