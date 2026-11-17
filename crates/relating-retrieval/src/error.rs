use thiserror::Error;

#[derive(Error, Debug)]
pub enum RetrievalError {
    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Embedding space mismatch: {0}")]
    SpaceMismatch(String),

    #[error("No evidence available: {0}")]
    NoEvidence(String),

    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Core error: {0}")]
    Core(#[from] relating_core::RelatingError),

    #[error("Internal error: {0}")]
    Internal(String),
}
