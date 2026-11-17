pub mod chunker;
pub mod error;
pub mod evaluation;
pub mod prompt;
pub mod similarity;

pub use chunker::{chunk_record, chunk_text, hash_content, ChunkCandidate, ChunkConfig};
pub use error::RetrievalError;
pub use evaluation::{aggregate_metrics, citation_precision, EVAL_METRICS_VERSION};
pub use prompt::{
    build_evidence_block, build_qa_request, extract_citation_markers,
    INSUFFICIENT_EVIDENCE_MESSAGE, RAG_PROMPT_VERSION,
};
pub use similarity::{
    cosine_similarity, keyword_score, rank_by_keyword, rank_by_vector, reciprocal_rank_fusion,
};
