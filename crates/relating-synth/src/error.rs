use thiserror::Error;

#[derive(Error, Debug)]
pub enum SynthError {
    #[error("Profiling failed: {0}")]
    Profile(String),

    #[error("Generation failed: {0}")]
    Generation(String),

    #[error("Invalid request: {0}")]
    Invalid(String),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("Internal error: {0}")]
    Internal(String),
}
