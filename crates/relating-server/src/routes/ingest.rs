use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_core::ProjectId;
use relating_ingest::{
    extract_entities_and_relationships, load_extracted_to_graph, parse_source_content,
};
use relating_metadata::{get_approved_ontology, get_project as db_get_project, list_sources};
use serde::Serialize;
use tracing::info;

#[derive(Serialize)]
pub struct IngestionResultResponse {
    pub sources_processed: usize,
    pub nodes_created: i64,
    pub relationships_created: i64,
    pub properties_set: i64,
    pub elapsed_ms: f64,
}

pub async fn run_ingestion_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<IngestionResultResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    // 1. Verify project exists
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    // 2. Enforce ontology approval gate
    let approved_ontology = get_approved_ontology(&state.pool, &project_id)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(
                "An approved ontology is required before extracting into the graph. Please review and approve an ontology proposal first.".to_string(),
            )
        })?;

    // 3. List all registered sources for project
    let sources = list_sources(&state.pool, &project_id).await?;
    if sources.is_empty() {
        return Err(AppError::BadRequest(
            "No data sources found in this project to ingest".to_string(),
        ));
    }

    let graph_path = state.project_graph_path(&project_id);
    let mut all_nodes = Vec::new();
    let mut all_rels = Vec::new();
    let start_time = std::time::Instant::now();

    for source in &sources {
        let stored_path_opt = source.metadata.get("stored_path").and_then(|v| v.as_str());

        let raw_bytes = if let Some(path_str) = stored_path_opt {
            std::fs::read(path_str).map_err(|e| {
                AppError::Internal(format!(
                    "Failed to read stored source file '{}': {}",
                    path_str, e
                ))
            })?
        } else {
            continue;
        };

        let (records, _profile) = parse_source_content(
            source.id.clone(),
            &source.filename,
            Some(&source.content_type),
            &raw_bytes,
        )?;

        let (nodes, rels) = extract_entities_and_relationships(
            source.id.clone(),
            &source.filename,
            &records,
            &approved_ontology.schema,
        )?;

        all_nodes.extend(nodes);
        all_rels.extend(rels);
    }

    info!(
        "Extracted {} candidate nodes and {} candidate relationships",
        all_nodes.len(),
        all_rels.len()
    );

    // 4. Load into Relate database
    let write_res =
        load_extracted_to_graph(state.graph.as_ref(), &graph_path, &all_nodes, &all_rels)?;

    let total_elapsed = start_time.elapsed().as_secs_f64() * 1000.0;

    Ok(Json(IngestionResultResponse {
        sources_processed: sources.len(),
        nodes_created: write_res.nodes_created,
        relationships_created: write_res.relationships_created,
        properties_set: write_res.properties_set,
        elapsed_ms: total_elapsed,
    }))
}
