use thiserror::Error;

#[derive(Error, Debug)]
pub enum SchemaError {
    #[error("Invalid schema document: {0}")]
    Invalid(String),

    #[error("Unsupported schema format '{0}': expected '{1}'")]
    UnsupportedFormat(String, String),

    #[error("Conformance check failed: {0}")]
    Conformance(String),

    #[error("Core error: {0}")]
    Core(#[from] relating_core::RelatingError),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("Internal error: {0}")]
    Internal(String),
}
