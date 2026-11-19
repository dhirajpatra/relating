//! PostgreSQL metadata support for team/server deployments.
//!
//! SQLite remains the default local store and is untouched. This module —
//! compiled only with the `postgres` cargo feature — provisions and validates
//! a shared PostgreSQL metadata database so small teams can point every
//! developer at one instance instead of passing SQLite files around.
//!
//! - `relating db check-postgres --url ...` validates reachability and reports
//!   which Relating tables exist (read-only, safe to run anywhere).
//! - `relating db init-postgres --url ...` creates the schema idempotently.

use relating_core::RelatingError;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;

pub const PG_SCHEMA: &str = include_str!("../migrations_pg/schema.sql");

pub fn expected_tables() -> Vec<String> {
    vec![
        "projects",
        "jobs",
        "sources",
        "ontologies",
        "embedding_profiles",
        "chunks",
        "evaluations",
        "aura_connections",
        "migrations",
        "schema_revisions",
        "retrieval_configs",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgCheckReport {
    pub reachable: bool,
    pub server_version: Option<String>,
    pub expected_tables: Vec<String>,
    pub found_tables: Vec<String>,
    pub missing_tables: Vec<String>,
    pub schema_ready: bool,
    pub message: String,
}

async fn connect(url: &str) -> Result<sqlx::PgPool, RelatingError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        PgPoolOptions::new().max_connections(2).connect(url),
    )
    .await
    .map_err(|_| RelatingError::Database("PostgreSQL connection timed out after 10s".to_string()))?
    .map_err(|e| RelatingError::Database(format!("Failed to connect to PostgreSQL: {}", e)))
}

/// Read-only validation: reachability, server version, and which Relating
/// tables exist. Never writes.
pub async fn check_postgres(database_url: &str) -> Result<PgCheckReport, RelatingError> {
    let pool = connect(database_url).await?;
    let server_version: Option<String> = sqlx::query("SELECT version()")
        .fetch_optional(&pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Version query failed: {}", e)))?
        .and_then(|row| row.try_get::<String, _>(0).ok());

    let rows = sqlx::query(
        "SELECT tablename FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema')",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Table listing failed: {}", e)))?;
    let present: std::collections::HashSet<String> = rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>(0).ok())
        .collect();

    let expected = expected_tables();
    let found: Vec<String> = expected
        .iter()
        .filter(|t| present.contains(*t))
        .cloned()
        .collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|t| !present.contains(*t))
        .cloned()
        .collect();
    let schema_ready = missing.is_empty();
    let message = if schema_ready {
        format!(
            "PostgreSQL is reachable and the Relating schema is ready ({} tables)",
            found.len()
        )
    } else {
        format!(
            "PostgreSQL is reachable but {} table(s) are missing: {}. Run `relating db init-postgres --url ...` to provision the schema.",
            missing.len(),
            missing.join(", ")
        )
    };
    Ok(PgCheckReport {
        reachable: true,
        server_version,
        expected_tables: expected,
        found_tables: found,
        missing_tables: missing,
        schema_ready,
        message,
    })
}

/// Idempotent provisioning: creates every missing Relating table/index.
/// Safe to re-run; existing tables are left untouched.
pub async fn ensure_pg_schema(database_url: &str) -> Result<PgCheckReport, RelatingError> {
    let pool = connect(database_url).await?;
    sqlx::raw_sql(PG_SCHEMA)
        .execute(&pool)
        .await
        .map_err(|e| RelatingError::Database(format!("PostgreSQL schema setup failed: {}", e)))?;
    check_postgres(database_url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pg_schema_covers_all_expected_tables() {
        for table in expected_tables() {
            assert!(
                PG_SCHEMA.contains(&format!("CREATE TABLE IF NOT EXISTS {}", table)),
                "schema.sql must create {}",
                table
            );
        }
        // SQLite-only column types must not leak into the Postgres DDL.
        assert!(
            !PG_SCHEMA.contains("BLOB NOT NULL") && !PG_SCHEMA.contains("REAL NOT NULL"),
            "use BYTEA / DOUBLE PRECISION column types, not SQLite BLOB / REAL"
        );
    }

    #[test]
    fn test_pg_schema_parses_as_postgres_dialect() {
        use sqlparser::dialect::PostgreSqlDialect;
        use sqlparser::parser::Parser;
        let statements = Parser::parse_sql(&PostgreSqlDialect {}, PG_SCHEMA)
            .expect("schema.sql must be valid PostgreSQL");
        // 11 tables + 14 indexes.
        assert_eq!(statements.len(), 25, "unexpected statement count");
    }
}
