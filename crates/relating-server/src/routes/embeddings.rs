use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_ai::{create_ai_provider, EmbeddingRequest};
use relating_core::{Chunk, ChunkId, EmbeddingProfile, EmbeddingProfileId, ProjectId};
use relating_ingest::parse_source_content;
use relating_metadata::{
    chunk_exists, count_chunks_for_profile, get_embedding_profile_by_name,
    get_project as db_get_project, insert_chunk, insert_embedding_profile, list_embedding_profiles,
    list_sources,
};
use relating_retrieval::{chunk_record, chunk_text, ChunkCandidate, ChunkConfig};
use serde::{Deserialize, Serialize};
use tracing::info;

pub const DEFAULT_PROFILE_NAME: &str = "default";
const EMBED_BATCH_SIZE: usize = 32;

#[derive(Deserialize)]
pub struct GenerateEmbeddingsRequest {
    #[serde(default)]
    pub profile_name: Option<String>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub overlap_chars: Option<usize>,
}

#[derive(Serialize)]
pub struct GenerateEmbeddingsResponse {
    pub profile: EmbeddingProfile,
    pub sources_processed: usize,
    pub chunks_created: usize,
    pub chunks_skipped: usize,
    pub dimensions: usize,
    pub elapsed_ms: f64,
}

#[derive(Serialize)]
pub struct EmbeddingProfileStatus {
    pub profile: EmbeddingProfile,
    pub chunk_count: u64,
}

#[derive(Serialize)]
pub struct EmbeddingStatusResponse {
    pub profiles: Vec<EmbeddingProfileStatus>,
}

fn is_structured_source(filename: &str, content_type: &str) -> bool {
    let lower_name = filename.to_lowercase();
    lower_name.ends_with(".csv")
        || lower_name.ends_with(".json")
        || content_type.contains("csv")
        || content_type.contains("json")
}

