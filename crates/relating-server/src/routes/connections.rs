//! Data connectors: SQL database and REST API sources with managed sync.
//!
//! Connectors fetch rows on demand (`POST .../sync`), map them to normalized
//! records, and run the standard extract-and-load pipeline against the
//! project's approved ontology. Each connector owns one managed source
//! (`connector://<name>`) refreshed in place, plus watermark/hash state for
//! incremental syncs. Polling is cron-style: `GET .../connectors/due`
//! lists connectors past their interval; there is no background daemon.

use crate::error::AppError;
use crate::routes::aura::validate_auth_env_var;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use relating_connectors::{
    fetch_rest_rows, fetch_sql_rows, map_rows_to_records, rows_content_hash, FieldMapping,
    RestConfig, RowMap, SqlConfig,
};
use relating_core::ActorContext;
use relating_core::{ProjectId, Source, SourceId, SourceStatus};
use relating_ingest::{extract_entities_and_relationships, load_extracted_to_graph};
use relating_metadata::{
    delete_connector as db_delete_connector, find_source_by_filename, get_approved_ontology,
    get_project as db_get_project, get_project_connector, insert_connector as db_insert_connector,
    insert_source, list_connectors as db_list_connectors, list_due_connectors as db_list_due,
    update_connector_sync_state, update_source,
};
use serde::{Deserialize, Serialize};

fn parse_project(id_str: &str) -> Result<ProjectId, AppError> {
    ProjectId::parse(id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))
}

async fn require_project(state: &AppState, id: &ProjectId) -> Result<(), AppError> {
    db_get_project(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;
    Ok(())
}

#[derive(Deserialize)]
pub struct CreateConnectorRequest {
    pub name: String,
    /// `sql` or `rest`.
    pub kind: String,
    pub config: serde_json::Value,
    #[serde(default)]
    pub mapping: Option<serde_json::Value>,
    #[serde(default)]
    pub poll_interval_secs: Option<i64>,
}

/// Headers that must ride `bearer_env`/secret references instead of raw
/// values. Stored configs stay credential-free.
const SECRET_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-auth-token",
    "cookie",
    "proxy-authorization",
];

fn validate_sql_config(value: &serde_json::Value) -> Result<SqlConfig, AppError> {
    let config: SqlConfig = serde_json::from_value(value.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid sql config: {}", e)))?;
    config.validate()?;
    if let Some(var) = config.password_env.as_deref() {
        if !var.trim().is_empty() {
            validate_auth_env_var(Some(var.to_string()))?;
        }
    }
    Ok(config)
}

fn validate_rest_config(value: &serde_json::Value) -> Result<RestConfig, AppError> {
    let config: RestConfig = serde_json::from_value(value.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid rest config: {}", e)))?;
    config.validate()?;
    for name in config.headers.keys() {
        if SECRET_HEADERS.contains(&name.trim().to_ascii_lowercase().as_str()) {
            return Err(AppError::BadRequest(format!(
                "Header '{}' must use bearer_env instead of a raw value; stored configs stay credential-free",
                name
            )));
        }
    }
    for var in [&config.bearer_env, &config.basic_password_env] {
        if let Some(name) = var.as_deref() {
            if !name.trim().is_empty() {
                validate_auth_env_var(Some(name.to_string()))?;
            }
        }
    }
    Ok(config)
}

fn parse_mapping(value: Option<&serde_json::Value>) -> Result<FieldMapping, AppError> {
    match value {
        None => Ok(FieldMapping::default()),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| AppError::BadRequest(format!("Invalid mapping: {}", e))),
    }
}

pub async fn create_connector_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CreateConnectorRequest>,
) -> Result<(StatusCode, Json<relating_metadata::DataConnector>), AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "Connector name cannot be empty".to_string(),
        ));
    }
    let kind = payload.kind.trim().to_ascii_lowercase();
    if kind != "sql" && kind != "rest" {
        return Err(AppError::BadRequest(
            "Connector kind must be 'sql' or 'rest'".to_string(),
        ));
    }
    // Validate before persisting: bad configs fail here, never at 2am sync.
    match kind.as_str() {
        "sql" => validate_sql_config(&payload.config).map(|_| ())?,
        _ => validate_rest_config(&payload.config).map(|_| ())?,
    };
    let mapping = parse_mapping(payload.mapping.as_ref())?;
    let mapping_value = serde_json::to_value(&mapping)
        .map_err(|e| AppError::Internal(format!("Failed to serialize mapping: {}", e)))?;
    if let Some(interval) = payload.poll_interval_secs {
        if interval <= 0 {
            return Err(AppError::BadRequest(
                "poll_interval_secs must be positive".to_string(),
            ));
        }
    }
    let connector = db_insert_connector(
        &state.pool,
        &project_id.to_string(),
        name,
        &kind,
        &payload.config,
        &mapping_value,
        payload.poll_interval_secs,
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            AppError::BadRequest(format!("A connector named '{}' already exists", name))
        } else {
            AppError::Core(e)
        }
    })?;
    Ok((StatusCode::CREATED, Json(connector)))
}

