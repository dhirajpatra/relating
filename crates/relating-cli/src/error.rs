use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Usage error: {0}")]
    Usage(String),

    #[error("Project error: {0}")]
    Project(String),

    #[error("Check failed: {0}")]
    Check(String),

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

    #[error("Schema error: {0}")]
    Schema(#[from] relating_schema::SchemaError),

    #[error("Internal error: {0}")]
    Internal(String),
}
