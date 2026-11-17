use chrono::{DateTime, Utc};
use relating_core::{ProjectId, RelatingError, SchemaRevision, SchemaRevisionId};
use sqlx::{Row, SqlitePool};

pub async fn insert_schema_revision(
    pool: &SqlitePool,
    revision: &SchemaRevision,
) -> Result<(), RelatingError> {
    let document_json = serde_json::to_string(&revision.document)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize document: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO schema_revisions (id, project_id, version, content_hash, document_json, created_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(revision.id.to_string())
    .bind(revision.project_id.to_string())
    .bind(revision.version as i64)
    .bind(&revision.content_hash)
    .bind(document_json)
    .bind(revision.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert schema revision: {}", e)))?;

    Ok(())
}

fn map_revision_row(row: sqlx::sqlite::SqliteRow) -> Result<SchemaRevision, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let version: i64 = row.get("version");
    let content_hash: String = row.get("content_hash");
    let document_json: String = row.get("document_json");
    let created_at_str: String = row.get("created_at");

    let id = SchemaRevisionId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid SchemaRevision UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);
    let document: serde_json::Value =
        serde_json::from_str(&document_json).unwrap_or(serde_json::Value::Null);

    Ok(SchemaRevision {
        id,
        project_id,
        version: version as u32,
        content_hash,
        document,
        created_at,
    })
}

pub async fn get_latest_schema_revision(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Option<SchemaRevision>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, version, content_hash, document_json, created_at
        FROM schema_revisions
        WHERE project_id = ?
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(project_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch schema revision: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_revision_row(row)?)),
        None => Ok(None),
    }
}

pub async fn get_schema_revision_by_version(
    pool: &SqlitePool,
    project_id: &ProjectId,
    version: u32,
) -> Result<Option<SchemaRevision>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, version, content_hash, document_json, created_at
        FROM schema_revisions
        WHERE project_id = ? AND version = ?
        "#,
    )
    .bind(project_id.to_string())
    .bind(version as i64)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch schema revision: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_revision_row(row)?)),
        None => Ok(None),
    }
}

pub async fn list_schema_revisions(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<SchemaRevision>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, version, content_hash, document_json, created_at
        FROM schema_revisions
        WHERE project_id = ?
        ORDER BY version ASC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list schema revisions: {}", e)))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(map_revision_row(row)?);
    }
    Ok(out)
}
