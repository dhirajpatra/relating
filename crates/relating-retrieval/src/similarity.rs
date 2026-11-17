use crate::error::RetrievalError;
use relating_core::Chunk;
use std::collections::{HashMap, HashSet};

/// Cosine similarity between two embeddings. Vectors from different
/// embedding spaces (different dimensions) are never compared.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Result<f32, RetrievalError> {
    if a.len() != b.len() {
        return Err(RetrievalError::SpaceMismatch(format!(
            "cannot compare {}-dim vector with {}-dim vector",
            a.len(),
            b.len()
        )));
    }
    if a.is_empty() {
        return Ok(0.0);
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return Ok(0.0);
    }
    Ok(dot / denom)
}

/// Rank chunk indexes by vector similarity (descending). Every chunk must
/// share the query embedding space; a single mismatch aborts the ranking.
pub fn rank_by_vector(
    query_embedding: &[f32],
    chunks: &[Chunk],
) -> Result<Vec<(usize, f32)>, RetrievalError> {
    let mut scored = Vec::with_capacity(chunks.len());
    for (idx, chunk) in chunks.iter().enumerate() {
        let score = cosine_similarity(query_embedding, &chunk.embedding)?;
        scored.push((idx, score));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored)
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(|t| t.to_lowercase())
        .collect()
}

/// Normalized token-overlap score in [0, 1]: fraction of distinct query
/// tokens present in the chunk. Deterministic lexical fallback/reranker.
pub fn keyword_score(query: &str, content: &str) -> f32 {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let content_tokens = tokenize(content);
    let hits = query_tokens.intersection(&content_tokens).count();
    hits as f32 / query_tokens.len() as f32
}

pub fn rank_by_keyword(query: &str, chunks: &[Chunk]) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = chunks
        .iter()
        .enumerate()
        .map(|(idx, chunk)| (idx, keyword_score(query, &chunk.content)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

/// Reciprocal Rank Fusion over per-strategy rankings. Each ranking maps a
/// chunk index to its 1-based rank; fused score = sum(1 / (k + rank)).
pub fn reciprocal_rank_fusion(rankings: &[Vec<(usize, f32)>], k: f32) -> Vec<(usize, f32)> {
    let mut fused: HashMap<usize, f32> = HashMap::new();
    for ranking in rankings {
        for (position, (idx, _)) in ranking.iter().enumerate() {
            let rank = (position + 1) as f32;
            *fused.entry(*idx).or_insert(0.0) += 1.0 / (k + rank);
        }
    }
    let mut out: Vec<(usize, f32)> = fused.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_core::{ChunkId, EmbeddingProfileId, ProjectId, SourceId};

    fn test_chunk(content: &str, embedding: Vec<f32>) -> Chunk {
        Chunk {
            id: ChunkId::new(),
            project_id: ProjectId::new(),
            profile_id: EmbeddingProfileId::new(),
            source_id: SourceId::new(),
            ordinal: 0,
            content: content.to_string(),
            content_hash: "hash".to_string(),
            metadata: serde_json::Value::Null,
            embedding,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_cosine_identical_is_one() {
        let score = cosine_similarity(&[1.0, 0.0, 0.5], &[1.0, 0.0, 0.5]).unwrap();
        assert!((score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_orthogonal_is_zero() {
        let score = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).unwrap();
        assert!(score.abs() < 1e-5);
    }

    #[test]
    fn test_cosine_mismatched_dimensions_rejected() {
        let err = cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.5]).unwrap_err();
        assert!(matches!(err, RetrievalError::SpaceMismatch(_)));
    }

    #[test]
    fn test_rank_by_vector_orders_best_first() {
        let chunks = vec![
            test_chunk("unrelated", vec![0.0, 1.0]),
            test_chunk("exact", vec![1.0, 0.0]),
        ];
        let ranked = rank_by_vector(&[1.0, 0.0], &chunks).unwrap();
        assert_eq!(ranked[0].0, 1);
        assert!((ranked[0].1 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_keyword_score_counts_query_coverage() {
        assert!((keyword_score("graph database", "a graph database engine") - 1.0).abs() < 1e-5);
        assert!((keyword_score("graph database", "a graph store") - 0.5).abs() < 1e-5);
        assert!((keyword_score("graph database", "nothing relevant here") - 0.0).abs() < 1e-5);
        assert_eq!(keyword_score("", "anything"), 0.0);
    }

    #[test]
    fn test_rrf_prefers_consensus_ranking() {
        let vector_ranking = vec![(0, 0.9), (1, 0.8), (2, 0.1)];
        let keyword_ranking = vec![(1, 1.0), (2, 0.5), (0, 0.0)];
        let fused = reciprocal_rank_fusion(&[vector_ranking, keyword_ranking], 60.0);
        assert_eq!(fused[0].0, 1);
    }
}
