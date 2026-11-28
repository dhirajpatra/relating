//! Append-only compliance audit log with a tamper-evident hash chain.
//!
//! Every row commits to its predecessor (`prev_hash` + field hashing), so a
//! deleted or edited row breaks verification. There is no update or delete
//! API: retention purges happen only through explicit operator SQL.

use chrono::{DateTime, Utc};
use relating_core::{sha256_hex, RelatingError};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

/// `prev_hash` of the first row, before any history exists.
pub const AUDIT_GENESIS_HASH: &str = "GENESIS";

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub seq: i64,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub outcome: String,
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
    pub created_at: DateTime<Utc>,
}

fn event_hash(
    prev_hash: &str,
    actor: &str,
    action: &str,
    resource: &str,
    outcome: &str,
    detail: &str,
    created_at: &str,
) -> String {
    sha256_hex(&format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        prev_hash, actor, action, resource, outcome, detail, created_at
    ))
}

fn event_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AuditEvent, RelatingError> {
    let created_at: String = row.get("created_at");
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RelatingError::Internal(format!("Invalid audit timestamp: {}", e)))?;
    Ok(AuditEvent {
        seq: row.get("seq"),
        actor: row.get("actor"),
        action: row.get("action"),
        resource: row.get("resource"),
        outcome: row.get("outcome"),
        detail: row.get("detail"),
        prev_hash: row.get("prev_hash"),
        hash: row.get("hash"),
        created_at,
    })
}

/// Append one event, chaining it onto the current tip. Single-writer
/// assumption: concurrent writers can fork the chain (both pass linkage but
/// share a predecessor), which `verify_audit_chain` reports.
pub async fn append_audit(
    pool: &SqlitePool,
    actor: &str,
    action: &str,
    resource: &str,
    outcome: &str,
    detail: &str,
) -> Result<AuditEvent, RelatingError> {
    let prev_hash: Option<String> =
        sqlx::query_scalar("SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(|e| RelatingError::Database(format!("Failed to read audit tip: {}", e)))?
            .flatten();
    let prev_hash = prev_hash.as_deref().unwrap_or(AUDIT_GENESIS_HASH);
    let created_at = Utc::now().to_rfc3339();
    let hash = event_hash(
        prev_hash,
        actor,
        action,
        resource,
        outcome,
        detail,
        &created_at,
    );
    sqlx::query(
        "INSERT INTO audit_log (actor, action, resource, outcome, detail, prev_hash, hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(resource)
    .bind(outcome)
    .bind(detail)
    .bind(prev_hash)
    .bind(&hash)
    .bind(&created_at)
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to append audit event: {}", e)))?;
    let seq: i64 = sqlx::query_scalar("SELECT seq FROM audit_log WHERE hash = ?")
        .bind(&hash)
        .fetch_one(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to read audit seq: {}", e)))?;
    Ok(AuditEvent {
        seq,
        actor: actor.to_string(),
        action: action.to_string(),
        resource: resource.to_string(),
        outcome: outcome.to_string(),
        detail: detail.to_string(),
        prev_hash: prev_hash.to_string(),
        hash,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| RelatingError::Internal(format!("Invalid audit timestamp: {}", e)))?,
    })
}

pub async fn list_audit(
    pool: &SqlitePool,
    after_seq: Option<i64>,
    actor: Option<&str>,
    limit: usize,
) -> Result<Vec<AuditEvent>, RelatingError> {
    let limit = limit.clamp(1, 5000) as i64;
    let rows = match (after_seq, actor) {
        (Some(after), Some(actor)) => sqlx::query(
            "SELECT seq, actor, action, resource, outcome, detail, prev_hash, hash, created_at \
             FROM audit_log WHERE seq > ? AND actor = ? ORDER BY seq ASC LIMIT ?",
        )
        .bind(after)
        .bind(actor)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to list audit events: {}", e)))?,
        (Some(after), None) => sqlx::query(
            "SELECT seq, actor, action, resource, outcome, detail, prev_hash, hash, created_at \
             FROM audit_log WHERE seq > ? ORDER BY seq ASC LIMIT ?",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to list audit events: {}", e)))?,
        (None, Some(actor)) => sqlx::query(
            "SELECT seq, actor, action, resource, outcome, detail, prev_hash, hash, created_at \
             FROM audit_log WHERE actor = ? ORDER BY seq ASC LIMIT ?",
        )
        .bind(actor)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to list audit events: {}", e)))?,
        (None, None) => sqlx::query(
            "SELECT seq, actor, action, resource, outcome, detail, prev_hash, hash, created_at \
             FROM audit_log ORDER BY seq ASC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to list audit events: {}", e)))?,
    };
    rows.iter().map(event_from_row).collect()
}

/// Raw audit row in column order: actor, action, resource, outcome, detail,
/// prev_hash, hash, created_at.
type RawAuditRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

/// Recompute every link. Returns the rows checked, or the first break found
/// (tampered fields, deleted head, or a forked predecessor).
pub async fn verify_audit_chain(pool: &SqlitePool) -> Result<u64, String> {
    let events = list_audit(pool, None, None, 5000)
        .await
        .map_err(|e| e.to_string())?;
    let mut expected_prev = AUDIT_GENESIS_HASH.to_string();
    for event in &events {
        if event.prev_hash != expected_prev {
            return Err(format!(
                "Audit chain break at seq {}: prev_hash does not match seq {} (deleted or forked history)",
                event.seq,
                event.seq - 1
            ));
        }
        expected_prev = event.hash.clone();
    }
    // Second pass verifies field hashes against stored text exactly, avoiding
    // timestamp-format drift between write and read paths.
    let rows: Vec<RawAuditRow> = sqlx::query_as(
        "SELECT actor, action, resource, outcome, detail, prev_hash, hash, created_at \
             FROM audit_log ORDER BY seq ASC LIMIT 5000",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for (actor, action, resource, outcome, detail, prev_hash, hash, created_at) in &rows {
        if event_hash(
            prev_hash, actor, action, resource, outcome, detail, created_at,
        ) != *hash
        {
            return Err(format!(
                "Audit chain break: row with hash {} fails field verification (edited values)",
                hash
            ));
        }
    }
    Ok(rows.len() as u64)
}
