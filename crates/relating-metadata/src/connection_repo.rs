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
        // A stored row belongs to exactly one project. The `.env` connection
        // has one stable id for every project, so once migration planning
        // materializes it for project A, projects B, C, ... must NOT resolve
        // A's row here (their proxy calls would fail the project check with
        // "Connection not found" while test/analyze silently used A's row).
        // Other projects fall through to their own transient env connection.
        if conn.project_id == *project_id {
            return Ok(Some(conn));
        }
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

/// Persist a resolved connection if no stored row carries its id.
///
/// Migrations reference `aura_connections(id)` via foreign key, so planning
/// against the transient `.env` connection materializes it first (same
/// stable id, so later steps resolve the identical target). Read paths
/// never call this: listing and testing keep working against the transient
/// connection without writing. On a display-name clash with an unrelated
/// stored row, the materialized copy takes a suffixed name.
///
/// A stored row that is itself env-managed (its id is the stable id of its
/// own URI) is refreshed from the current environment when the URI still
/// matches but other fields drifted — so correcting `.env` (for example a
/// wrong `AURA_DATABASE`) heals the materialized row instead of leaving a
/// stale copy that keeps shadowing the fix.
pub async fn ensure_aura_connection(
    pool: &SqlitePool,
    conn: &AuraConnection,
) -> Result<AuraConnection, RelatingError> {
    if let Some(stored) = get_aura_connection(pool, &conn.id).await? {
        return Ok(refresh_env_managed_row(pool, &stored)
            .await?
            .unwrap_or(stored));
    }
    let mut candidate = conn.clone();
    for attempt in 0..4 {
        if attempt > 0 {
            candidate.name = format!("{} {}", conn.name, attempt + 1);
        }
        let inserted = sqlx::query(
            r#"
            INSERT OR IGNORE INTO aura_connections (id, project_id, name, uri, username, database, auth_env_var, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(candidate.id.to_string())
        .bind(candidate.project_id.to_string())
        .bind(&candidate.name)
        .bind(&candidate.uri)
        .bind(&candidate.username)
        .bind(candidate.database.clone())
        .bind(&candidate.auth_env_var)
        .bind(candidate.created_at.to_rfc3339())
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to insert Aura connection: {}", e)))?;
        if inserted.rows_affected() > 0 {
            return Ok(candidate);
        }
        // Either another writer won the race (return the stored row) or the
        // name clashed with an unrelated row (retry with a suffix).
        if let Some(stored) = get_aura_connection(pool, &conn.id).await? {
            return Ok(stored);
        }
    }
    Err(RelatingError::Database(
        "Failed to materialize the Aura connection: display-name clash".to_string(),
    ))
}

/// Refresh an env-managed stored row from the current environment.
///
/// Returns `Ok(None)` when the row is user-managed (random id) or the
/// environment no longer describes its URI (changed `AURA_URI` or missing
/// password): such rows are left untouched. Otherwise returns the row with
/// name/username/database/auth brought in sync when they drifted.
async fn refresh_env_managed_row(
    pool: &SqlitePool,
    stored: &AuraConnection,
) -> Result<Option<AuraConnection>, RelatingError> {
    if relating_core::aura_env_connection_id(&stored.uri) != stored.id {
        return Ok(None);
    }
    let Some(current) = relating_core::aura_connection_from_env(&stored.project_id) else {
        return Ok(None);
    };
    if current.uri != stored.uri {
        return Ok(None);
    }
    if current.name == stored.name
        && current.username == stored.username
        && current.database == stored.database
        && current.auth_env_var == stored.auth_env_var
    {
        return Ok(None);
    }
    sqlx::query(
        r#"
        UPDATE aura_connections
        SET name = ?, username = ?, database = ?, auth_env_var = ?
        WHERE id = ?
        "#,
    )
    .bind(&current.name)
    .bind(&current.username)
    .bind(current.database.clone())
    .bind(&current.auth_env_var)
    .bind(stored.id.to_string())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to refresh Aura connection: {}", e)))?;
    let mut refreshed = stored.clone();
    refreshed.name = current.name;
    refreshed.username = current.username;
    refreshed.database = current.database;
    refreshed.auth_env_var = current.auth_env_var;
    Ok(Some(refreshed))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_in_memory_db;
    use crate::migration_repo::insert_migration;
    use crate::project_repo::insert_project;
    use relating_core::{AiProviderConfig, Migration, MigrationId, MigrationStatus, Project};
    use std::sync::Mutex;

    // Process env is global; serialize the tests that mutate AURA_*.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct AuraEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    impl AuraEnvGuard {
        fn set(uri: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap();
            let saved: Vec<(String, Option<String>)> = [
                "AURA_URI",
                "AURA_USERNAME",
                "AURA_DATABASE",
                "AURA_NAME",
                "AURA_PASSWORD",
                "AURA_PASSWORD_FILE",
                "RELATING_SECRET_BACKEND",
            ]
            .iter()
            .map(|n| (n.to_string(), std::env::var(n).ok()))
            .collect();
            for (name, _) in &saved {
                std::env::remove_var(name);
            }
            std::env::set_var("AURA_URI", uri);
            std::env::set_var("AURA_PASSWORD", "test-password");
            Self {
                _lock: lock,
                saved,
            }
        }
    }

    impl Drop for AuraEnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    async fn seed_project(pool: &SqlitePool) -> Project {
        let now = Utc::now();
        let project = Project {
            id: ProjectId::new(),
            name: "conn-test".to_string(),
            description: String::new(),
            created_at: now,
            updated_at: now,
            ai_config: AiProviderConfig::mock(),
        };
        insert_project(pool, &project).await.unwrap();
        project
    }

    fn transient_conn(project: &Project) -> AuraConnection {
        AuraConnection {
            id: AuraConnectionId::new(),
            project_id: project.id.clone(),
            name: "aura (env)".to_string(),
            uri: "neo4j+s://example.databases.neo4j.io".to_string(),
            username: "neo4j".to_string(),
            database: None,
            auth_env_var: "AURA_PASSWORD".to_string(),
            created_at: Utc::now(),
        }
    }

    fn migration_for(project: &Project, conn_id: &AuraConnectionId) -> Migration {
        let now = Utc::now();
        Migration {
            id: MigrationId::new(),
            project_id: project.id.clone(),
            connection_id: conn_id.clone(),
            status: MigrationStatus::Planned,
            plan: serde_json::Value::Null,
            progress: Default::default(),
            report: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn test_ensure_materializes_transient_connection_for_migrations() {
        let pool = init_in_memory_db().await.unwrap();
        let project = seed_project(&pool).await;
        let conn = transient_conn(&project);

        // Reproduces the plan-time failure: a migration cannot reference a
        // connection row that does not exist.
        let before = insert_migration(&pool, &migration_for(&project, &conn.id)).await;
        assert!(
            before.is_err(),
            "expected the FK to reject a transient connection id"
        );

        let stored = ensure_aura_connection(&pool, &conn).await.unwrap();
        assert_eq!(stored.id, conn.id);
        assert_eq!(stored.uri, conn.uri);

        insert_migration(&pool, &migration_for(&project, &conn.id))
            .await
            .expect("migration insert must succeed after ensure");

        // Idempotent: second ensure returns the row without duplicating.
        let again = ensure_aura_connection(&pool, &conn).await.unwrap();
        assert_eq!(again.id, conn.id);
        let rows = list_aura_connections(&pool, &project.id).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn test_ensure_suffixes_name_on_clash() {
        let pool = init_in_memory_db().await.unwrap();
        let project = seed_project(&pool).await;

        let mut occupant = transient_conn(&project);
        occupant.id = AuraConnectionId::new();
        occupant.uri = "bolt://localhost:7687".to_string();
        insert_aura_connection(&pool, &occupant).await.unwrap();

        let conn = transient_conn(&project);
        let stored = ensure_aura_connection(&pool, &conn).await.unwrap();
        assert_eq!(stored.id, conn.id);
        assert_eq!(stored.name, "aura (env) 2");
    }

    #[tokio::test]
    async fn test_ensure_refreshes_env_managed_row_on_drift() {
        // Shares process env with the test below; serialized via ENV_LOCK.
        let _serial = ENV_LOCK.lock().unwrap();
        let saved: Vec<(String, Option<String>)> = [
            "AURA_URI",
            "AURA_USERNAME",
            "AURA_DATABASE",
            "AURA_NAME",
            "AURA_PASSWORD",
            "AURA_PASSWORD_FILE",
            "RELATING_SECRET_BACKEND",
        ]
        .iter()
        .map(|n| (n.to_string(), std::env::var(n).ok()))
        .collect();
        std::env::set_var("AURA_URI", "neo4j+s://refresh.databases.neo4j.io");
        std::env::set_var("AURA_PASSWORD", "test-password");
        std::env::set_var("AURA_DATABASE", "wrongdb");

        let outcome = async {
            let pool = init_in_memory_db().await.unwrap();
            let project = seed_project(&pool).await;

            // Materialize with the wrong database, as an earlier plan did.
            let stale =
                relating_core::aura_connection_from_env(&project.id).expect("env connection");
            assert_eq!(stale.database.as_deref(), Some("wrongdb"));
            insert_aura_connection(&pool, &stale).await.unwrap();

            // Correct .env, then ensure heals the row instead of shadowing it.
            std::env::set_var("AURA_DATABASE", "neo4j");
            let fixed =
                relating_core::aura_connection_from_env(&project.id).expect("env connection");
            let stored = ensure_aura_connection(&pool, &fixed).await.unwrap();
            assert_eq!(stored.id, stale.id);
            assert_eq!(stored.database.as_deref(), Some("neo4j"));
            let refetched = get_aura_connection(&pool, &stale.id)
                .await
                .unwrap()
                .expect("row persists");
            assert_eq!(refetched.database.as_deref(), Some("neo4j"));
        }
        .await;

        for (name, value) in saved {
            match value {
                Some(v) => std::env::set_var(&name, v),
                None => std::env::remove_var(&name),
            }
        }
        outcome
    }

    #[tokio::test]
    async fn test_resolving_env_does_not_leak_across_projects() {
        let _env = AuraEnvGuard::set("neo4j+s://xscope.databases.neo4j.io");

        let pool = init_in_memory_db().await.unwrap();
        let project_a = seed_project(&pool).await;
        let project_b = seed_project(&pool).await;

        // Plan-time ensure materializes the env connection for project A.
        let env_a =
            relating_core::aura_connection_from_env(&project_a.id).expect("env connection");
        insert_aura_connection(&pool, &env_a).await.unwrap();

        // Project A resolves its stored row.
        let got_a = get_aura_connection_resolving_env(&pool, &project_a.id, &env_a.id)
            .await
            .unwrap()
            .expect("project A resolves");
        assert_eq!(got_a.project_id, project_a.id);

        // Project B resolves its OWN transient env connection (same stable
        // id, its own project) — never A's stored row. Before the
        // project-scoped fix this returned A's row, and proxy calls from B
        // failed the project check with "Connection not found".
        let got_b = get_aura_connection_resolving_env(&pool, &project_b.id, &env_a.id)
            .await
            .unwrap()
            .expect("project B resolves");
        assert_eq!(got_b.id, env_a.id);
        assert_eq!(got_b.project_id, project_b.id);
    }
}
