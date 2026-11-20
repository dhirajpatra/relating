use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_core::{
    Project, ProjectId, RetrievalConfig, RetrievalStrategy, SchemaRevision, SchemaRevisionId,
};
use relating_metadata::{
    get_latest_schema_revision, get_project as db_get_project, get_retrieval_config,
    get_schema_revision_by_version, insert_schema_revision, list_schema_revisions,
    upsert_retrieval_config,
};
use relating_schema::{
    check_conformance, diff_documents, propose_migration, ConformanceReport, MigrationProposal,
    SchemaDiff, SchemaDocument,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct SchemaFile {
    pub path: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct ExportResponse {
    pub files: Vec<SchemaFile>,
    pub document: SchemaDocument,
}

#[derive(Deserialize)]
pub struct CreateRevisionRequest {
    #[serde(default)]
    pub document: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct CreateRevisionResponse {
    pub revision: SchemaRevision,
    pub created: bool,
}

#[derive(Deserialize)]
pub struct DiffRequest {
    #[serde(default)]
    pub from_version: Option<u32>,
    #[serde(default)]
    pub to_version: Option<u32>,
    #[serde(default)]
    pub to_document: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct DiffResponse {
    pub diff: SchemaDiff,
    pub proposal: MigrationProposal,
}

#[derive(Deserialize)]
pub struct ApplyRequest {
    pub document: serde_json::Value,
}

#[derive(Serialize)]
pub struct ApplyResponse {
    pub revision: SchemaRevision,
    pub diff: Option<SchemaDiff>,
    pub conformance: ConformanceReport,
}

fn parse_project(id_str: &str) -> Result<ProjectId, AppError> {
    ProjectId::parse(id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))
}

async fn require_project(state: &AppState, id: &ProjectId) -> Result<Project, AppError> {
    db_get_project(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))
}

fn parse_document(value: &serde_json::Value) -> Result<SchemaDocument, AppError> {
    let document: SchemaDocument = serde_json::from_value(value.clone())
        .map_err(|e| AppError::BadRequest(format!("Schema document is not valid: {}", e)))?;
    document.validate()?;
    Ok(document)
}

async fn live_document(state: &AppState, project: &Project) -> Result<SchemaDocument, AppError> {
    let graph_path = state.project_graph_path(&project.id);
    let document = relating_schema::build_live_document(
        &state.pool,
        state.graph.as_ref(),
        &project.id,
        &graph_path,
        Some(project.name.clone()),
    )
    .await?;
    Ok(document)
}

fn export_file_list(document: &SchemaDocument) -> Result<Vec<SchemaFile>, AppError> {
    Ok(relating_schema::export_files(document)?
        .into_iter()
        .map(|(path, content)| SchemaFile { path, content })
        .collect())
}

pub async fn export_schema_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<ExportResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    let project = require_project(&state, &project_id).await?;
    let document = live_document(&state, &project).await?;

    let files = export_file_list(&document)?;

    Ok(Json(ExportResponse { files, document }))
}

pub async fn create_revision_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CreateRevisionRequest>,
) -> Result<(StatusCode, Json<CreateRevisionResponse>), AppError> {
    let project_id = parse_project(&id_str)?;
    let project = require_project(&state, &project_id).await?;

    let document = match payload.document {
        Some(value) => parse_document(&value)?,
        None => live_document(&state, &project).await?,
    };
    let hash = document.content_hash()?;

    // Idempotent snapshots: identical content returns the existing revision.
    if let Some(latest) = get_latest_schema_revision(&state.pool, &project_id).await? {
        if latest.content_hash == hash {
            return Ok((
                StatusCode::OK,
                Json(CreateRevisionResponse {
                    revision: latest,
                    created: false,
                }),
            ));
        }
        let revision = SchemaRevision {
            id: SchemaRevisionId::new(),
            project_id,
            version: latest.version + 1,
            content_hash: hash,
            document: serde_json::to_value(&document)
                .map_err(|e| AppError::Internal(format!("Failed to serialize document: {}", e)))?,
            created_at: Utc::now(),
        };
        insert_schema_revision(&state.pool, &revision).await?;
        return Ok((
            StatusCode::CREATED,
            Json(CreateRevisionResponse {
                revision,
                created: true,
            }),
        ));
    }

    let revision = SchemaRevision {
        id: SchemaRevisionId::new(),
        project_id,
        version: 1,
        content_hash: hash,
        document: serde_json::to_value(&document)
            .map_err(|e| AppError::Internal(format!("Failed to serialize document: {}", e)))?,
        created_at: Utc::now(),
    };
    insert_schema_revision(&state.pool, &revision).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateRevisionResponse {
            revision,
            created: true,
        }),
    ))
}

pub async fn list_revisions_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<SchemaRevision>>, AppError> {
    let project_id = parse_project(&id_str)?;
    let revisions = list_schema_revisions(&state.pool, &project_id).await?;
    Ok(Json(revisions))
}

