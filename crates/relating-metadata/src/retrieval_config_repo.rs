use chrono::{DateTime, Utc};
use relating_core::{ProjectId, RelatingError, RetrievalConfig, RetrievalStrategy};
use sqlx::{Row, SqlitePool};

fn strategy_to_str(strategy: &RetrievalStrategy) -> &'static str {
    match strategy {
        RetrievalStrategy::Vector => "vector",
        RetrievalStrategy::Keyword => "keyword",
        RetrievalStrategy::Hybrid => "hybrid",
    }
}

fn strategy_from_str(s: &str) -> RetrievalStrategy {
    match s {
        "vector" => RetrievalStrategy::Vector,
        "keyword" => RetrievalStrategy::Keyword,
        _ => RetrievalStrategy::Hybrid,
    }
}

pub async fn upsert_retrieval_config(
    pool: &SqlitePool,
    config: &RetrievalConfig,
) -> Result<(), RelatingError> {
    sqlx::query(
        r#"
        INSERT INTO retrieval_configs (project_id, default_strategy, default_top_k, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(project_id) DO UPDATE SET
            default_strategy = excluded.default_strategy,
            default_top_k = excluded.default_top_k,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(config.project_id.to_string())
    .bind(strategy_to_str(&config.default_strategy))
    .bind(config.default_top_k as i64)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to save retrieval config: {}", e)))?;

    Ok(())
}

pub async fn get_retrieval_config(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Option<RetrievalConfig>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT project_id, default_strategy, default_top_k, updated_at
        FROM retrieval_configs
        WHERE project_id = ?
        "#,
    )
    .bind(project_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch retrieval config: {}", e)))?;

    match row {
        Some(row) => {
            let project_id_str: String = row.get("project_id");
            let strategy_str: String = row.get("default_strategy");
            let top_k: i64 = row.get("default_top_k");
            let updated_at_str: String = row.get("updated_at");

            let project_id = ProjectId::parse(&project_id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
            let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc);

            Ok(Some(RetrievalConfig {
                project_id,
                default_strategy: strategy_from_str(&strategy_str),
                default_top_k: top_k as usize,
                updated_at,
            }))
        }
        None => Ok(None),
    }
}
