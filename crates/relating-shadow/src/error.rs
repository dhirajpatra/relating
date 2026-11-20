use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShadowError {
    #[error("Replay failed: {0}")]
    Replay(String),

    #[error("Invalid request: {0}")]
    Invalid(String),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("Migration error: {0}")]
    Migrate(#[from] relating_migrate::MigrateError),

    #[error("Internal error: {0}")]
    Internal(String),
}
