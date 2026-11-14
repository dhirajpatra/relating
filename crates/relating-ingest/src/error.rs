use thiserror::Error;

#[derive(Error, Debug)]
pub enum IngestError {
    #[error("Parse error in {filename}: {message}")]
    ParseError { filename: String, message: String },

    #[error("Unsupported content format: {0}")]
    UnsupportedFormat(String),

    #[error("Ontology error: {0}")]
    Ontology(String),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("AI error: {0}")]
    Ai(#[from] relating_ai::AiError),

    #[error("Graph error: {0}")]
    Graph(#[from] relating_graph::GraphError),

    #[error("Internal error: {0}")]
    Internal(String),
}
