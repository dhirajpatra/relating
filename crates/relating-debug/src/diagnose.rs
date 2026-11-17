use crate::trace::{query_term_overlap, DebugTrace};
use relating_ai::{AiProvider, ChatMessage, ChatRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisRule {
    pub code: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiHypothesis {
    pub text: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub rules: Vec<DiagnosisRule>,
    pub likely_causes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_hypothesis: Option<AiHypothesis>,
}

fn rule(code: &str, severity: Severity, message: impl Into<String>) -> DiagnosisRule {
    DiagnosisRule {
        code: code.to_string(),
        severity,
        message: message.into(),
    }
}

/// Deterministic checks over the trace. These always run, with or without AI.
pub fn diagnose_rules(trace: &DebugTrace) -> Vec<DiagnosisRule> {
    let mut rules = Vec::new();

    if trace.passages.is_empty() {
        rules.push(rule(
            "DBG-E001",
            Severity::Warning,
            "No evidence retrieved: the index may be empty, the profile mismatched, or the question out of scope",
        ));
        return rules;
    }

    // Vocabulary mismatch on the top passage.
    if let Some(top) = trace.passages.first() {
        if query_term_overlap(&trace.question, &top.content) == 0.0 {
            rules.push(rule(
                "DBG-W001",
                Severity::Warning,
                "Top passage shares no query terms (vector-only match): possible vocabulary mismatch; consider ontology synonyms or keyword weight",
            ));
        }
    }

    // Citation behavior.
    if trace.citations.is_empty() {
        rules.push(rule(
            "DBG-W002",
            Severity::Warning,
            "Model answered from evidence but cited nothing: prompt or model citation behavior",
        ));
    }
    if trace.invalid_markers > 0 {
        rules.push(rule(
            "DBG-W003",
            Severity::Warning,
            format!(
                "Model cited {} passage(s) that were never retrieved (hallucinated citations)",
                trace.invalid_markers
            ),
        ));
    }

    // Extraction coverage.
    let linked_chunks = trace
        .graph_links
        .iter()
        .filter(|l| !l.nodes.is_empty())
        .count();
    if linked_chunks == 0 {
        rules.push(rule(
            "DBG-W004",
            Severity::Warning,
            "No graph entities linked to retrieved chunks: extraction may have missed entities or lineage is broken",
        ));
    } else {
        let without_rels = trace
            .graph_links
            .iter()
            .filter(|l| !l.nodes.is_empty() && l.relationship_types.is_empty())
            .count();
        if without_rels > 0 {
            rules.push(rule(
                "DBG-W005",
                Severity::Warning,
                format!(
                    "{} chunk(s) link to entities with no relationships: extraction may have missed relations",
                    without_rels
                ),
            ));
        }
    }

    // Possible unresolved duplicates: same (label, name) on distinct uids.
    let mut seen: HashMap<(String, String), String> = HashMap::new();
    let mut duplicates = 0usize;
    for link in &trace.graph_links {
        for node in &link.nodes {
            if let Some(name) = &node.name {
                for label in &node.labels {
                    let key = (label.clone(), name.clone());
                    match seen.get(&key) {
                        Some(first_uid) if first_uid != &node.uid => duplicates += 1,
                        _ => {
                            seen.entry(key).or_insert_with(|| node.uid.clone());
                        }
                    }
                }
            }
        }
    }
    if duplicates > 0 {
        rules.push(rule(
            "DBG-W006",
            Severity::Warning,
            "Same label+name on distinct uids: possible unresolved duplicate entities",
        ));
    }

    // Ontology mapping.
    if !trace.ontology_approved {
        rules.push(rule(
            "DBG-W007",
            Severity::Warning,
            "No approved ontology: label mapping is heuristic; approve one for stable contracts",
        ));
    } else {
        let mapped = trace
            .ontology_mapping
            .iter()
            .filter(|m| m.mapped_entity.is_some())
            .count();
        rules.push(rule(
            "DBG-I001",
            Severity::Info,
            format!(
                "Ontology maps {}/{} linked labels",
                mapped,
                trace.ontology_mapping.len()
            ),
        ));
    }

    if rules.iter().all(|r| r.severity == Severity::Info) {
        rules.push(rule(
            "DBG-I002",
            Severity::Info,
            "No structural problems detected; remaining quality depends on model reasoning",
        ));
    }

    rules
}

pub fn likely_causes(rules: &[DiagnosisRule]) -> Vec<String> {
    let mut causes = Vec::new();
    let has = |code: &str| rules.iter().any(|r| r.code == code);
    if has("DBG-E001") {
        causes.push("missing evidence (empty index or out-of-scope query)".to_string());
    }
    if has("DBG-W001") {
        causes.push("retrieval configuration (vocabulary mismatch)".to_string());
    }
    if has("DBG-W002") || has("DBG-W003") {
        causes.push("model citation behavior".to_string());
    }
    if has("DBG-W004") || has("DBG-W005") {
        causes.push("extraction coverage (entities or relationships)".to_string());
    }
    if has("DBG-W006") {
        causes.push("entity resolution (duplicate entities)".to_string());
    }
    if has("DBG-W007") {
        causes.push("ontology mapping (no approved ontology)".to_string());
    }
    if causes.is_empty() {
        causes.push("model reasoning over the provided context".to_string());
    }
    causes
}

fn trace_summary(trace: &DebugTrace) -> String {
    let mut out = format!(
        "Question: {}\nStrategy: {:?}, profile: {} ({} {}d), passages: {}, grounded: {}\n",
        trace.question,
        trace.strategy,
        trace.profile_name,
        trace.embedding_model,
        trace.dimensions,
        trace.passages.len(),
        trace.grounded
    );
    for row in trace.ranking.iter().take(5) {
        out.push_str(&format!(
            "Passage #{} {} (fused {:.3}, vector {:?}, keyword {:?}): {}\n",
            row.rank,
            row.filename,
            row.fused_score,
            row.vector_rank,
            row.keyword_rank,
            row.content_snippet
        ));
    }
    out.push_str(&format!(
        "Answer: {}\n",
        trace.answer.chars().take(500).collect::<String>()
    ));
    out
}

/// Optional AI second opinion. Returns None (with an info rule left to the
/// caller) when the provider call fails, so diagnosis degrades honestly.
pub async fn diagnose_ai(
    provider: &Arc<dyn AiProvider>,
    trace: &DebugTrace,
    rules: &[DiagnosisRule],
    chat_model: &str,
) -> Option<AiHypothesis> {
    let rules_text = rules
        .iter()
        .map(|r| format!("[{}] {}", r.code, r.message))
        .collect::<Vec<_>>()
        .join("\n");
    let req = ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You diagnose GraphRAG answer failures. Given the retrieval trace and rule findings, state the single most likely cause in two sentences, then one concrete fix. Do not invent evidence.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Trace:\n{}\nRule findings:\n{}",
                    trace_summary(trace),
                    rules_text
                ),
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(400),
    };
    provider
        .chat_completion(&req)
        .await
        .ok()
        .map(|resp| AiHypothesis {
            text: resp.content,
            model: chat_model.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{DebugTrace, GraphLink, RankingRow};
    use relating_core::{RetrievalStrategy, RetrievedPassage};

    fn empty_trace() -> DebugTrace {
        DebugTrace {
            question: "Who?".to_string(),
            strategy: RetrievalStrategy::Hybrid,
            profile_name: "none".to_string(),
            embedding_model: "m".to_string(),
            dimensions: 0,
            passages: Vec::new(),
            ranking: Vec::new(),
            graph_links: Vec::new(),
            ontology_mapping: Vec::new(),
            ontology_approved: false,
            evidence_chars: 0,
            evidence_preview: String::new(),
            prompt_version: "rag-qa-v1".to_string(),
            answer: String::new(),
            grounded: false,
            citations: Vec::new(),
            invalid_markers: 0,
            chat_model: "c".to_string(),
            latency_ms: 0.0,
        }
    }

    fn passage(content: &str) -> RetrievedPassage {
        RetrievedPassage {
            chunk_id: relating_core::ChunkId::new(),
            source_id: relating_core::SourceId::new(),
            filename: "s.csv".to_string(),
            score: 0.9,
            rank: 1,
            content: content.to_string(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_empty_trace_yields_e001() {
        let rules = diagnose_rules(&empty_trace());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].code, "DBG-E001");
        assert_eq!(
            likely_causes(&rules),
            vec!["missing evidence (empty index or out-of-scope query)".to_string()]
        );
    }

    #[test]
    fn test_vocabulary_and_citation_rules() {
        let mut trace = empty_trace();
        trace.passages = vec![passage("completely unrelated content here")];
        trace.ranking = vec![RankingRow {
            rank: 1,
            chunk_id: "c".to_string(),
            filename: "s.csv".to_string(),
            fused_score: 0.9,
            vector_rank: Some(1),
            keyword_rank: Some(9),
            content_snippet: "unrelated".to_string(),
        }];
        trace.question = "Who is Ada?".to_string();
        trace.grounded = true;
        let rules = diagnose_rules(&trace);
        let codes: Vec<&str> = rules.iter().map(|r| r.code.as_str()).collect();
        assert!(codes.contains(&"DBG-W001"));
        assert!(codes.contains(&"DBG-W002"));
        assert!(codes.contains(&"DBG-W004"));
    }

    #[test]
    fn test_hallucinated_citations_flagged() {
        let mut trace = empty_trace();
        trace.passages = vec![passage("Who is Ada Ada Ada")];
        trace.invalid_markers = 2;
        trace.grounded = true;
        let rules = diagnose_rules(&trace);
        assert!(rules.iter().any(|r| r.code == "DBG-W003"));
    }

    #[test]
    fn test_clean_trace_is_info_only() {
        let mut trace = empty_trace();
        trace.passages = vec![passage("Who is Ada Ada")];
        trace.graph_links = vec![GraphLink {
            chunk_id: "c".to_string(),
            filename: "s.csv".to_string(),
            lineage_ref: Some("row_1".to_string()),
            nodes: vec![crate::trace::LinkedNode {
                uid: "Person:1".to_string(),
                labels: vec!["Person".to_string()],
                name: Some("Ada".to_string()),
            }],
            relationship_types: vec!["KNOWS".to_string()],
            note: None,
        }];
        trace.ontology_approved = true;
        trace.ontology_mapping = vec![crate::trace::OntologyMapping {
            label: "Person".to_string(),
            mapped_entity: Some("Person".to_string()),
            ontology_relationships: vec!["KNOWS:Person->Person".to_string()],
        }];
        trace.grounded = true;
        trace.citations = vec![relating_core::AnswerCitation {
            index: 1,
            chunk_id: relating_core::ChunkId::new(),
            source_id: relating_core::SourceId::new(),
            filename: "s.csv".to_string(),
            excerpt: "Ada".to_string(),
        }];
        let rules = diagnose_rules(&trace);
        assert!(rules.iter().all(|r| r.severity == Severity::Info));
        assert_eq!(
            likely_causes(&rules),
            vec!["model reasoning over the provided context".to_string()]
        );
    }
}
