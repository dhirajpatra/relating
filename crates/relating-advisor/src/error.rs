use thiserror::Error;

#[derive(Error, Debug)]
pub enum AdvisorError {
    #[error("Analysis failed: {0}")]
    Analysis(String),

    #[error("Invalid request: {0}")]
    Invalid(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
