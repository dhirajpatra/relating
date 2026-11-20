use relating_ai::{ChatMessage, ChatRequest};
use relating_core::RetrievedPassage;

/// Versioned prompt for grounded question answering. Every answer persists
/// this version so evaluations stay comparable across prompt changes.
pub const RAG_PROMPT_VERSION: &str = "rag-qa-v1";

pub const INSUFFICIENT_EVIDENCE_MESSAGE: &str =
    "I could not find supporting evidence in this project's indexed sources, so I cannot answer this question.";

pub fn build_evidence_block(passages: &[RetrievedPassage]) -> String {
    let mut block = String::new();
    for (i, passage) in passages.iter().enumerate() {
        block.push_str(&format!(
            "[{}] (source: {}, score: {:.3})\n{}\n\n",
            i + 1,
            passage.filename,
            passage.score,
            passage.content.trim()
        ));
    }
    block
}

pub fn build_qa_request(question: &str, passages: &[RetrievedPassage]) -> ChatRequest {
    let evidence = build_evidence_block(passages);
    ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You answer questions using ONLY the evidence passages below. Cite every factual claim with the passage number in square brackets, e.g. [1]. If the evidence does not support an answer, say so explicitly and do not invent facts.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Evidence passages:\n\n{}\nQuestion: {}\n\nAnswer with citations like [1], [2].",
                    evidence, question
                ),
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(800),
    }
}

pub fn excerpt(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(max_chars).collect();
    format!("{}...", head)
}

/// Resolve 1-based markers against retrieved passages. Returns the valid
/// citations plus the count of markers that point nowhere (hallucinated).
pub fn build_citations(
    passages: &[RetrievedPassage],
    markers: &[u32],
) -> (Vec<relating_core::AnswerCitation>, usize) {
    let mut citations = Vec::new();
    let mut invalid = 0usize;
    for marker in markers {
        match passages.get((*marker as usize).wrapping_sub(1)) {
            Some(passage) if *marker >= 1 => citations.push(relating_core::AnswerCitation {
                index: *marker,
                chunk_id: passage.chunk_id.clone(),
                source_id: passage.source_id.clone(),
                filename: passage.filename.clone(),
                excerpt: excerpt(&passage.content, 200),
            }),
            _ => invalid += 1,
        }
    }
    (citations, invalid)
}

/// Extract 1-based citation markers like [1], [2] from answer text.
pub fn extract_citation_markers(answer: &str) -> Vec<u32> {
    let mut markers = Vec::new();
    let mut chars = answer.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '[' {
            continue;
        }
        let mut digits = String::new();
        for next in chars.by_ref() {
            if next == ']' {
                break;
            }
            if next.is_ascii_digit() {
                digits.push(next);
            } else {
                digits.clear();
                break;
            }
        }
        if !digits.is_empty() {
            if let Ok(n) = digits.parse::<u32>() {
                if n >= 1 && !markers.contains(&n) {
                    markers.push(n);
                }
            }
        }
    }
    markers.sort_unstable();
    markers
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_core::{ChunkId, SourceId};

    fn test_passage(filename: &str, content: &str) -> RetrievedPassage {
        RetrievedPassage {
            chunk_id: ChunkId::new(),
            source_id: SourceId::new(),
            filename: filename.to_string(),
            score: 0.9,
            rank: 1,
            content: content.to_string(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_evidence_block_numbers_passages() {
        let passages = vec![
            test_passage("a.csv", "Ada is an engineer."),
            test_passage("b.csv", "Bob manages Ada."),
        ];
        let block = build_evidence_block(&passages);
        assert!(block.contains("[1] (source: a.csv"));
        assert!(block.contains("[2] (source: b.csv"));
        assert!(block.contains("Ada is an engineer."));
    }

    #[test]
    fn test_qa_request_carries_evidence_and_question() {
        let passages = vec![test_passage("a.csv", "Ada is an engineer.")];
        let req = build_qa_request("Who is Ada?", &passages);
        assert_eq!(req.messages.len(), 2);
        assert!(req.messages[1].content.contains("Who is Ada?"));
        assert!(req.messages[1].content.contains("Ada is an engineer."));
    }

    #[test]
    fn test_extract_citation_markers() {
        assert_eq!(extract_citation_markers("Ada is an engineer [1]."), vec![1]);
        assert_eq!(
            extract_citation_markers("As shown in [2] and [1], see [2] again."),
            vec![1, 2]
        );
        assert!(extract_citation_markers("No citations here.").is_empty());
        assert!(extract_citation_markers("Not a marker [x] or [].").is_empty());
    }
}
