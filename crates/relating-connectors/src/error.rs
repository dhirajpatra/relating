use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("Invalid connector config: {0}")]
    Config(String),
    #[error("SQL source error: {0}")]
    Sql(String),
    #[error("REST source error: {0}")]
    Rest(String),
    #[error("Mapping error: {0}")]
    Mapping(String),
}
