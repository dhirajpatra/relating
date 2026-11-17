use thiserror::Error;

#[derive(Error, Debug)]
pub enum PortError {
    #[error("Scan failed: {0}")]
    Scan(String),

    #[error("Analysis failed: {0}")]
    Analysis(String),

    #[error("Invalid request: {0}")]
    Invalid(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
