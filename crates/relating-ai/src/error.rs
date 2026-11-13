use thiserror::Error;

#[derive(Error, Debug)]
pub enum AiError {
    #[error("Missing API key or credentials: {0}")]
    MissingCredentials(String),

    #[error("AI provider connection failed: {0}")]
    ConnectionFailed(String),

    #[error("AI provider returned error: status {status}, message: {message}")]
    ProviderError { status: u16, message: String },

    #[error("Serialization / parsing error: {0}")]
    Serialization(String),

    #[error("Incompatible embedding space: {0}")]
    IncompatibleEmbeddingSpace(String),

    #[error("Internal AI error: {0}")]
    Internal(String),
}
