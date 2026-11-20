use thiserror::Error;

#[derive(Error, Debug)]
pub enum MigrateError {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Operation timed out: {0}")]
    Timeout(String),

    #[error("Migration plan error: {0}")]
    Plan(String),

    #[error("Verification failed: {0}")]
    Verification(String),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("Core error: {0}")]
    Core(#[from] relating_core::RelatingError),

    #[error("Internal error: {0}")]
    Internal(String),
}
