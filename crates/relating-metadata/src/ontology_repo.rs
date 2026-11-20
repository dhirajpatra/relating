use chrono::{DateTime, Utc};
use relating_core::{Ontology, OntologyId, OntologySchema, OntologyStatus, ProjectId, RelatingError};
use sqlx::{Row, SqlitePool};

pub async fn insert_ontology(pool: &SqlitePool, ontology: &Ontology) -> Result<(), RelatingError> {
    let status_str = match ontology.status {
        OntologyStatus::Proposed => "proposed",
        OntologyStatus::Approved => "approved",
        OntologyStatus::Rejected => "rejected",
    };

    let schema_json = serde_json::to_string(&ontology.schema)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize ontology schema: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO ontologies (id, project_id, version, status, schema_json, created_at, approved_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(ontology.id.to_string())
    .bind(ontology.project_id.to_string())
    .bind(ontology.version as i64)
    .bind(status_str)
    .bind(schema_json)
    .bind(ontology.created_at.to_rfc3339())
    .bind(ontology.approved_at.map(|t| t.to_rfc3339()))
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert ontology: {}", e)))?;

    Ok(())
}

pub async fn get_latest_ontology(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Option<Ontology>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, version, status, schema_json, created_at, approved_at
        FROM ontologies
        WHERE project_id = ?
        ORDER BY version DESC, created_at DESC
        LIMIT 1
        "#,
    )
    .bind(project_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch latest ontology: {}", e)))?;

    map_ontology_row(row)
}

pub async fn get_approved_ontology(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Option<Ontology>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, version, status, schema_json, created_at, approved_at
        FROM ontologies
        WHERE project_id = ? AND status = 'approved'
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(project_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch approved ontology: {}", e)))?;

    map_ontology_row(row)
}

pub async fn approve_ontology(
    pool: &SqlitePool,
    id: &OntologyId,
) -> Result<bool, RelatingError> {
    let now = Utc::now();
    let result = sqlx::query(
        r#"
        UPDATE ontologies
        SET status = 'approved', approved_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now.to_rfc3339())
    .bind(id.to_string())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to approve ontology: {}", e)))?;

    Ok(result.rows_affected() > 0)
}

fn map_ontology_row(row: Option<sqlx::sqlite::SqliteRow>) -> Result<Option<Ontology>, RelatingError> {
    match row {
        Some(row) => {
            let id_str: String = row.get("id");
            let project_id_str: String = row.get("project_id");
            let version: i64 = row.get("version");
            let status_str: String = row.get("status");
            let schema_json: String = row.get("schema_json");
            let created_at_str: String = row.get("created_at");
            let approved_at_str: Option<String> = row.get("approved_at");

            let id = OntologyId::parse(&id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid Ontology UUID: {}", e)))?;
            let project_id = ProjectId::parse(&project_id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc);
            let approved_at = approved_at_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s).ok().map(|t| t.with_timezone(&Utc))
            });

            let status = match status_str.as_str() {
                "approved" => OntologyStatus::Approved,
                "rejected" => OntologyStatus::Rejected,
                _ => OntologyStatus::Proposed,
            };

            let schema: OntologySchema = serde_json::from_str(&schema_json)
                .map_err(|e| RelatingError::Internal(format!("Failed to deserialize ontology schema: {}", e)))?;

            Ok(Some(Ontology {
                id,
                project_id,
                version: version as u32,
                status,
                schema,
                created_at,
                approved_at,
            }))
        }
        None => Ok(None),
    }
}
