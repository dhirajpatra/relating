use thiserror::Error;

#[derive(Error, Debug)]
pub enum DebugError {
    #[error("Trace failed: {0}")]
    Trace(String),

    #[error("Core error: {0}")]
    Core(#[from] relating_core::RelatingError),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Retrieval error: {0}")]
    Retrieval(#[from] relating_retrieval::RetrievalError),

    #[error("Internal error: {0}")]
    Internal(String),
}
