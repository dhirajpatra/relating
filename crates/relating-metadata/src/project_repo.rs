use chrono::{DateTime, Utc};
use relating_core::{AiProviderConfig, Project, ProjectId, RelatingError};
use sqlx::{Row, SqlitePool};

pub async fn insert_project(pool: &SqlitePool, project: &Project) -> Result<(), RelatingError> {
    let ai_config_json = serde_json::to_string(&project.ai_config)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize AI config: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO projects (id, name, description, created_at, updated_at, ai_config)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(project.id.to_string())
    .bind(&project.name)
    .bind(&project.description)
    .bind(project.created_at.to_rfc3339())
    .bind(project.updated_at.to_rfc3339())
    .bind(ai_config_json)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert project: {}", e)))?;

    Ok(())
}

pub async fn get_project(pool: &SqlitePool, id: &ProjectId) -> Result<Option<Project>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, name, description, created_at, updated_at, ai_config
        FROM projects
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch project: {}", e)))?;

    match row {
        Some(row) => {
            let id_str: String = row.get("id");
            let name: String = row.get("name");
            let description: String = row.get("description");
            let created_at_str: String = row.get("created_at");
            let updated_at_str: String = row.get("updated_at");
            let ai_config_json: String = row.get("ai_config");

            let id = ProjectId::parse(&id_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid UUID in DB: {}", e)))?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid created_at timestamp: {}", e)))?
                .with_timezone(&Utc);
            let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| RelatingError::Internal(format!("Invalid updated_at timestamp: {}", e)))?
                .with_timezone(&Utc);
            let ai_config: AiProviderConfig = serde_json::from_str(&ai_config_json)
                .map_err(|e| RelatingError::Internal(format!("Failed to deserialize AI config: {}", e)))?;

            Ok(Some(Project {
                id,
                name,
                description,
                created_at,
                updated_at,
                ai_config,
            }))
        }
        None => Ok(None),
    }
}

pub async fn list_projects(pool: &SqlitePool) -> Result<Vec<Project>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, description, created_at, updated_at, ai_config
        FROM projects
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list projects: {}", e)))?;

    let mut projects = Vec::with_capacity(rows.len());
    for row in rows {
        let id_str: String = row.get("id");
        let name: String = row.get("name");
        let description: String = row.get("description");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let ai_config_json: String = row.get("ai_config");

        let id = ProjectId::parse(&id_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid UUID in DB: {}", e)))?;
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid created_at timestamp: {}", e)))?
            .with_timezone(&Utc);
        let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| RelatingError::Internal(format!("Invalid updated_at timestamp: {}", e)))?
            .with_timezone(&Utc);
        let ai_config: AiProviderConfig = serde_json::from_str(&ai_config_json)
            .map_err(|e| RelatingError::Internal(format!("Failed to deserialize AI config: {}", e)))?;

        projects.push(Project {
            id,
            name,
            description,
            created_at,
            updated_at,
            ai_config,
        });
    }

    Ok(projects)
}

pub async fn delete_project(pool: &SqlitePool, id: &ProjectId) -> Result<bool, RelatingError> {
    let result = sqlx::query(
        r#"
        DELETE FROM projects
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to delete project: {}", e)))?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_in_memory_db;

    #[tokio::test]
    async fn test_project_crud() {
        let pool = init_in_memory_db().await.unwrap();

        let project = Project {
            id: ProjectId::new(),
            name: "Test Project".to_string(),
            description: "A test project description".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ai_config: AiProviderConfig::mock(),
        };

        insert_project(&pool, &project).await.unwrap();

        let fetched = get_project(&pool, &project.id).await.unwrap().expect("Project should exist");
        assert_eq!(fetched.name, "Test Project");
        assert_eq!(fetched.description, "A test project description");

        let list = list_projects(&pool).await.unwrap();
        assert_eq!(list.len(), 1);

        let deleted = delete_project(&pool, &project.id).await.unwrap();
        assert!(deleted);

        let fetched_after = get_project(&pool, &project.id).await.unwrap();
        assert!(fetched_after.is_none());
    }
}
