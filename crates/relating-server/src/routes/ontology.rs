use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_ai::create_ai_provider;
use relating_core::{Ontology, OntologyId, OntologyStatus, ProjectId};
use relating_ingest::{propose_ontology, SourceProfile};
use relating_metadata::{
    approve_ontology as db_approve, get_approved_ontology, get_latest_ontology,
    get_project as db_get_project, insert_ontology, list_sources,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ProposeOntologyRequest {
    #[serde(default)]
    pub domain_hint: Option<String>,
}

#[derive(Deserialize)]
pub struct ApproveOntologyRequest {
    pub ontology_id: String,
}

#[derive(Deserialize)]
pub struct ImportOntologyRequest {
    pub schema: relating_core::OntologySchema,
}

#[derive(Serialize)]
pub struct OntologyStatusResponse {
    pub latest: Option<Ontology>,
    pub approved: Option<Ontology>,
}

pub async fn propose_ontology_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ProposeOntologyRequest>,
) -> Result<(StatusCode, Json<Ontology>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let sources = list_sources(&state.pool, &project_id).await?;
    if sources.is_empty() {
        return Err(AppError::BadRequest(
            "Please upload at least one source file before proposing an ontology".to_string(),
        ));
    }

    let mut profiles = Vec::new();
    for s in &sources {
        let field_names: Vec<String> = s
            .metadata
            .get("field_names")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|val| val.as_str().map(|str_val| str_val.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let total_records = s
            .metadata
            .get("total_records")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let sample_records = s
            .metadata
            .get("sample_records")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|val| {
                        val.as_object()
                            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();

        profiles.push((
            s.filename.clone(),
            SourceProfile {
                total_records,
                field_names,
                sample_records,
            },
        ));
    }

    let ai_provider = create_ai_provider(&project.ai_config)?;
    let schema = propose_ontology(ai_provider, &profiles, payload.domain_hint.as_deref()).await?;

    let prev_latest = get_latest_ontology(&state.pool, &project_id).await?;
    let version = prev_latest.map(|o| o.version + 1).unwrap_or(1);

    let ontology = Ontology {
        id: OntologyId::new(),
        project_id,
        version,
        status: OntologyStatus::Proposed,
        schema,
        created_at: Utc::now(),
        approved_at: None,
    };

    insert_ontology(&state.pool, &ontology).await?;

    Ok((StatusCode::CREATED, Json(ontology)))
}

pub async fn get_ontology_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<OntologyStatusResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let latest = get_latest_ontology(&state.pool, &project_id).await?;
    let approved = get_approved_ontology(&state.pool, &project_id).await?;

    Ok(Json(OntologyStatusResponse { latest, approved }))
}

/// Import a fixed ontology schema as a new proposal (deterministic seeding
/// and schema-as-code round-trips). The schema is validated structurally —
/// non-empty entities with unique labels, relationship endpoints that
/// reference known labels — then stored as `Proposed`; it still needs
/// explicit approval before ingestion, exactly like an LLM proposal.
pub async fn import_ontology_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ImportOntologyRequest>,
) -> Result<(StatusCode, Json<Ontology>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let _project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let schema = payload.schema;
    if schema.entities.is_empty() {
        return Err(AppError::BadRequest(
            "Ontology schema must define at least one entity".to_string(),
        ));
    }
    let mut labels = std::collections::HashSet::new();
    for entity in &schema.entities {
        if entity.label.trim().is_empty() {
            return Err(AppError::BadRequest(
                "Ontology entity labels cannot be empty".to_string(),
            ));
        }
        if !labels.insert(entity.label.clone()) {
            return Err(AppError::BadRequest(format!(
                "Duplicate ontology entity label '{}'",
                entity.label
            )));
        }
    }
    for rel in &schema.relationships {
        if rel.rel_type.trim().is_empty() {
            return Err(AppError::BadRequest(
                "Ontology relationship types cannot be empty".to_string(),
            ));
        }
        for endpoint in [&rel.source_label, &rel.target_label] {
            if !labels.contains(endpoint) {
                return Err(AppError::BadRequest(format!(
                    "Relationship '{}' references unknown entity label '{}'",
                    rel.rel_type, endpoint
                )));
            }
        }
    }

    let prev_latest = get_latest_ontology(&state.pool, &project_id).await?;
    let version = prev_latest.map(|o| o.version + 1).unwrap_or(1);

    let ontology = Ontology {
        id: OntologyId::new(),
        project_id,
        version,
        status: OntologyStatus::Proposed,
        schema,
        created_at: Utc::now(),
        approved_at: None,
    };

    insert_ontology(&state.pool, &ontology).await?;

    Ok((StatusCode::CREATED, Json(ontology)))
}

pub async fn approve_ontology_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ApproveOntologyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let ontology_id = OntologyId::parse(&payload.ontology_id).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid Ontology UUID '{}': {}",
            payload.ontology_id, e
        ))
    })?;

    let approved = db_approve(&state.pool, &ontology_id).await?;
    if !approved {
        return Err(AppError::NotFound(format!(
            "Ontology with ID '{}' not found",
            ontology_id
        )));
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "approved_ontology_id": ontology_id.to_string(),
        "message": "Ontology approved successfully. Ready for extraction and graph loading."
    })))
}
