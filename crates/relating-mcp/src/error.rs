use thiserror::Error;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("Project not found or not in scope: {0}")]
    Scope(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Invalid tool arguments: {0}")]
    InvalidArguments(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Core error: {0}")]
    Core(#[from] relating_core::RelatingError),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Retrieval error: {0}")]
    Retrieval(#[from] relating_retrieval::RetrievalError),

    #[error("Migration error: {0}")]
    Migrate(#[from] relating_migrate::MigrateError),

    #[error("Internal error: {0}")]
    Internal(String),
}
