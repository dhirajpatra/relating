use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_ai::{create_ai_provider, AiProvider, EmbeddingRequest};
use relating_core::{Project, ProjectId, RetrievalStrategy, RetrievalTrace, RetrievedPassage};
use relating_metadata::{
    get_embedding_profile_by_name, get_project as db_get_project, list_chunks_for_profile,
    list_sources,
};
use relating_retrieval::{rank_by_keyword, rank_by_vector, reciprocal_rank_fusion, RetrievalError};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

pub const DEFAULT_TOP_K: usize = 5;
pub const MAX_TOP_K: usize = 50;

#[derive(Deserialize)]
pub struct RetrievalRequest {
    pub query: String,
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

pub async fn run_retrieval(
    state: &AppState,
    project: &Project,
    provider: &Arc<dyn AiProvider>,
    query: &str,
    strategy: RetrievalStrategy,
    top_k: usize,
    profile_name: Option<&str>,
) -> Result<RetrievalTrace, AppError> {
    let start_time = std::time::Instant::now();
    let top_k = top_k.clamp(1, MAX_TOP_K);

    let profile = get_embedding_profile_by_name(
        &state.pool,
        &project.id,
        profile_name.unwrap_or(crate::routes::embeddings::DEFAULT_PROFILE_NAME),
    )
    .await?
    .ok_or_else(|| {
        RetrievalError::NoEvidence(
            "No embedding index exists for this project yet. Generate embeddings first."
                .to_string(),
        )
    })?;

    let chunks = list_chunks_for_profile(&state.pool, &profile.id).await?;
    if chunks.is_empty() {
        return Err(RetrievalError::NoEvidence(
            "The embedding index is empty. Generate embeddings first.".to_string(),
        )
        .into());
    }

    let ranked: Vec<(usize, f32)> = match strategy {
        RetrievalStrategy::Vector => {
            let query_embedding = embed_query(provider, query, &profile.dimensions).await?;
            rank_by_vector(&query_embedding, &chunks)?
        }
        RetrievalStrategy::Keyword => rank_by_keyword(query, &chunks),
        RetrievalStrategy::Hybrid => {
            let query_embedding = embed_query(provider, query, &profile.dimensions).await?;
            let vector_ranking = rank_by_vector(&query_embedding, &chunks)?;
            let keyword_ranking = rank_by_keyword(query, &chunks);
            reciprocal_rank_fusion(&[vector_ranking, keyword_ranking], 60.0)
        }
    };

    let sources = list_sources(&state.pool, &project.id).await?;
    let filenames: HashMap<String, String> = sources
        .into_iter()
        .map(|s| (s.id.to_string(), s.filename))
        .collect();

    let mut passages = Vec::new();
    for (rank, (idx, score)) in ranked.into_iter().take(top_k).enumerate() {
        let chunk = &chunks[idx];
        passages.push(RetrievedPassage {
            chunk_id: chunk.id.clone(),
            source_id: chunk.source_id.clone(),
            filename: filenames
                .get(&chunk.source_id.to_string())
                .cloned()
                .unwrap_or_else(|| "unknown source".to_string()),
            score,
            rank: (rank + 1) as u32,
            content: chunk.content.clone(),
            metadata: chunk.metadata.clone(),
        });
    }

    Ok(RetrievalTrace {
        query: query.to_string(),
        strategy,
        profile_id: profile.id,
        profile_name: profile.name,
        embedding_model: profile.model,
        dimensions: profile.dimensions,
        top_k,
        passages,
        latency_ms: start_time.elapsed().as_secs_f64() * 1000.0,
    })
}

async fn embed_query(
    provider: &Arc<dyn AiProvider>,
    query: &str,
    expected_dimensions: &usize,
) -> Result<Vec<f32>, AppError> {
    let resp = provider
        .embed(&EmbeddingRequest {
            inputs: vec![query.to_string()],
        })
        .await?;
    let embedding =
        resp.embeddings.into_iter().next().ok_or_else(|| {
            AppError::Internal("Provider returned no query embedding".to_string())
        })?;
    if embedding.len() != *expected_dimensions {
        return Err(RetrievalError::SpaceMismatch(format!(
            "query embedding has {} dims but the index uses {} dims; embeddings come from a different model",
            embedding.len(),
            expected_dimensions
        ))
        .into());
    }
    Ok(embedding)
}

pub async fn run_retrieval_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<RetrievalRequest>,
) -> Result<Json<RetrievalTrace>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("Query cannot be empty".to_string()));
    }

    let provider = create_ai_provider(&project.ai_config)?;
    let trace = run_retrieval(
        &state,
        &project,
        &provider,
        payload.query.trim(),
        payload.strategy.unwrap_or_default(),
        payload.top_k.unwrap_or(DEFAULT_TOP_K),
        payload.profile_name.as_deref(),
    )
    .await?;

    Ok(Json(trace))
}
