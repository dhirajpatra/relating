//! Compliance audit log: best-effort recording plus export.
//!
//! The auth middleware records every gated request; domain handlers add
//! explicit events (logins, grants, approvals, executions). Export filters to
//! one project by resource path and always verifies the full hash chain, so
//! reviewers see tamper evidence alongside the rows.

use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use relating_metadata::DbPool;
use serde::Deserialize;

/// Best-effort audit append: a failing audit store must never fail the
/// request it records, so errors are traced and swallowed.
pub async fn audit_event(
    pool: &DbPool,
    actor: &str,
    action: &str,
    resource: &str,
    outcome: &str,
    detail: &str,
) {
    if let Err(e) =
        relating_metadata::append_audit(pool, actor, action, resource, outcome, detail).await
    {
        tracing::warn!("audit append failed: {}", e);
    }
}

#[derive(Deserialize, Default)]
pub struct ExportQuery {
    /// `json` (default) or `csv`.
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub after_seq: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub async fn export_audit_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, AppError> {
    let project_id = relating_core::ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;
    relating_metadata::get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let events = relating_metadata::list_audit(
        &state.pool,
        query.after_seq,
        query.actor.as_deref(),
        query.limit.unwrap_or(1000),
    )
    .await?;
    // Project scope is a resource-path filter: rows whose resource mentions
    // this project. Global auth events stay visible in the unfiltered log.
    let needle = project_id.to_string();
    let scoped: Vec<_> = events
        .into_iter()
        .filter(|e| e.resource.contains(&needle))
        .collect();
    let chain = match relating_metadata::verify_audit_chain(&state.pool).await {
        Ok(checked) => serde_json::json!({"valid": true, "checked": checked}),
        Err(break_desc) => serde_json::json!({"valid": false, "error": break_desc}),
    };

    let format = query.format.as_deref().unwrap_or("json");
    if format == "csv" {
        let mut out =
            String::from("seq,actor,action,resource,outcome,detail,prev_hash,hash,created_at\n");
        for e in &scoped {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                e.seq,
                csv_cell(&e.actor),
                csv_cell(&e.action),
                csv_cell(&e.resource),
                csv_cell(&e.outcome),
                csv_cell(&e.detail),
                csv_cell(&e.prev_hash),
                csv_cell(&e.hash),
                e.created_at.to_rfc3339(),
            ));
        }
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            out,
        )
            .into_response());
    }
    if format != "json" {
        return Err(AppError::BadRequest(format!(
            "Unknown audit format '{}': use json or csv",
            format
        )));
    }
    Ok(Json(serde_json::json!({
        "project_id": project_id.to_string(),
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "chain": chain,
        "event_count": scoped.len(),
        "events": scoped,
    }))
    .into_response())
}
