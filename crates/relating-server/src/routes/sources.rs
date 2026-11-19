use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_core::{ProjectId, RegisterSourceRequest, Source, SourceId, SourceStatus};
use relating_ingest::{compute_sha256, parse_source_content};
use relating_metadata::{
    delete_chunks_for_source, delete_source as db_delete_source, get_project as db_get_project,
    get_source as db_get_source, insert_source, list_sources as db_list_sources,
};

pub async fn register_source_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<RegisterSourceRequest>,
) -> Result<(StatusCode, Json<Source>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    // Verify project exists
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let filename = payload.filename.trim();
    if filename.is_empty() {
        return Err(AppError::BadRequest("Filename cannot be empty".to_string()));
    }

    // Binary uploads (PDF/DOCX) arrive as base64; text uploads use `content`.
    let raw_bytes: Vec<u8> = match payload.content_base64.as_deref() {
        Some(encoded) if !encoded.trim().is_empty() => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| AppError::BadRequest(format!("Invalid base64 content: {}", e)))?
        }
        _ => payload.content.as_bytes().to_vec(),
    };
    if raw_bytes.is_empty() {
        return Err(AppError::BadRequest(
            "Source content is empty: provide 'content' or 'content_base64'".to_string(),
        ));
    }
    let content_hash = compute_sha256(&raw_bytes);
    let byte_size = raw_bytes.len() as u64;
    let source_id = SourceId::new();

    // Parse and profile source
    let (_records, profile) = parse_source_content(
        source_id.clone(),
        filename,
        payload.content_type.as_deref(),
        &raw_bytes,
    )?;

    // Store source file on disk
    let sources_dir = state.project_dir(&project_id).join("sources");
    if !sources_dir.exists() {
        let _ = std::fs::create_dir_all(&sources_dir);
    }
    let file_dest = sources_dir.join(format!("{}_{}", source_id, filename));
    let _ = std::fs::write(&file_dest, &raw_bytes);

    let metadata = serde_json::json!({
        "total_records": profile.total_records,
        "field_names": profile.field_names,
        "sample_records": profile.sample_records,
        "stored_path": file_dest.to_string_lossy(),
    });

    let source = Source {
        id: source_id,
        project_id,
        filename: filename.to_string(),
        content_hash,
        content_type: payload
            .content_type
            .unwrap_or_else(|| "text/plain".to_string()),
        byte_size,
        status: SourceStatus::Pending,
        metadata,
        created_at: Utc::now(),
    };

    insert_source(&state.pool, &source).await?;

    Ok((StatusCode::CREATED, Json(source)))
}

pub async fn list_sources_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<Source>>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let sources = db_list_sources(&state.pool, &project_id).await?;
    Ok(Json(sources))
}

pub async fn delete_source_handler(
    State(state): State<AppState>,
    Path((id_str, source_id_str)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let source_id = SourceId::parse(&source_id_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Source UUID '{}': {}", source_id_str, e))
    })?;

    // Try finding stored path from source metadata before deleting
    if let Ok(Some(source)) = db_get_source(&state.pool, &source_id).await {
        if let Some(path_str) = source.metadata.get("stored_path").and_then(|v| v.as_str()) {
            let path = std::path::Path::new(path_str);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let deleted = db_delete_source(&state.pool, &source_id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "Source '{}' not found",
            source_id
        )));
    }

    // Remove embedding chunks so deleted sources stop appearing in retrieval
    let _ = delete_chunks_for_source(&state.pool, &source_id).await;

    Ok(StatusCode::NO_CONTENT)
}
