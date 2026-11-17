use crate::error::RetrievalError;
use crate::similarity::{rank_by_keyword, rank_by_vector, reciprocal_rank_fusion};
use relating_ai::{AiProvider, EmbeddingRequest};
use relating_core::{Project, RetrievalStrategy, RetrievalTrace, RetrievedPassage};
use relating_metadata::{
    get_embedding_profile_by_name, list_chunks_for_profile, list_sources, DbPool,
};
use std::collections::HashMap;
use std::sync::Arc;

pub const DEFAULT_TOP_K: usize = 5;
pub const MAX_TOP_K: usize = 50;
pub const DEFAULT_PROFILE_NAME: &str = "default";

pub async fn run_retrieval(
    pool: &DbPool,
    provider: &Arc<dyn AiProvider>,
    project: &Project,
    query: &str,
    strategy: RetrievalStrategy,
    top_k: usize,
    profile_name: Option<&str>,
) -> Result<RetrievalTrace, RetrievalError> {
    let start_time = std::time::Instant::now();
    let top_k = top_k.clamp(1, MAX_TOP_K);

    let profile = get_embedding_profile_by_name(
        pool,
        &project.id,
        profile_name.unwrap_or(DEFAULT_PROFILE_NAME),
    )
    .await?
    .ok_or_else(|| {
        RetrievalError::NoEvidence(
            "No embedding index exists for this project yet. Generate embeddings first."
                .to_string(),
        )
    })?;

    let chunks = list_chunks_for_profile(pool, &profile.id).await?;
    if chunks.is_empty() {
        return Err(RetrievalError::NoEvidence(
            "The embedding index is empty. Generate embeddings first.".to_string(),
        ));
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

    let sources = list_sources(pool, &project.id).await?;
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
) -> Result<Vec<f32>, RetrievalError> {
    let resp = provider
        .embed(&EmbeddingRequest {
            inputs: vec![query.to_string()],
        })
        .await?;
    let embedding = resp.embeddings.into_iter().next().ok_or_else(|| {
        RetrievalError::Internal("Provider returned no query embedding".to_string())
    })?;
    if embedding.len() != *expected_dimensions {
        return Err(RetrievalError::SpaceMismatch(format!(
            "query embedding has {} dims but the index uses {} dims; embeddings come from a different model",
            embedding.len(),
            expected_dimensions
        )));
    }
    Ok(embedding)
}
