use chrono::{DateTime, Utc};
use relating_core::{AuraConnection, AuraConnectionId, ProjectId, RelatingError};
use sqlx::{Row, SqlitePool};

pub async fn insert_aura_connection(
    pool: &SqlitePool,
    conn: &AuraConnection,
) -> Result<(), RelatingError> {
    sqlx::query(
        r#"
        INSERT INTO aura_connections (id, project_id, name, uri, username, database, auth_env_var, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(conn.id.to_string())
    .bind(conn.project_id.to_string())
    .bind(&conn.name)
    .bind(&conn.uri)
    .bind(&conn.username)
    .bind(conn.database.clone())
    .bind(&conn.auth_env_var)
    .bind(conn.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert Aura connection: {}", e)))?;

    Ok(())
}

fn map_connection_row(row: sqlx::sqlite::SqliteRow) -> Result<AuraConnection, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let name: String = row.get("name");
    let uri: String = row.get("uri");
    let username: String = row.get("username");
    let database: Option<String> = row.get("database");
    let auth_env_var: String = row.get("auth_env_var");
    let created_at_str: String = row.get("created_at");

    let id = AuraConnectionId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid AuraConnection UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);

    Ok(AuraConnection {
        id,
        project_id,
        name,
        uri,
        username,
        database,
        auth_env_var,
        created_at,
    })
}

pub async fn get_aura_connection(
    pool: &SqlitePool,
    id: &AuraConnectionId,
) -> Result<Option<AuraConnection>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, name, uri, username, database, auth_env_var, created_at
        FROM aura_connections
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch Aura connection: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_connection_row(row)?)),
        None => Ok(None),
    }
}

pub async fn list_aura_connections(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<AuraConnection>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, name, uri, username, database, auth_env_var, created_at
        FROM aura_connections
        WHERE project_id = ?
        ORDER BY created_at ASC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list Aura connections: {}", e)))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(map_connection_row(row)?);
    }
    Ok(out)
}

/// Fetch a connection by id, resolving the `.env` connection on a miss.
///
/// Stored connections always win. When no stored row matches, the
/// environment-derived connection is returned if its stable id matches the
/// requested id (and the password still resolves); otherwise `None`.
pub async fn get_aura_connection_resolving_env(
    pool: &SqlitePool,
    project_id: &ProjectId,
    id: &AuraConnectionId,
) -> Result<Option<AuraConnection>, RelatingError> {
    if let Some(conn) = get_aura_connection(pool, id).await? {
        return Ok(Some(conn));
    }
    Ok(relating_core::aura_connection_from_env(project_id).filter(|env| &env.id == id))
}

/// List stored connections plus the `.env` connection when available.
///
/// The env connection is appended only when no stored connection already
/// points at the same URI, so operators never see the same target twice.
pub async fn list_aura_connections_including_env(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<AuraConnection>, RelatingError> {
    let mut conns = list_aura_connections(pool, project_id).await?;
    if let Some(env) = relating_core::aura_connection_from_env(project_id) {
        if !conns.iter().any(|c| c.uri == env.uri) {
            conns.push(env);
        }
    }
    Ok(conns)
}

pub async fn delete_aura_connection(
    pool: &SqlitePool,
    id: &AuraConnectionId,
) -> Result<bool, RelatingError> {
    let result = sqlx::query("DELETE FROM aura_connections WHERE id = ?")
        .bind(id.to_string())
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to delete Aura connection: {}", e)))?;

    Ok(result.rows_affected() > 0)
}
