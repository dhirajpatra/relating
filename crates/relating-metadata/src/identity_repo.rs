//! Identity store: OIDC users, linked issuer identities, RBAC grants,
//! login sessions, and pending login round-trips.

use chrono::{DateTime, Utc};
use relating_core::{RelatingError, Role};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub id: String,
    pub user_id: String,
    pub issuer: String,
    pub subject: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Grant {
    pub id: String,
    pub user_id: String,
    pub project_id: Option<String>,
    pub role: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// Grant plus the grantee email, for admin UIs and audit export.
#[derive(Debug, Clone, Serialize)]
pub struct GrantView {
    #[serde(flatten)]
    pub grant: Grant,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PendingLogin {
    pub state: String,
    pub nonce: String,
    pub redirect_uri: String,
    pub expires_at: DateTime<Utc>,
}

fn parse_time(value: &str, column: &str) -> Result<DateTime<Utc>, RelatingError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RelatingError::Internal(format!("Invalid {} timestamp: {}", column, e)))
}

fn user_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<User, RelatingError> {
    let created_at: String = row.get("created_at");
    Ok(User {
        id: row.get("id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        created_at: parse_time(&created_at, "users.created_at")?,
    })
}

pub async fn find_user_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<User>, RelatingError> {
    let row = sqlx::query("SELECT id, email, display_name, created_at FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to look up user: {}", e)))?;
    row.map(|r| user_from_row(&r)).transpose()
}

pub async fn find_user_by_id(pool: &SqlitePool, id: &str) -> Result<Option<User>, RelatingError> {
    let row = sqlx::query("SELECT id, email, display_name, created_at FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to look up user: {}", e)))?;
    row.map(|r| user_from_row(&r)).transpose()
}

/// Find by email or create a shell row (identity links on first login).
/// Lets admins pre-grant people who have never signed in.
pub async fn find_or_create_user(
    pool: &SqlitePool,
    email: &str,
    display_name: Option<&str>,
) -> Result<User, RelatingError> {
    if let Some(user) = find_user_by_email(pool, email).await? {
        return Ok(user);
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(email)
        .bind(display_name)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to create user: {}", e)))?;
    Ok(User {
        id,
        email: email.to_string(),
        display_name: display_name.map(str::to_string),
        created_at: parse_time(&now, "users.created_at")?,
    })
}

pub async fn find_identity(
    pool: &SqlitePool,
    issuer: &str,
    subject: &str,
) -> Result<Option<Identity>, RelatingError> {
    let row = sqlx::query(
        "SELECT id, user_id, issuer, subject, created_at FROM identities WHERE issuer = ? AND subject = ?",
    )
    .bind(issuer)
    .bind(subject)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to look up identity: {}", e)))?;
    row.map(|r| {
        let created_at: String = r.get("created_at");
        Ok(Identity {
            id: r.get("id"),
            user_id: r.get("user_id"),
            issuer: r.get("issuer"),
            subject: r.get("subject"),
            created_at: parse_time(&created_at, "identities.created_at")?,
        })
    })
    .transpose()
}

pub async fn create_identity(
    pool: &SqlitePool,
    user_id: &str,
    issuer: &str,
    subject: &str,
) -> Result<Identity, RelatingError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO identities (id, user_id, issuer, subject, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(issuer)
    .bind(subject)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to link identity: {}", e)))?;
    Ok(Identity {
        id,
        user_id: user_id.to_string(),
        issuer: issuer.to_string(),
        subject: subject.to_string(),
        created_at: parse_time(&now, "identities.created_at")?,
    })
}

fn grant_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Grant, RelatingError> {
    let created_at: String = row.get("created_at");
    Ok(Grant {
        id: row.get("id"),
        user_id: row.get("user_id"),
        project_id: row.get("project_id"),
        role: row.get("role"),
        created_by: row.get("created_by"),
        created_at: parse_time(&created_at, "grants.created_at")?,
    })
}

pub async fn list_grants_for_project(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<GrantView>, RelatingError> {
    let rows = sqlx::query(
        "SELECT g.id, g.user_id, g.project_id, g.role, g.created_by, g.created_at, u.email \
         FROM grants g JOIN users u ON u.id = g.user_id \
         WHERE g.project_id = ? ORDER BY g.created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list grants: {}", e)))?;
    rows.iter()
        .map(|r| {
            Ok(GrantView {
                grant: grant_from_row(r)?,
                email: r.get("email"),
            })
        })
        .collect()
}

pub async fn list_global_grants(pool: &SqlitePool) -> Result<Vec<GrantView>, RelatingError> {
    let rows = sqlx::query(
        "SELECT g.id, g.user_id, g.project_id, g.role, g.created_by, g.created_at, u.email \
         FROM grants g JOIN users u ON u.id = g.user_id \
         WHERE g.project_id IS NULL ORDER BY g.created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list global grants: {}", e)))?;
    rows.iter()
        .map(|r| {
            Ok(GrantView {
                grant: grant_from_row(r)?,
                email: r.get("email"),
            })
        })
        .collect()
}

