pub mod chunker;
pub mod error;
pub mod evaluation;
pub mod prompt;
pub mod service;
pub mod similarity;

pub use chunker::{chunk_record, chunk_text, hash_content, ChunkCandidate, ChunkConfig};
pub use error::RetrievalError;
pub use evaluation::{aggregate_metrics, citation_precision, EVAL_METRICS_VERSION};
pub use prompt::{
    build_citations, build_evidence_block, build_qa_request, excerpt, extract_citation_markers,
    INSUFFICIENT_EVIDENCE_MESSAGE, RAG_PROMPT_VERSION,
};
pub use service::{run_retrieval, DEFAULT_PROFILE_NAME, DEFAULT_TOP_K, MAX_TOP_K};
pub use similarity::{
    cosine_similarity, keyword_score, rank_by_keyword, rank_by_vector, reciprocal_rank_fusion,
};
