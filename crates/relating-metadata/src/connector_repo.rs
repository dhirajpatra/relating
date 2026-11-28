//! Data-connector registry: SQL and REST source configurations plus
//! per-connector sync state (watermarks, content hashes, last sync).

use chrono::{DateTime, Utc};
use relating_core::RelatingError;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct DataConnector {
    pub id: String,
    pub project_id: String,
    pub name: String,
    /// `sql` or `rest`.
    pub kind: String,
    /// Validated connector config. Secrets live behind `*_env` references
    /// resolved at sync time; raw credentials are rejected at creation.
    pub config: serde_json::Value,
    pub mapping: serde_json::Value,
    pub watermark: Option<String>,
    pub last_sync: Option<DateTime<Utc>>,
    pub sync_hash: Option<String>,
    pub poll_interval_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn parse_time(value: &str, column: &str) -> Result<DateTime<Utc>, RelatingError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RelatingError::Internal(format!("Invalid {} timestamp: {}", column, e)))
}

fn connector_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<DataConnector, RelatingError> {
    let created_at: String = row.get("created_at");
    let updated_at: String = row.get("updated_at");
    let last_sync: Option<String> = row.get("last_sync");
    let config_str: String = row.get("config_json");
    let mapping_str: String = row.get("mapping_json");
    Ok(DataConnector {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        kind: row.get("kind"),
        config: serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Null),
        mapping: serde_json::from_str(&mapping_str).unwrap_or(serde_json::json!({})),
        watermark: row.get("watermark"),
        last_sync: last_sync
            .map(|s| parse_time(&s, "data_connectors.last_sync"))
            .transpose()?,
        sync_hash: row.get("sync_hash"),
        poll_interval_secs: row.get("poll_interval_secs"),
        created_at: parse_time(&created_at, "data_connectors.created_at")?,
        updated_at: parse_time(&updated_at, "data_connectors.updated_at")?,
    })
}

const SELECT_COLUMNS: &str = "id, project_id, name, kind, config_json, mapping_json, watermark, last_sync, sync_hash, poll_interval_secs, created_at, updated_at";

pub async fn insert_connector(
    pool: &SqlitePool,
    project_id: &str,
    name: &str,
    kind: &str,
    config: &serde_json::Value,
    mapping: &serde_json::Value,
    poll_interval_secs: Option<i64>,
) -> Result<DataConnector, RelatingError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let config_str = serde_json::to_string(config)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize config: {}", e)))?;
    let mapping_str = serde_json::to_string(mapping)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize mapping: {}", e)))?;
    sqlx::query(
        "INSERT INTO data_connectors (id, project_id, name, kind, config_json, mapping_json, poll_interval_secs, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(name)
    .bind(kind)
    .bind(&config_str)
    .bind(&mapping_str)
    .bind(poll_interval_secs)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert connector: {}", e)))?;
    get_connector(pool, &id)
        .await?
        .ok_or_else(|| RelatingError::Internal("Connector vanished after insert".to_string()))
}

pub async fn get_connector(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<DataConnector>, RelatingError> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM data_connectors WHERE id = ?",
        SELECT_COLUMNS
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to look up connector: {}", e)))?;
    row.map(|r| connector_from_row(&r)).transpose()
}

pub async fn get_project_connector(
    pool: &SqlitePool,
    project_id: &str,
    id: &str,
) -> Result<Option<DataConnector>, RelatingError> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM data_connectors WHERE id = ? AND project_id = ?",
        SELECT_COLUMNS
    ))
    .bind(id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to look up connector: {}", e)))?;
    row.map(|r| connector_from_row(&r)).transpose()
}

pub async fn list_connectors(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<DataConnector>, RelatingError> {
    let rows = sqlx::query(&format!(
        "SELECT {} FROM data_connectors WHERE project_id = ? ORDER BY created_at ASC",
        SELECT_COLUMNS
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list connectors: {}", e)))?;
    rows.iter().map(connector_from_row).collect()
}

pub async fn delete_connector(pool: &SqlitePool, id: &str) -> Result<bool, RelatingError> {
    let done = sqlx::query("DELETE FROM data_connectors WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to delete connector: {}", e)))?;
    Ok(done.rows_affected() > 0)
}

pub async fn update_connector_sync_state(
    pool: &SqlitePool,
    id: &str,
    watermark: Option<&str>,
    sync_hash: &str,
    now: &DateTime<Utc>,
) -> Result<(), RelatingError> {
    sqlx::query(
        "UPDATE data_connectors SET watermark = ?, sync_hash = ?, last_sync = ?, updated_at = ? WHERE id = ?",
    )
    .bind(watermark)
    .bind(sync_hash)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to update connector state: {}", e)))?;
    Ok(())
}

/// Connectors whose `poll_interval_secs` elapsed (or never synced). Backs
/// cron-style schedulers: `GET .../connectors/due`, then `POST .../sync`
/// for each id.
pub async fn list_due_connectors(
    pool: &SqlitePool,
    project_id: &str,
    now: &DateTime<Utc>,
) -> Result<Vec<DataConnector>, RelatingError> {
    let all = list_connectors(pool, project_id).await?;
    Ok(all
        .into_iter()
        .filter(|c| match (c.poll_interval_secs, c.last_sync) {
            (Some(interval), Some(last)) => (*now - last).num_seconds() >= interval.max(60),
            (Some(_), None) => true,
            (None, _) => false,
        })
        .collect())
}
