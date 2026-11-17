use crate::error::DebugError;
use relating_ai::AiProvider;
use relating_core::{AnswerCitation, Project, RetrievalStrategy, RetrievedPassage};
use relating_graph::GraphBackend;
use relating_metadata::{
    get_approved_ontology, get_latest_ontology, list_chunks_for_profile, DbPool,
};
use relating_retrieval::{
    build_citations, build_evidence_block, build_qa_request, excerpt, extract_citation_markers,
    rank_by_keyword, rank_by_vector, run_retrieval, RetrievalError, RAG_PROMPT_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingRow {
    pub rank: u32,
    pub chunk_id: String,
    pub filename: String,
    pub fused_score: f32,
    pub vector_rank: Option<u32>,
    pub keyword_rank: Option<u32>,
    pub content_snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedNode {
    pub uid: String,
    pub labels: Vec<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLink {
    pub chunk_id: String,
    pub filename: String,
    pub lineage_ref: Option<String>,
    pub nodes: Vec<LinkedNode>,
    pub relationship_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyMapping {
    pub label: String,
    pub mapped_entity: Option<String>,
    pub ontology_relationships: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugTrace {
    pub question: String,
    pub strategy: RetrievalStrategy,
    pub profile_name: String,
    pub embedding_model: String,
    pub dimensions: usize,
    pub passages: Vec<RetrievedPassage>,
    pub ranking: Vec<RankingRow>,
    pub graph_links: Vec<GraphLink>,
    pub ontology_mapping: Vec<OntologyMapping>,
    pub ontology_approved: bool,
    pub evidence_chars: usize,
    pub evidence_preview: String,
    pub prompt_version: String,
    pub answer: String,
    pub grounded: bool,
    pub citations: Vec<AnswerCitation>,
    pub invalid_markers: usize,
    pub chat_model: String,
    pub latency_ms: f64,
}

fn token_overlap(a: &str, b: &str) -> f32 {
    let tokenize = |text: &str| -> HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 1)
            .map(|t| t.to_lowercase())
            .collect()
    };
    let query_tokens = tokenize(a);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let content_tokens = tokenize(b);
    query_tokens.intersection(&content_tokens).count() as f32 / query_tokens.len() as f32
}

pub fn query_term_overlap(question: &str, content: &str) -> f32 {
    token_overlap(question, content)
}

#[allow(clippy::too_many_arguments)]
pub async fn build_trace(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    store_path: &Path,
    project: &Project,
    provider: &Arc<dyn AiProvider>,
    question: &str,
    strategy: RetrievalStrategy,
    top_k: usize,
) -> Result<DebugTrace, DebugError> {
    let start = std::time::Instant::now();

    // Stage 1: retrieval candidates (shared service: same behavior as Q&A).
    let trace = match run_retrieval(
        pool,
        provider,
        project,
        question,
        strategy.clone(),
        top_k,
        None,
    )
    .await
    {
        Ok(trace) => trace,
        Err(RetrievalError::NoEvidence(_)) => {
            return Ok(DebugTrace {
                question: question.to_string(),
                strategy,
                profile_name: "none".to_string(),
                embedding_model: project.ai_config.embedding_model.clone(),
                dimensions: 0,
                passages: Vec::new(),
                ranking: Vec::new(),
                graph_links: Vec::new(),
                ontology_mapping: Vec::new(),
                ontology_approved: get_approved_ontology(pool, &project.id).await?.is_some(),
                evidence_chars: 0,
                evidence_preview: String::new(),
                prompt_version: RAG_PROMPT_VERSION.to_string(),
                answer: String::new(),
                grounded: false,
                citations: Vec::new(),
                invalid_markers: 0,
                chat_model: project.ai_config.chat_model.clone(),
                latency_ms: start.elapsed().as_secs_f64() * 1000.0,
            });
        }
        Err(other) => return Err(DebugError::Retrieval(other)),
    };

    // Stage 2: ranking decisions — per-strategy ranks behind the fused order.
    let mut ranking = Vec::new();
    {
        let chunks = list_chunks_for_profile(pool, &trace.profile_id).await?;
        let index_of: HashMap<String, usize> = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.to_string(), i))
            .collect();
        let query_embedding = provider
            .embed(&relating_ai::EmbeddingRequest {
                inputs: vec![question.to_string()],
            })
            .await
            .ok()
            .and_then(|r| r.embeddings.into_iter().next());
        let vector_ranks: HashMap<usize, u32> = match &query_embedding {
            Some(embedding) => rank_by_vector(embedding, &chunks)
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(pos, (idx, _))| (idx, pos as u32 + 1))
                .collect(),
            None => HashMap::new(),
        };
        let keyword_ranks: HashMap<usize, u32> = rank_by_keyword(question, &chunks)
            .into_iter()
            .enumerate()
            .map(|(pos, (idx, _))| (idx, pos as u32 + 1))
            .collect();
        for passage in &trace.passages {
            let idx = index_of.get(&passage.chunk_id.to_string()).copied();
            ranking.push(RankingRow {
                rank: passage.rank,
                chunk_id: passage.chunk_id.to_string(),
                filename: passage.filename.clone(),
                fused_score: passage.score,
                vector_rank: idx.and_then(|i| vector_ranks.get(&i).copied()),
                keyword_rank: idx.and_then(|i| keyword_ranks.get(&i).copied()),
                content_snippet: excerpt(&passage.content, 200),
            });
        }
    }

    // Stage 3: graph links — entities carrying the chunk's provenance.
    let mut graph_links = Vec::new();
    for passage in &trace.passages {
        let lineage_ref = passage
            .metadata
            .get("lineage_ref")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut link = GraphLink {
            chunk_id: passage.chunk_id.to_string(),
            filename: passage.filename.clone(),
            lineage_ref: lineage_ref.clone(),
            nodes: Vec::new(),
            relationship_types: Vec::new(),
            note: None,
        };
        match &lineage_ref {
            Some(lineage) => {
                let mut params = HashMap::new();
                params.insert("ref".to_string(), serde_json::json!(lineage));
                params.insert("file".to_string(), serde_json::json!(passage.filename));
                match graph.execute_read(
                    store_path,
                    "MATCH (n) WHERE n._lineage_ref = $ref AND n._source_file = $file RETURN n.uid AS uid, labels(n) AS labels, n.name AS name LIMIT 5",
                    params,
                    None,
                ) {
                    Ok(page) => {
                        for row in &page.rows {
                            let uid = row.get("uid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let labels = row
                                .get("labels")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
                                })
                                .unwrap_or_default();
                            let name = row.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
                            if !uid.is_empty() {
                                // Sample relationship types around this node.
                                let mut rel_params = HashMap::new();
                                rel_params.insert("uid".to_string(), serde_json::json!(uid));
                                if let Ok(rel_page) = graph.execute_read(
                                    store_path,
                                    "MATCH (n {uid: $uid})-[r]-(m) RETURN type(r) AS t LIMIT 25",
                                    rel_params,
                                    None,
                                ) {
                                    for rel_row in &rel_page.rows {
                                        if let Some(t) = rel_row.get("t").and_then(|v| v.as_str()) {
                                            if !link.relationship_types.contains(&t.to_string()) {
                                                link.relationship_types.push(t.to_string());
                                            }
                                        }
                                    }
                                }
                                link.nodes.push(LinkedNode { uid, labels, name });
                            }
                        }
                        if link.nodes.is_empty() {
                            link.note = Some("No graph entities carry this chunk's lineage".to_string());
                        }
                    }
                    Err(e) => {
                        link.note = Some(format!("Entity lookup failed: {}", e));
                    }
                }
            }
            None => {
                link.note =
                    Some("Prose chunk without record lineage; no direct entity link".to_string());
            }
        }
        graph_links.push(link);
    }

    // Stage 4: ontology mapping for every linked label.
    let ontology = get_approved_ontology(pool, &project.id)
        .await?
        .or(get_latest_ontology(pool, &project.id).await?);
    let ontology_approved = get_approved_ontology(pool, &project.id).await?.is_some();
    let mut labels_seen = HashSet::new();
    let mut ontology_mapping = Vec::new();
    for link in &graph_links {
        for node in &link.nodes {
            for label in &node.labels {
                if !labels_seen.insert(label.clone()) {
                    continue;
                }
                let mapped = ontology
                    .as_ref()
                    .and_then(|o| o.schema.entities.iter().find(|e| &e.label == label));
                let rels = match &ontology {
                    Some(o) => o
                        .schema
                        .relationships
                        .iter()
                        .filter(|r| &r.source_label == label || &r.target_label == label)
                        .map(|r| format!("{}:{}->{}", r.rel_type, r.source_label, r.target_label))
                        .collect(),
                    None => Vec::new(),
                };
                ontology_mapping.push(OntologyMapping {
                    label: label.clone(),
                    mapped_entity: mapped.map(|e| e.display_name.clone()),
                    ontology_relationships: rels,
                });
            }
        }
    }

    // Stage 5+6: final context and answer (same prompt as Q&A).
    let evidence_block = build_evidence_block(&trace.passages);
    let chat_req = build_qa_request(question, &trace.passages);
    let chat_resp = provider.chat_completion(&chat_req).await?;
    let markers = extract_citation_markers(&chat_resp.content);
    let (citations, invalid_markers) = build_citations(&trace.passages, &markers);

    Ok(DebugTrace {
        question: question.to_string(),
        strategy: trace.strategy,
        profile_name: trace.profile_name,
        embedding_model: trace.embedding_model,
        dimensions: trace.dimensions,
        passages: trace.passages,
        ranking,
        graph_links,
        ontology_mapping,
        ontology_approved,
        evidence_chars: evidence_block.len(),
        evidence_preview: excerpt(&evidence_block, 2000),
        prompt_version: RAG_PROMPT_VERSION.to_string(),
        answer: chat_resp.content,
        grounded: true,
        citations,
        invalid_markers,
        chat_model: project.ai_config.chat_model.clone(),
        latency_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_overlap() {
        assert!(
            (query_term_overlap("graph database", "a graph database engine") - 1.0).abs() < 1e-5
        );
        assert_eq!(query_term_overlap("graph database", "nothing here"), 0.0);
        assert_eq!(query_term_overlap("", "anything"), 0.0);
    }
}
