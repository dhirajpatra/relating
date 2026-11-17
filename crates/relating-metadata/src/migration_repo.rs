use chrono::{DateTime, Utc};
use relating_core::{
    AuraConnectionId, Migration, MigrationId, MigrationProgress, MigrationStatus, ProjectId,
    RelatingError,
};
use sqlx::{Row, SqlitePool};

fn status_to_str(status: &MigrationStatus) -> &'static str {
    match status {
        MigrationStatus::Draft => "draft",
        MigrationStatus::Analyzed => "analyzed",
        MigrationStatus::Planned => "planned",
        MigrationStatus::Approved => "approved",
        MigrationStatus::Running => "running",
        MigrationStatus::Paused => "paused",
        MigrationStatus::Completed => "completed",
        MigrationStatus::Failed => "failed",
        MigrationStatus::Verified => "verified",
    }
}

fn status_from_str(s: &str) -> MigrationStatus {
    match s {
        "analyzed" => MigrationStatus::Analyzed,
        "planned" => MigrationStatus::Planned,
        "approved" => MigrationStatus::Approved,
        "running" => MigrationStatus::Running,
        "paused" => MigrationStatus::Paused,
        "completed" => MigrationStatus::Completed,
        "failed" => MigrationStatus::Failed,
        "verified" => MigrationStatus::Verified,
        _ => MigrationStatus::Draft,
    }
}

pub async fn insert_migration(
    pool: &SqlitePool,
    migration: &Migration,
) -> Result<(), RelatingError> {
    let plan_json = serde_json::to_string(&migration.plan)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize plan: {}", e)))?;
    let progress_json = serde_json::to_string(&migration.progress)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize progress: {}", e)))?;
    let report_json = serde_json::to_string(&migration.report)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize report: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO migrations (id, project_id, connection_id, status, plan_json, progress_json, report_json, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(migration.id.to_string())
    .bind(migration.project_id.to_string())
    .bind(migration.connection_id.to_string())
    .bind(status_to_str(&migration.status))
    .bind(plan_json)
    .bind(progress_json)
    .bind(report_json)
    .bind(migration.created_at.to_rfc3339())
    .bind(migration.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert migration: {}", e)))?;

    Ok(())
}

fn map_migration_row(row: sqlx::sqlite::SqliteRow) -> Result<Migration, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let connection_id_str: String = row.get("connection_id");
    let status_str: String = row.get("status");
    let plan_json: String = row.get("plan_json");
    let progress_json: String = row.get("progress_json");
    let report_json: String = row.get("report_json");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    let id = MigrationId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Migration UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let connection_id = AuraConnectionId::parse(&connection_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid AuraConnection UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);

    let plan: serde_json::Value =
        serde_json::from_str(&plan_json).unwrap_or(serde_json::Value::Null);
    let progress: MigrationProgress = serde_json::from_str(&progress_json).unwrap_or_default();
    let report: serde_json::Value =
        serde_json::from_str(&report_json).unwrap_or(serde_json::Value::Null);

    Ok(Migration {
        id,
        project_id,
        connection_id,
        status: status_from_str(&status_str),
        plan,
        progress,
        report,
        created_at,
        updated_at,
    })
}

pub async fn get_migration(
    pool: &SqlitePool,
    id: &MigrationId,
) -> Result<Option<Migration>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, connection_id, status, plan_json, progress_json, report_json, created_at, updated_at
        FROM migrations
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch migration: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_migration_row(row)?)),
        None => Ok(None),
    }
}

pub async fn list_migrations(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<Migration>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, connection_id, status, plan_json, progress_json, report_json, created_at, updated_at
        FROM migrations
        WHERE project_id = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list migrations: {}", e)))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(map_migration_row(row)?);
    }
    Ok(out)
}

pub async fn update_migration(
    pool: &SqlitePool,
    migration: &Migration,
) -> Result<(), RelatingError> {
    let plan_json = serde_json::to_string(&migration.plan)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize plan: {}", e)))?;
    let progress_json = serde_json::to_string(&migration.progress)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize progress: {}", e)))?;
    let report_json = serde_json::to_string(&migration.report)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize report: {}", e)))?;

    sqlx::query(
        r#"
        UPDATE migrations
        SET status = ?, plan_json = ?, progress_json = ?, report_json = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(status_to_str(&migration.status))
    .bind(plan_json)
    .bind(progress_json)
    .bind(report_json)
    .bind(Utc::now().to_rfc3339())
    .bind(migration.id.to_string())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to update migration: {}", e)))?;

    Ok(())
}