pub async fn generate_embeddings_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<GenerateEmbeddingsRequest>,
) -> Result<(StatusCode, Json<GenerateEmbeddingsResponse>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let sources = list_sources(&state.pool, &project_id).await?;
    if sources.is_empty() {
        return Err(AppError::BadRequest(
            "No data sources found in this project to embed".to_string(),
        ));
    }

    let profile_name = payload
        .profile_name
        .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string());
    let chunk_config = ChunkConfig {
        max_chars: payload.max_chars.unwrap_or(1200).clamp(100, 8000),
        overlap_chars: payload.overlap_chars.unwrap_or(150).clamp(0, 1000),
    };

    // 1. Build chunk candidates from every source
    let mut pending: Vec<(relating_core::SourceId, ChunkCandidate)> = Vec::new();
    let mut ordinal = 0u32;
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

        if is_structured_source(&source.filename, &source.content_type) {
            for record in &records {
                let mut candidate = chunk_record(ordinal, &record.lineage_ref, &record.fields);
                candidate.ordinal = ordinal;
                pending.push((source.id.clone(), candidate));
                ordinal += 1;
            }
        } else {
            let joined = records
                .iter()
                .filter_map(|r| r.fields.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n");
            for mut candidate in chunk_text(&joined, Some(&source.filename), &chunk_config) {
                candidate.ordinal = ordinal;
                pending.push((source.id.clone(), candidate));
                ordinal += 1;
            }
        }
    }

    if pending.is_empty() {
        return Err(AppError::BadRequest(
            "No chunkable content found in this project's sources".to_string(),
        ));
    }

    // 2. Resolve the embedding profile (dimensions are discovered from the
    // provider on first use and enforced on every later call).
    let existing = get_embedding_profile_by_name(&state.pool, &project_id, &profile_name).await?;
    let mut chunks_skipped = 0usize;
    let mut to_embed: Vec<(relating_core::SourceId, ChunkCandidate)> = Vec::new();
    if let Some(ref profile) = existing {
        for (source_id, candidate) in pending {
            if chunk_exists(&state.pool, &profile.id, &candidate.content_hash).await? {
                chunks_skipped += 1;
            } else {
                to_embed.push((source_id, candidate));
            }
        }
    } else {
        to_embed = pending;
    }

    let start_time = std::time::Instant::now();
    let provider = create_ai_provider(&project.ai_config)?;

    // 3. Embed new candidates in batches
    let mut new_embeddings: Vec<Vec<f32>> = Vec::with_capacity(to_embed.len());
    for batch in to_embed.chunks(EMBED_BATCH_SIZE) {
        let inputs: Vec<String> = batch.iter().map(|(_, c)| c.content.clone()).collect();
        let resp = provider.embed(&EmbeddingRequest { inputs }).await?;
        if resp.embeddings.len() != batch.len() {
            return Err(AppError::Internal(format!(
                "Provider returned {} embeddings for {} inputs",
                resp.embeddings.len(),
                batch.len()
            )));
        }
        new_embeddings.extend(resp.embeddings);
    }

    // 4. Create the profile on first use, or enforce dimension compatibility
    let profile = match existing {
        Some(profile) => {
            for embedding in &new_embeddings {
                if embedding.len() != profile.dimensions {
                    return Err(relating_retrieval::RetrievalError::SpaceMismatch(format!(
                        "provider returned {}-dim embeddings but profile '{}' uses {} dims (model '{}')",
                        embedding.len(),
                        profile.name,
                        profile.dimensions,
                        profile.model
                    ))
                    .into());
                }
            }
            profile
        }
        None => {
            let dimensions = new_embeddings.first().map(|e| e.len()).unwrap_or(0);
            if dimensions == 0 {
                return Err(AppError::Internal(
                    "Provider returned empty embeddings".to_string(),
                ));
            }
            let provider_name = format!("{:?}", project.ai_config.provider_type).to_lowercase();
            let profile = EmbeddingProfile {
                id: EmbeddingProfileId::new(),
                project_id: project_id.clone(),
                name: profile_name,
                provider: provider_name,
                model: project.ai_config.embedding_model.clone(),
                model_version: None,
                dimensions,
                modality: "mixed".to_string(),
                created_at: Utc::now(),
            };
            insert_embedding_profile(&state.pool, &profile).await?;
            info!(
                "Created embedding profile '{}' ({} dims, model '{}')",
                profile.name, profile.dimensions, profile.model
            );
            profile
        }
    };

    // 5. Persist new chunks (unique on profile + content hash, so retries are safe)
    let mut chunks_created = 0usize;
    for ((source_id, candidate), embedding) in to_embed.into_iter().zip(new_embeddings) {
        let chunk = Chunk {
            id: ChunkId::new(),
            project_id: project_id.clone(),
            profile_id: profile.id.clone(),
            source_id,
            ordinal: candidate.ordinal,
            content: candidate.content,
            content_hash: candidate.content_hash,
            metadata: candidate.metadata,
            embedding,
            created_at: Utc::now(),
        };
        insert_chunk(&state.pool, &chunk).await?;
        chunks_created += 1;
    }

    let total_elapsed = start_time.elapsed().as_secs_f64() * 1000.0;
    info!(
        "Embeddings generated for project {}: {} created, {} skipped ({} dims)",
        project_id, chunks_created, chunks_skipped, profile.dimensions
    );

    Ok((
        StatusCode::CREATED,
        Json(GenerateEmbeddingsResponse {
            dimensions: profile.dimensions,
            profile,
            sources_processed: sources.len(),
            chunks_created,
            chunks_skipped,
            elapsed_ms: total_elapsed,
        }),
    ))
}

pub async fn get_embeddings_status_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<EmbeddingStatusResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let profiles = list_embedding_profiles(&state.pool, &project_id).await?;
    let mut out = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let chunk_count = count_chunks_for_profile(&state.pool, &profile.id).await?;
        out.push(EmbeddingProfileStatus {
            profile,
            chunk_count,
        });
    }

    Ok(Json(EmbeddingStatusResponse { profiles: out }))
}
