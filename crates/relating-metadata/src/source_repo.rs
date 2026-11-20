use chrono::{DateTime, Utc};
use relating_core::{ProjectId, RelatingError, Source, SourceId, SourceStatus};
use sqlx::{Row, SqlitePool};

pub async fn insert_source(pool: &SqlitePool, source: &Source) -> Result<(), RelatingError> {
    let status_str = match source.status {
        SourceStatus::Pending => "pending",
        SourceStatus::Processed => "processed",
        SourceStatus::Failed => "failed",
    };

    let metadata_json = serde_json::to_string(&source.metadata)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize source metadata: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO sources (id, project_id, filename, content_hash, content_type, byte_size, status, metadata, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(source.id.to_string())
    .bind(source.project_id.to_string())
    .bind(&source.filename)
    .bind(&source.content_hash)
    .bind(&source.content_type)
    .bind(source.byte_size as i64)
    .bind(status_str)
    .bind(metadata_json)
    .bind(source.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert source: {}", e)))?;

    Ok(())
}

pub async fn get_source(pool: &SqlitePool, id: &SourceId) -> Result<Option<Source>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, filename, content_hash, content_type, byte_size, status, metadata, created_at
        FROM sources
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch source: {}", e)))?;

    match row {
        Some(row) => {
            let id_str: String = row.get("id");
            let project_id_str: String = row.get("project_id");
            let filename: String = row.get("filename");
            let content_hash: String = row.get("content_hash");
            let content_type: String = row.get("content_type");
            let byte_size: i64 = row.get("byte_size");
            let status_str: String = row.get("status");
            let metadata_str: String = row.get("metadata");
            let created_at_str: String = row.get("created_at");

            let id = SourceId::parse(&id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid Source UUID: {}", e)))?;
            let project_id = ProjectId::parse(&project_id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc);

            let status = match status_str.as_str() {
                "processed" => SourceStatus::Processed,
                "failed" => SourceStatus::Failed,
                _ => SourceStatus::Pending,
            };

            let metadata: serde_json::Value = serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null);

            Ok(Some(Source {
                id,
                project_id,
                filename,
                content_hash,
                content_type,
                byte_size: byte_size as u64,
                status,
                metadata,
                created_at,
            }))
        }
        None => Ok(None),
    }
}

pub async fn list_sources(pool: &SqlitePool, project_id: &ProjectId) -> Result<Vec<Source>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, filename, content_hash, content_type, byte_size, status, metadata, created_at
        FROM sources
        WHERE project_id = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list sources: {}", e)))?;

    let mut sources = Vec::with_capacity(rows.len());
    for row in rows {
        let id_str: String = row.get("id");
        let project_id_str: String = row.get("project_id");
        let filename: String = row.get("filename");
        let content_hash: String = row.get("content_hash");
        let content_type: String = row.get("content_type");
        let byte_size: i64 = row.get("byte_size");
        let status_str: String = row.get("status");
        let metadata_str: String = row.get("metadata");
        let created_at_str: String = row.get("created_at");

        let id = SourceId::parse(&id_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid Source UUID: {}", e)))?;
        let project_id = ProjectId::parse(&project_id_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&Utc);

        let status = match status_str.as_str() {
            "processed" => SourceStatus::Processed,
            "failed" => SourceStatus::Failed,
            _ => SourceStatus::Pending,
        };

        let metadata: serde_json::Value = serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null);

        sources.push(Source {
            id,
            project_id,
            filename,
            content_hash,
            content_type,
            byte_size: byte_size as u64,
            status,
            metadata,
            created_at,
        });
    }

    Ok(sources)
}

pub async fn delete_source(pool: &SqlitePool, id: &SourceId) -> Result<bool, RelatingError> {
    let result = sqlx::query("DELETE FROM sources WHERE id = ?")
        .bind(id.to_string())
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to delete source: {}", e)))?;

    Ok(result.rows_affected() > 0)
}