pub async fn get_revision_handler(
    State(state): State<AppState>,
    Path((id_str, version)): Path<(String, u32)>,
) -> Result<Json<SchemaRevision>, AppError> {
    let project_id = parse_project(&id_str)?;
    let revision = get_schema_revision_by_version(&state.pool, &project_id, version)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Schema revision v{} not found", version)))?;
    Ok(Json(revision))
}

pub async fn diff_schema_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<DiffRequest>,
) -> Result<Json<DiffResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    let project = require_project(&state, &project_id).await?;

    let from_revision = match payload.from_version {
        Some(version) => Some(
            get_schema_revision_by_version(&state.pool, &project_id, version)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("Schema revision v{} not found", version))
                })?,
        ),
        None => get_latest_schema_revision(&state.pool, &project_id).await?,
    };
    let from_revision = from_revision.ok_or_else(|| {
        AppError::BadRequest(
            "No schema revisions exist yet: snapshot the current schema first".to_string(),
        )
    })?;
    let from_doc = parse_document(&from_revision.document)?;

    let to_doc = if let Some(value) = payload.to_document {
        parse_document(&value)?
    } else if let Some(version) = payload.to_version {
        let revision = get_schema_revision_by_version(&state.pool, &project_id, version)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Schema revision v{} not found", version)))?;
        parse_document(&revision.document)?
    } else {
        live_document(&state, &project).await?
    };

    let diff = diff_documents(&from_doc, &to_doc);
    let latest = get_latest_schema_revision(&state.pool, &project_id).await?;
    let next_version = latest.map(|r| r.version + 1).unwrap_or(1);
    let proposal = propose_migration(&diff, next_version);

    Ok(Json(DiffResponse { diff, proposal }))
}

pub async fn apply_schema_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<ApplyRequest>,
) -> Result<(StatusCode, Json<ApplyResponse>), AppError> {
    let project_id = parse_project(&id_str)?;
    let _ = require_project(&state, &project_id).await?;
    let document = parse_document(&payload.document)?;

    // Diff against the previous revision when one exists.
    let latest = get_latest_schema_revision(&state.pool, &project_id).await?;
    let diff = match &latest {
        Some(revision) => {
            let from_doc = parse_document(&revision.document)?;
            Some(diff_documents(&from_doc, &document))
        }
        None => None,
    };

    // Conformance gates nothing here (R4 CI enforces); it is reported.
    let graph_path = state.project_graph_path(&project_id);
    let conformance = check_conformance(state.graph.as_ref(), &graph_path, &document, 200)?;

    let hash = document.content_hash()?;
    let version = latest.map(|r| r.version + 1).unwrap_or(1);
    let now = Utc::now();
    let revision = SchemaRevision {
        id: SchemaRevisionId::new(),
        project_id,
        version,
        content_hash: hash,
        document: serde_json::to_value(&document)
            .map_err(|e| AppError::Internal(format!("Failed to serialize document: {}", e)))?,
        created_at: now,
    };
    insert_schema_revision(&state.pool, &revision).await?;

    Ok((
        StatusCode::CREATED,
        Json(ApplyResponse {
            revision,
            diff,
            conformance,
        }),
    ))
}

#[derive(Serialize)]
pub struct RetrievalConfigResponse {
    pub configured: bool,
    pub default_strategy: RetrievalStrategy,
    pub default_top_k: usize,
}

#[derive(Deserialize)]
pub struct UpdateRetrievalConfigRequest {
    #[serde(default)]
    pub default_strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub default_top_k: Option<usize>,
}

pub async fn get_retrieval_config_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<RetrievalConfigResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;

    match get_retrieval_config(&state.pool, &project_id).await? {
        Some(config) => Ok(Json(RetrievalConfigResponse {
            configured: true,
            default_strategy: config.default_strategy,
            default_top_k: config.default_top_k,
        })),
        None => Ok(Json(RetrievalConfigResponse {
            configured: false,
            default_strategy: RetrievalStrategy::Hybrid,
            default_top_k: 5,
        })),
    }
}

pub async fn update_retrieval_config_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<UpdateRetrievalConfigRequest>,
) -> Result<Json<RetrievalConfigResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;

    if let Some(top_k) = payload.default_top_k {
        if top_k == 0 || top_k > 50 {
            return Err(AppError::BadRequest(
                "default_top_k must be between 1 and 50".to_string(),
            ));
        }
    }

    let existing = get_retrieval_config(&state.pool, &project_id).await?;
    let config = RetrievalConfig {
        project_id: project_id.clone(),
        default_strategy: payload.default_strategy.unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|c| c.default_strategy.clone())
                .unwrap_or_default()
        }),
        default_top_k: payload
            .default_top_k
            .unwrap_or_else(|| existing.as_ref().map(|c| c.default_top_k).unwrap_or(5)),
        updated_at: Utc::now(),
    };
    upsert_retrieval_config(&state.pool, &config).await?;

    Ok(Json(RetrievalConfigResponse {
        configured: true,
        default_strategy: config.default_strategy,
        default_top_k: config.default_top_k,
    }))
}
