use relating_core::RelatingError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::str::FromStr;
use tracing::info;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db(database_url: &str) -> Result<DbPool, RelatingError> {
    // If it's a file URL or path, ensure parent directory exists
    if database_url.starts_with("sqlite://") && !database_url.contains(":memory:") {
        let file_path = database_url.trim_start_matches("sqlite://");
        let path = Path::new(file_path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    RelatingError::Database(format!(
                        "Failed to create directory for database {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
            }
        }
    }

    let connection_options = SqliteConnectOptions::from_str(database_url)
        .map_err(|e| {
            RelatingError::Database(format!("Invalid SQLite URL '{}': {}", database_url, e))
        })?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connection_options)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to connect to SQLite: {}", e)))?;

    run_migrations(&pool).await?;

    info!("Database initialized and migrations applied successfully.");
    Ok(pool)
}

pub const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial_schema.sql");
pub const SOURCES_AND_ONTOLOGIES_SCHEMA: &str =
    include_str!("../migrations/0002_sources_and_ontologies.sql");
pub const RETRIEVAL_SCHEMA: &str = include_str!("../migrations/0003_retrieval.sql");
pub const MIGRATION_SCHEMA: &str = include_str!("../migrations/0004_migration.sql");
pub const SCHEMA_AS_CODE_SCHEMA: &str = include_str!("../migrations/0005_schema.sql");

pub async fn run_migrations(pool: &DbPool) -> Result<(), RelatingError> {
    sqlx::raw_sql(INITIAL_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Migration 0001 failed: {}", e)))?;

    sqlx::raw_sql(SOURCES_AND_ONTOLOGIES_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Migration 0002 failed: {}", e)))?;

    sqlx::raw_sql(RETRIEVAL_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Migration 0003 failed: {}", e)))?;

    sqlx::raw_sql(MIGRATION_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Migration 0004 failed: {}", e)))?;

    sqlx::raw_sql(SCHEMA_AS_CODE_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Migration 0005 failed: {}", e)))?;

    Ok(())
}

pub async fn init_in_memory_db() -> Result<DbPool, RelatingError> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .map_err(|e| {
            RelatingError::Database(format!("Failed to connect to in-memory SQLite: {}", e))
        })?;

    run_migrations(&pool).await?;
    Ok(pool)
}