pub async fn list_connectors_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<relating_metadata::DataConnector>>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    Ok(Json(
        db_list_connectors(&state.pool, &project_id.to_string()).await?,
    ))
}

pub async fn get_connector_handler(
    State(state): State<AppState>,
    Path((id_str, cid)): Path<(String, String)>,
) -> Result<Json<relating_metadata::DataConnector>, AppError> {
    let project_id = parse_project(&id_str)?;
    Ok(Json(
        get_project_connector(&state.pool, &project_id.to_string(), &cid)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Connector '{}' not found", cid)))?,
    ))
}

pub async fn delete_connector_handler(
    State(state): State<AppState>,
    Path((id_str, cid)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let project_id = parse_project(&id_str)?;
    // Scope check first: cross-project ids 404 instead of deleting.
    get_project_connector(&state.pool, &project_id.to_string(), &cid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Connector '{}' not found", cid)))?;
    db_delete_connector(&state.pool, &cid).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct ConnectorTestReport {
    pub ok: bool,
    pub kind: String,
    pub rows_seen: usize,
    pub columns: Vec<String>,
    pub detail: String,
}

/// Read-only probe: one row through the real fetch path, zero writes (no
/// source registration, no watermark updates).
pub async fn test_connector_handler(
    State(state): State<AppState>,
    Path((id_str, cid)): Path<(String, String)>,
) -> Result<Json<ConnectorTestReport>, AppError> {
    let project_id = parse_project(&id_str)?;
    let connector = get_project_connector(&state.pool, &project_id.to_string(), &cid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Connector '{}' not found", cid)))?;
    match connector.kind.as_str() {
        "sql" => {
            let mut config = validate_sql_config(&connector.config)?;
            config.limit_per_sync = Some(1);
            // Probe ignores watermarks: LIMIT 1 over the raw snapshot query.
            let plain = SqlConfig {
                watermark_column: None,
                ..config.clone()
            };
            let fetch = fetch_sql_rows(&plain, None).await?;
            let columns = fetch
                .rows
                .first()
                .map(|row| {
                    let mut keys: Vec<String> = row.keys().cloned().collect();
                    keys.sort();
                    keys
                })
                .unwrap_or_default();
            Ok(Json(ConnectorTestReport {
                ok: true,
                kind: "sql".to_string(),
                rows_seen: fetch.rows.len(),
                columns,
                detail: format!("Connected to {}", config.redacted_url()),
            }))
        }
        _ => {
            let mut config = validate_rest_config(&connector.config)?;
            config.pagination = Some(relating_connectors::PaginationConfig {
                mode: "offset".to_string(),
                offset_param: None,
                page_param: None,
                size_param: None,
                page_size: Some(1),
                max_pages: Some(1),
            });
            let fetch = fetch_rest_rows(&config).await?;
            let columns = fetch
                .rows
                .first()
                .map(|row| {
                    let mut keys: Vec<String> = row.keys().cloned().collect();
                    keys.sort();
                    keys
                })
                .unwrap_or_default();
            Ok(Json(ConnectorTestReport {
                ok: true,
                kind: "rest".to_string(),
                rows_seen: fetch.rows.len(),
                columns,
                detail: format!("Fetched one page from {}", config.base_url),
            }))
        }
    }
}

#[derive(Deserialize, Default)]
pub struct SyncQuery {
    /// Ignore watermarks and stored hashes; re-transfer everything.
    #[serde(default)]
    pub full: bool,
}

#[derive(Serialize)]
pub struct ConnectorSyncReport {
    pub connector_id: String,
    pub rows_fetched: usize,
    pub records_mapped: usize,
    pub rows_skipped: u64,
    pub unchanged: bool,
    pub nodes_created: i64,
    pub relationships_created: i64,
    pub watermark: Option<String>,
    pub elapsed_ms: f64,
}

fn managed_source_filename(connector_name: &str) -> String {
    format!("connector://{}", connector_name)
}

pub async fn sync_connector_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Path((id_str, cid)): Path<(String, String)>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<ConnectorSyncReport>, AppError> {
    let started = std::time::Instant::now();
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    let connector = get_project_connector(&state.pool, &project_id.to_string(), &cid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Connector '{}' not found", cid)))?;

    let approved_ontology = get_approved_ontology(&state.pool, &project_id)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(
                "An approved ontology is required before syncing a connector into the graph."
                    .to_string(),
            )
        })?;

    let mapping: FieldMapping = serde_json::from_value(connector.mapping.clone())
        .map_err(|e| AppError::Internal(format!("Stored connector mapping is corrupt: {}", e)))?;
    let watermark = (!query.full).then(|| connector.watermark.clone()).flatten();
    let (rows, new_watermark): (Vec<RowMap>, Option<String>) = match connector.kind.as_str() {
        "sql" => {
            let config = validate_sql_config(&connector.config)?;
            let fetch = fetch_sql_rows(&config, watermark.as_deref()).await?;
            (fetch.rows, fetch.new_watermark)
        }
        _ => {
            let config = validate_rest_config(&connector.config)?;
            let fetch = fetch_rest_rows(&config).await?;
            (fetch.rows, None)
        }
    };

    let sync_hash = rows_content_hash(&rows);
    let now = Utc::now();
    // Incremental short-circuit, two shapes: watermarked deltas arrive
    // empty when nothing changed, while full snapshots compare hashes.
    let delta_empty = rows.is_empty() && connector.watermark.is_some();
    let snapshot_same = !query.full
        && connector
            .sync_hash
            .as_deref()
            .is_some_and(|stored| stored == sync_hash);
    if !query.full && (delta_empty || snapshot_same) {
        update_connector_sync_state(
            &state.pool,
            &connector.id,
            connector.watermark.as_deref(),
            connector.sync_hash.as_deref().unwrap_or(&sync_hash),
            &now,
        )
        .await?;
        return Ok(Json(ConnectorSyncReport {
            connector_id: connector.id,
            rows_fetched: rows.len(),
            records_mapped: 0,
            rows_skipped: 0,
            unchanged: true,
            nodes_created: 0,
            relationships_created: 0,
            watermark: connector.watermark,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        }));
    }

    // Managed source: one row per connector, refreshed in place.
    let filename = managed_source_filename(&connector.name);
    let mut source = match find_source_by_filename(&state.pool, &project_id, &filename).await? {
        Some(existing) => existing,
        None => {
            let fresh = Source {
                id: SourceId::new(),
                project_id: project_id.clone(),
                filename: filename.clone(),
                content_hash: sync_hash.clone(),
                content_type: format!("application/x-connector+{}", connector.kind),
                byte_size: 0,
                status: SourceStatus::Pending,
                metadata: serde_json::json!({"connector_id": connector.id}),
                created_at: now,
            };
            insert_source(&state.pool, &fresh).await?;
            fresh
        }
    };

    let lineage_prefix = format!("{}:{}", connector.kind, connector.name);
    let (records, skipped) =
        map_rows_to_records(source.id.clone(), &rows, &mapping, &lineage_prefix);
    let (nodes, rels) = extract_entities_and_relationships(
        source.id.clone(),
        &filename,
        &records,
        &approved_ontology.schema,
    )?;
    let graph_path = state.project_graph_path(&project_id);
    let write_res = load_extracted_to_graph(state.graph.as_ref(), &graph_path, &nodes, &rels)?;

    source.content_hash = sync_hash.clone();
    source.status = SourceStatus::Processed;
    if let Some(meta) = source.metadata.as_object_mut() {
        meta.insert("connector_id".to_string(), serde_json::json!(connector.id));
        meta.insert("last_sync".to_string(), serde_json::json!(now.to_rfc3339()));
        meta.insert("last_row_count".to_string(), serde_json::json!(rows.len()));
    }
    update_source(&state.pool, &source).await?;
    let watermark_out = new_watermark.or(connector.watermark);
    update_connector_sync_state(
        &state.pool,
        &connector.id,
        watermark_out.as_deref(),
        &sync_hash,
        &now,
    )
    .await?;
    crate::routes::audit::audit_event(
        &state.pool,
        &actor.actor.display(),
        "connector.synced",
        &format!(
            "/api/v1/projects/{}/connectors/{}",
            project_id, connector.id
        ),
        "ok",
        &format!("{} rows", rows.len()),
    )
    .await;

    Ok(Json(ConnectorSyncReport {
        connector_id: connector.id,
        rows_fetched: rows.len(),
        records_mapped: records.len(),
        rows_skipped: skipped,
        unchanged: false,
        nodes_created: write_res.nodes_created,
        relationships_created: write_res.relationships_created,
        watermark: watermark_out,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    }))
}

/// Cron-style scheduler helper: connectors whose poll interval elapsed.
pub async fn due_connectors_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<relating_metadata::DataConnector>>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    Ok(Json(
        db_list_due(&state.pool, &project_id.to_string(), &Utc::now()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_managed_source_filename_is_stable() {
        assert_eq!(managed_source_filename("crm"), "connector://crm");
    }

    #[test]
    fn test_rest_rejects_raw_secret_headers() {
        let config = serde_json::json!({
            "base_url": "https://api.example/items",
            "headers": {"X-Api-Key": "live-secret"}
        });
        let err = validate_rest_config(&config).unwrap_err();
        assert!(err.to_string().contains("bearer_env"), "{}", err);
        let ok = serde_json::json!({
            "base_url": "https://api.example/items",
            "headers": {"X-Tenant": "acme"},
            "bearer_env": "MY_API_TOKEN"
        });
        assert!(validate_rest_config(&ok).is_ok());
    }
}
