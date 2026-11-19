use crate::error::GraphError;
use serde::{Deserialize, Serialize};

/// Stable query target for application code.
///
/// Apps query Relating (local Relate store or an Aura connection) through one
/// shaped API. Switching backends is a connection-string flip, never a query
/// rewrite: the same Cypher text and bound parameters run on either target.
///
/// Connection strings:
/// - Local Relate store: `relate://local` (the project's own store; the
///   project itself is addressed by the surrounding API path or MCP scope).
/// - Aura connection: `aura://<connection-id>` where `<connection-id>` is the
///   id of an Aura connection registered on the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryTarget {
    /// The project's local Relate store.
    Local,
    /// A registered Aura connection, addressed by connection id.
    Aura { connection_id: String },
}

impl QueryTarget {
    pub fn connection_string(&self) -> String {
        match self {
            QueryTarget::Local => "relate://local".to_string(),
            QueryTarget::Aura { connection_id } => format!("aura://{}", connection_id),
        }
    }

    pub fn parse(connection_string: &str) -> Result<Self, GraphError> {
        let trimmed = connection_string.trim();
        if trimmed.eq_ignore_ascii_case("relate://local") || trimmed.eq_ignore_ascii_case("local") {
            return Ok(QueryTarget::Local);
        }
        if let Some(rest) = trimmed
            .strip_prefix("aura://")
            .or_else(|| trimmed.strip_prefix("AURA://"))
        {
            let id = rest.trim();
            if id.is_empty() {
                return Err(GraphError::Storage(
                    "Invalid connection string: 'aura://' must be followed by a connection id"
                        .to_string(),
                ));
            }
            if id.contains('/') || id.contains(' ') {
                return Err(GraphError::Storage(format!(
                    "Invalid connection string '{}': connection id must not contain '/' or spaces",
                    connection_string
                )));
            }
            return Ok(QueryTarget::Aura {
                connection_id: id.to_string(),
            });
        }
        Err(GraphError::Storage(format!(
            "Unknown connection string '{}': expected 'relate://local' or 'aura://<connection-id>'",
            connection_string
        )))
    }

    pub fn is_local(&self) -> bool {
        matches!(self, QueryTarget::Local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_local_variants() {
        assert_eq!(
            QueryTarget::parse("relate://local").unwrap(),
            QueryTarget::Local
        );
        assert_eq!(QueryTarget::parse("local").unwrap(), QueryTarget::Local);
        assert_eq!(
            QueryTarget::parse("  RELATE://LOCAL ").unwrap(),
            QueryTarget::Local
        );
    }

    #[test]
    fn test_parse_aura_round_trip() {
        let target = QueryTarget::parse("aura://conn-123").unwrap();
        assert_eq!(
            target,
            QueryTarget::Aura {
                connection_id: "conn-123".to_string()
            }
        );
        assert_eq!(target.connection_string(), "aura://conn-123");
        assert_eq!(
            QueryTarget::parse(&target.connection_string()).unwrap(),
            target
        );
    }

    #[test]
    fn test_parse_rejects_garbage() {
        assert!(QueryTarget::parse("aura://").is_err());
        assert!(QueryTarget::parse("aura://a/b").is_err());
        assert!(QueryTarget::parse("neo4j://host").is_err());
        assert!(QueryTarget::parse("").is_err());
    }
}
