use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("Store not found: {0}")]
    StoreNotFound(String),

    #[error("Read-only violation: {0}")]
    ReadOnlyViolation(String),

    #[error("Cypher syntax error: {0}")]
    Syntax(String),

    #[error("Cypher semantic error: {0}")]
    Semantic(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Store locked: {0}")]
    Locked(String),

    #[error("Parameter error: {0}")]
    Parameter(String),

    #[error("Internal graph error: {0}")]
    Internal(String),
}

impl From<relate::Error> for GraphError {
    fn from(err: relate::Error) -> Self {
        match err {
            relate::Error::Syntax(s) => GraphError::Syntax(s),
            relate::Error::Semantic(s) => GraphError::Semantic(s),
            relate::Error::Storage(s) => GraphError::Storage(s),
            relate::Error::Locked(s) => GraphError::Locked(s),
            relate::Error::ParameterMissing(s) => GraphError::Parameter(s),
            other => GraphError::Internal(other.to_string()),
        }
    }
}