fn strongest_role(rows: &[String]) -> Option<Role> {
    rows.iter()
        .filter_map(|r| Role::parse(r))
        .max_by_key(|r| r.rank())
}

/// Strongest global (project-less) role held by the user, if any.
pub async fn user_global_role(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<Role>, RelatingError> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT role FROM grants WHERE user_id = ? AND project_id IS NULL")
            .bind(user_id)
            .fetch_all(pool)
            .await
            .map_err(|e| RelatingError::Database(format!("Failed to read global role: {}", e)))?;
    Ok(strongest_role(&rows))
}

/// Strongest role the user holds on one project, if any.
pub async fn user_project_role(
    pool: &SqlitePool,
    user_id: &str,
    project_id: &str,
) -> Result<Option<Role>, RelatingError> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT role FROM grants WHERE user_id = ? AND project_id = ?")
            .bind(user_id)
            .bind(project_id)
            .fetch_all(pool)
            .await
            .map_err(|e| RelatingError::Database(format!("Failed to read project role: {}", e)))?;
    Ok(strongest_role(&rows))
}

pub async fn create_grant(
    pool: &SqlitePool,
    user_id: &str,
    project_id: Option<&str>,
    role: Role,
    created_by: &str,
) -> Result<Grant, RelatingError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO grants (id, user_id, project_id, role, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(project_id)
    .bind(role.as_str())
    .bind(created_by)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to create grant: {}", e)))?;
    Ok(Grant {
        id,
        user_id: user_id.to_string(),
        project_id: project_id.map(str::to_string),
        role: role.as_str().to_string(),
        created_by: created_by.to_string(),
        created_at: parse_time(&now, "grants.created_at")?,
    })
}

pub async fn delete_grant(pool: &SqlitePool, grant_id: &str) -> Result<bool, RelatingError> {
    let done = sqlx::query("DELETE FROM grants WHERE id = ?")
        .bind(grant_id)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to delete grant: {}", e)))?;
    Ok(done.rows_affected() > 0)
}

pub async fn create_session(
    pool: &SqlitePool,
    user_id: &str,
    token_hash: &str,
    expires_at: &DateTime<Utc>,
) -> Result<Session, RelatingError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to create session: {}", e)))?;
    Ok(Session {
        id,
        user_id: user_id.to_string(),
        token_hash: token_hash.to_string(),
        expires_at: *expires_at,
        created_at: now,
    })
}

pub async fn find_session_by_hash(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<Session>, RelatingError> {
    let row = sqlx::query(
        "SELECT id, user_id, token_hash, expires_at, created_at FROM sessions WHERE token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to look up session: {}", e)))?;
    row.map(|r| {
        let expires_at: String = r.get("expires_at");
        let created_at: String = r.get("created_at");
        Ok(Session {
            id: r.get("id"),
            user_id: r.get("user_id"),
            token_hash: r.get("token_hash"),
            expires_at: parse_time(&expires_at, "sessions.expires_at")?,
            created_at: parse_time(&created_at, "sessions.created_at")?,
        })
    })
    .transpose()
}

pub async fn delete_expired_sessions(pool: &SqlitePool, now: &DateTime<Utc>) -> u64 {
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .map(|done| done.rows_affected())
        .unwrap_or(0)
}

pub async fn create_pending_login(
    pool: &SqlitePool,
    state: &str,
    nonce: &str,
    redirect_uri: &str,
    expires_at: &DateTime<Utc>,
) -> Result<(), RelatingError> {
    sqlx::query(
        "INSERT INTO pending_logins (state, nonce, redirect_uri, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(state)
    .bind(nonce)
    .bind(redirect_uri)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to start login: {}", e)))?;
    Ok(())
}

/// Take a pending login exactly once: returns it only when unexpired, and
/// always consumes the row so states never replay.
pub async fn take_pending_login(
    pool: &SqlitePool,
    state: &str,
    now: &DateTime<Utc>,
) -> Result<Option<PendingLogin>, RelatingError> {
    let row = sqlx::query(
        "SELECT state, nonce, redirect_uri, expires_at FROM pending_logins WHERE state = ?",
    )
    .bind(state)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to read login state: {}", e)))?;
    let Some(row) = row else { return Ok(None) };
    let expires_at: String = row.get("expires_at");
    let pending = PendingLogin {
        state: row.get("state"),
        nonce: row.get("nonce"),
        redirect_uri: row.get("redirect_uri"),
        expires_at: parse_time(&expires_at, "pending_logins.expires_at")?,
    };
    sqlx::query("DELETE FROM pending_logins WHERE state = ?")
        .bind(state)
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to consume login state: {}", e)))?;
    if pending.expires_at <= *now {
        return Ok(None);
    }
    Ok(Some(pending))
}
