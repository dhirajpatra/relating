use crate::error::AppError;
use crate::routes::aura::{build_backend, get_connection};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use relating_core::{
    Actor, ActorContext, AuraConnectionId, Migration, MigrationId, MigrationProgress,
    MigrationStatus, ProjectId,
};
use relating_graph::GraphBackend;
use relating_metadata::{
    get_migration as db_get_migration, get_project as db_get_project, insert_migration,
    list_migrations as db_list_migrations, update_migration,
};
use relating_migrate::{
    analyze_project_with_options, dry_run, is_hash_fallback_uid, json_to_bolt,
    migrate_node_batch_with_options, migrate_relationship_batch_with_options,
    preview_migration_with_options, propagate_node_deletes, propagate_rel_deletes,
    rollback_migration, source_blockers_for_dry_run, uid_list_hash, vector_blockers_for_project,
    verify_migration, AnalysisOptions, BoltValue, CompatFinding, DegreeExpectation, DryRunReport,
    MigrationPlan, Neo4jBackend, RollbackOutcome, SampleExpectation, SourceCounts,
    VerificationReport, DEFAULT_BATCH_SIZE, MAX_HASH_UIDS_PER_LABEL, MIGRATION_TAG_PROPERTY,
    UPDATED_AT_PROPERTY, VERSION_PROPERTY,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
pub struct AnalyzeRequest {
    #[serde(default)]
    pub connection_id: Option<String>,
    /// Optional target Aura tier for vector RAM checks: a preset id
    /// (`free`, `professional-4gb`, ...), `<N>gb`, `<N>mb`, or `custom:<mb>`.
    #[serde(default)]
    pub aura_tier: Option<String>,
}

#[derive(Serialize)]
pub struct AnalyzeResponse {
    pub source_findings: Vec<CompatFinding>,
    pub target_findings: Vec<CompatFinding>,
    pub target_reachable: bool,
    pub target_agent: Option<String>,
}

#[derive(Deserialize)]
pub struct PlanRequest {
    pub connection_id: String,
    #[serde(default)]
    pub batch_size: Option<usize>,
    /// Optional target Aura tier for capacity pre-flight (`free`,
    /// `professional-4gb`, `<N>gb`, `<N>mb`, `custom:<mb>`). Stored on the
    /// plan so later dry-runs re-check the same spec.
    #[serde(default)]
    pub aura_tier: Option<String>,
}

#[derive(Serialize)]
pub struct PlanResponse {
    pub migration: Migration,
    pub dry_run: DryRunReport,
}

#[derive(Deserialize)]
pub struct ApproveRequest {
    pub approved: bool,
}

fn escape_cypher_label(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Stable node page scan: ORDER BY uid makes SKIP/LIMIT deterministic across
/// concurrent writes. Skip/limit are internally computed integers only.
fn build_node_page_query(label: &str, skip: usize, limit: usize) -> String {
    format!(
        "MATCH (n:{}) RETURN n AS node ORDER BY n.uid SKIP {} LIMIT {}",
        escape_cypher_label(label),
        skip,
        limit
    )
}

/// Stable relationship page scan: ORDER BY endpoints keeps pages stable.
fn build_rel_page_query(rel_type: &str, skip: usize, limit: usize) -> String {
    format!(
        "MATCH (s)-[r:{}]->(t) RETURN s.uid AS suid, t.uid AS tuid, r AS rel ORDER BY s.uid, t.uid SKIP {} LIMIT {}",
        escape_cypher_label(rel_type),
        skip,
        limit
    )
}

/// Stable uid listing for content-hash verification.
fn build_uid_list_query(label: &str) -> String {
    format!(
        "MATCH (n:{}) RETURN n.uid AS uid ORDER BY uid",
        escape_cypher_label(label)
    )
}

fn parse_project(id_str: &str) -> Result<ProjectId, AppError> {
    ProjectId::parse(id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))
}

async fn require_project(state: &AppState, id: &ProjectId) -> Result<(), AppError> {
    let _ = db_get_project(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;
    Ok(())
}

fn parse_plan(migration: &Migration) -> Result<MigrationPlan, AppError> {
    serde_json::from_value(migration.plan.clone())
        .map_err(|e| AppError::Internal(format!("Stored migration plan is corrupt: {}", e)))
}

pub async fn analyze_migration_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;

    let graph_path = state.project_graph_path(&project_id);
    let connection = match payload.connection_id {
        Some(conn_id_str) => {
            let conn_id = AuraConnectionId::parse(&conn_id_str).map_err(|e| {
                AppError::BadRequest(format!("Invalid Connection UUID '{}': {}", conn_id_str, e))
            })?;
            Some(get_connection(&state.pool, &project_id, &conn_id).await?)
        }
        None => None,
    };

    let report = analyze_project_with_options(
        &state.pool,
        state.graph.as_ref(),
        &project_id,
        &graph_path,
        connection.as_ref(),
        AnalysisOptions {
            aura_tier: payload.aura_tier,
        },
    )
    .await?;

    Ok(Json(AnalyzeResponse {
        source_findings: report.source_findings,
        target_findings: report.target_findings,
        target_reachable: report.target_reachable,
        target_agent: report.target_agent,
    }))
}

pub async fn plan_migration_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Path(id_str): Path<String>,
    Json(payload): Json<PlanRequest>,
) -> Result<(StatusCode, Json<PlanResponse>), AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;

    let conn_id = AuraConnectionId::parse(&payload.connection_id).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid Connection UUID '{}': {}",
            payload.connection_id, e
        ))
    })?;
    let conn = get_connection(&state.pool, &project_id, &conn_id).await?;
    // Migrations reference aura_connections(id) via foreign key: persist the
    // resolved connection (a no-op for stored rows) so planning against the
    // transient `.env` connection does not fail the insert below.
    let conn = relating_metadata::ensure_aura_connection(&state.pool, &conn).await?;

    let graph_path = state.project_graph_path(&project_id);
    let batch_size = payload
        .batch_size
        .unwrap_or(DEFAULT_BATCH_SIZE)
        .clamp(1, 5000);
    let (plan, mut report) = preview_migration_with_options(
        &state.pool,
        state.graph.as_ref(),
        &project_id,
        &graph_path,
        &conn,
        batch_size,
        payload.aura_tier,
    )
    .await?;
    // Deterministic vector blockers (embedding dims Aura cannot index) gate
    // approval exactly like unreachable targets and empty sources.
    report
        .blockers
        .extend(vector_blockers_for_project(&state.pool, &project_id).await?);

    let now = Utc::now();
    let mut migration = Migration {
        id: MigrationId::new(),
        project_id,
        connection_id: conn.id.clone(),
        status: MigrationStatus::Planned,
        plan: serde_json::to_value(&plan)
            .map_err(|e| AppError::Internal(format!("Failed to serialize plan: {}", e)))?,
        progress: MigrationProgress {
            batches_total: plan.total_batches,
            created_by: Some(actor.actor.display()),
            ..Default::default()
        },
        report: serde_json::Value::Null,
        created_at: now,
        updated_at: now,
    };

    migration.report = serde_json::json!({ "dry_run": report });
    insert_migration(&state.pool, &migration).await?;

    let dry_run_report: DryRunReport = serde_json::from_value(
        migration
            .report
            .get("dry_run")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| AppError::Internal(format!("Failed to read dry-run: {}", e)))?;

    Ok((
        StatusCode::CREATED,
        Json(PlanResponse {
            migration,
            dry_run: dry_run_report,
        }),
    ))
}

pub async fn dry_run_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<DryRunReport>, AppError> {
    let _project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let mut migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;
    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;

    let plan = parse_plan(&migration)?;
    let target_check = match build_backend(&conn, None) {
        Ok(backend) => backend.test_connection().await,
        Err(e) => Err(relating_migrate::MigrateError::Connection(e.to_string())),
    };
    let mut report = dry_run(plan.clone(), &target_check);
    report
        .blockers
        .extend(vector_blockers_for_project(&state.pool, &migration.project_id).await?);
    // Re-check every deterministic source blocker (property shapes, merge
    // keys, tier capacity) so a re-drive after data changes stays honest.
    let graph_path = state.project_graph_path(&migration.project_id);
    report.blockers.extend(source_blockers_for_dry_run(
        state.graph.as_ref(),
        &graph_path,
        &plan,
    ));

    // A dry run performs zero remote writes by construction.
    migration.report = serde_json::json!({ "dry_run": report });
    update_migration(&state.pool, &migration).await?;

    let out: DryRunReport = serde_json::from_value(
        migration
            .report
            .get("dry_run")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| AppError::Internal(format!("Failed to read dry-run: {}", e)))?;
    Ok(Json(out))
}

pub async fn approve_migration_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Path((id_str, mid_str)): Path<(String, String)>,
    Json(payload): Json<ApproveRequest>,
) -> Result<Json<Migration>, AppError> {
    let _project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let mut migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;

    if !payload.approved {
        migration.status = MigrationStatus::Planned;
        migration.progress.approved_by = None;
        migration.progress.message = Some("Approval revoked by user".to_string());
        update_migration(&state.pool, &migration).await?;
        return Ok(Json(migration));
    }

    // Second-actor rule for SSO users: the planner cannot approve their own
    // migration. Break-glass (api-token) and open (anonymous) modes are
    // exempt — there is no second identity to compare against.
    if let Actor::User { email, .. } = &actor.actor {
        if migration.progress.created_by.as_deref() == Some(format!("user:{}", email).as_str()) {
            return Err(AppError::BadRequest(
                "Approvals need a second actor: the planner cannot approve their own migration"
                    .to_string(),
            ));
        }
    }

    let blockers = migration
        .report
        .get("dry_run")
        .and_then(|d| d.get("blockers"))
        .and_then(|b| b.as_array())
        .cloned()
        .unwrap_or_default();
    if !blockers.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Cannot approve: dry-run reports {} blocker(s). Resolve them and re-run the dry-run first.",
            blockers.len()
        )));
    }

    migration.status = MigrationStatus::Approved;
    migration.progress.approved_by = Some(actor.actor.display());
    migration.progress.message = Some(format!(
        "Approved by {}; ready to execute",
        actor.actor.display()
    ));
    crate::routes::audit::audit_event(
        &state.pool,
        &actor.actor.display(),
        "migration.approved",
        &format!("/api/v1/projects/{}/migrations/{}", id_str, mid),
        "ok",
        "",
    )
    .await;
    update_migration(&state.pool, &migration).await?;
    Ok(Json(migration))
}

/// Stamp optimistic-concurrency fields: preserve the source version when it
/// carries one, otherwise start at 1 with the current timestamp. The target
/// MERGE only overwrites when the incoming version is newer.
fn stamp_version(
    bolt_props: &mut BTreeMap<String, BoltValue>,
    source_props: &serde_json::Map<String, serde_json::Value>,
) {
    if !bolt_props.contains_key(VERSION_PROPERTY) {
        let version = source_props
            .get(VERSION_PROPERTY)
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .max(1);
        bolt_props.insert(VERSION_PROPERTY.to_string(), BoltValue::Int(version));
    }
    if !bolt_props.contains_key(UPDATED_AT_PROPERTY) {
        let now = Utc::now().to_rfc3339();
        let stamp = source_props
            .get(UPDATED_AT_PROPERTY)
            .and_then(|v| v.as_str())
            .unwrap_or(&now)
            .to_string();
        bolt_props.insert(UPDATED_AT_PROPERTY.to_string(), BoltValue::String(stamp));
    }
}

fn node_rows_from_page(
    rows: &[HashMap<String, serde_json::Value>],
) -> (Vec<BTreeMap<String, BoltValue>>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for row in rows {
        let node = match row.get("node") {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let props = match node.get("properties") {
            Some(serde_json::Value::Object(map)) => map,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let uid = match props.get("uid").and_then(|v| v.as_str()) {
            Some(uid) => uid.to_string(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let mut bolt_props = BTreeMap::new();
        for (k, v) in props {
            // The migration tag is stamped by ON CREATE, never copied from
            // user data: a same-named user property must not forge it.
            if k != MIGRATION_TAG_PROPERTY {
                bolt_props.insert(k.clone(), json_to_bolt(v));
            }
        }
        stamp_version(&mut bolt_props, props);
        let mut bolt_row = BTreeMap::new();
        bolt_row.insert("uid".to_string(), BoltValue::String(uid));
        bolt_row.insert("props".to_string(), BoltValue::Map(bolt_props));
        out.push(bolt_row);
    }
    (out, skipped)
}

fn rel_rows_from_page(
    rel_type: &str,
    rows: &[HashMap<String, serde_json::Value>],
) -> (Vec<BTreeMap<String, BoltValue>>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for row in rows {
        let suid = row.get("suid").and_then(|v| v.as_str());
        let tuid = row.get("tuid").and_then(|v| v.as_str());
        let rel = row.get("rel");
        let props = rel.and_then(|r| r.get("properties"));
        match (suid, tuid, props) {
            (Some(s), Some(t), Some(serde_json::Value::Object(map))) => {
                // Stable per-edge key: the source record id when present
                // (delta pushes update in place), else a content hash so
                // parallel edges between the same endpoints stay distinct.
                let source_id = rel
                    .and_then(|r| r.get("id"))
                    .map(|id| id.to_string())
                    .filter(|id| !id.is_empty() && id != "null");
                let uid = relating_migrate::relationship_uid(
                    rel_type,
                    s,
                    t,
                    source_id.as_deref(),
                    &serde_json::Value::Object(map.clone()),
                );
                let mut bolt_props = BTreeMap::new();
                for (k, v) in map {
                    if k != MIGRATION_TAG_PROPERTY {
                        bolt_props.insert(k.clone(), json_to_bolt(v));
                    }
                }
                stamp_version(&mut bolt_props, map);
                let mut bolt_row = BTreeMap::new();
                bolt_row.insert("uid".to_string(), BoltValue::String(uid));
                bolt_row.insert("suid".to_string(), BoltValue::String(s.to_string()));
                bolt_row.insert("tuid".to_string(), BoltValue::String(t.to_string()));
                bolt_row.insert("props".to_string(), BoltValue::Map(bolt_props));
                out.push(bolt_row);
            }
            _ => skipped += 1,
        }
    }
    (out, skipped)
}

pub async fn execute_migration_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<Migration>, AppError> {
    let project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let mut migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;

    match migration.status {
        MigrationStatus::Approved => {
            if migration.progress.sync_mode.is_none() {
                migration.progress.sync_mode = Some("full".to_string());
            }
        }
        MigrationStatus::Failed | MigrationStatus::Paused => {
            // Resume: completed batches are skipped below.
        }
        MigrationStatus::Completed | MigrationStatus::Verified => {
            // Delta sync: re-scan the source for new/changed records.
            // Versioned MERGE makes re-transfer idempotent; only newer
            // versions overwrite. Batches restart, totals stay cumulative.
            migration.progress.completed_batches.clear();
            migration.progress.batches_done = 0;
            migration.progress.sync_mode = Some("delta".to_string());
            migration.progress.message =
                Some("Delta sync: re-scanning source for changes".to_string());
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "Migration must be approved before execution (current status: {:?})",
                other
            )));
        }
    }

    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;
    let backend = build_backend(&conn, None)?;
    // Fail fast before any remote write when credentials are wrong.
    backend.test_connection().await?;

    let plan = parse_plan(&migration)?;
    // Rollback tag: everything this execution creates is stamped with the
    // migration id (ON CREATE only), so rollback can remove exactly it.
    let mid_tag = mid.to_string();
    migration.status = MigrationStatus::Running;
    migration.progress.batches_total = plan.total_batches;
    migration.progress.executed_by = Some(actor.actor.display());
    crate::routes::audit::audit_event(
        &state.pool,
        &actor.actor.display(),
        "migration.executed",
        &format!("/api/v1/projects/{}/migrations/{}", id_str, mid),
        "ok",
        "",
    )
    .await;
    update_migration(&state.pool, &migration).await?;

    // 1. Idempotent DDL first.
    for ddl in &plan.ddl_statements {
        if let Err(e) = backend.run_write(ddl, BTreeMap::new()).await {
            migration.status = MigrationStatus::Failed;
            migration.progress.message = Some(format!("DDL failed: {}", e));
            update_migration(&state.pool, &migration).await?;
            return Err(AppError::Migrate(e));
        }
    }

    // 2. Node batches, resumable by completed batch keys.
    let graph_path = state.project_graph_path(&project_id);
    for batch in &plan.node_batches {
        for chunk_idx in 0..batch.batch_count {
            let key = format!("nodes:{}:{}", batch.name, chunk_idx);
            if migration.progress.completed_batches.contains(&key) {
                continue;
            }
            // skip/limit are internally computed integers, never user input.
            let query =
                build_node_page_query(&batch.name, chunk_idx * batch.batch_size, batch.batch_size);
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let (rows, skipped) = node_rows_from_page(&page.rows);
            match migrate_node_batch_with_options(&backend, &batch.name, rows, Some(&mid_tag)).await
            {
                Ok(outcome) => {
                    migration.progress.nodes_migrated += outcome.nodes_created.max(0) as u64;
                    migration.progress.batches_done += 1;
                    migration.progress.completed_batches.push(key.clone());
                    migration.progress.high_water_mark = Some(key);
                    migration.progress.last_sync = Some(Utc::now());
                    if skipped > 0 {
                        migration.progress.message = Some(format!(
                            "Skipped {} node(s) without uid merge keys",
                            skipped
                        ));
                    }
                    update_migration(&state.pool, &migration).await?;
                }
                Err(e) => {
                    migration.status = MigrationStatus::Failed;
                    migration.progress.message = Some(format!("Node batch failed: {}", e));
                    update_migration(&state.pool, &migration).await?;
                    return Err(AppError::Migrate(e));
                }
            }
        }
    }

    // 3. Relationship batches (endpoints were migrated in step 2).
    let mut unstable_edges = 0usize;
    for batch in &plan.rel_batches {
        for chunk_idx in 0..batch.batch_count {
            let key = format!("rels:{}:{}", batch.name, chunk_idx);
            if migration.progress.completed_batches.contains(&key) {
                continue;
            }
            let query =
                build_rel_page_query(&batch.name, chunk_idx * batch.batch_size, batch.batch_size);
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let (rows, _skipped) = rel_rows_from_page(&batch.name, &page.rows);
            unstable_edges += rows
                .iter()
                .filter_map(|r| match r.get("uid") {
                    Some(BoltValue::String(uid)) if is_hash_fallback_uid(uid) => Some(()),
                    _ => None,
                })
                .count();
            match migrate_relationship_batch_with_options(
                &backend,
                &batch.name,
                rows,
                Some(&mid_tag),
            )
            .await
            {
                Ok(outcome) => {
                    migration.progress.relationships_migrated +=
                        outcome.relationships_created.max(0) as u64;
                    migration.progress.batches_done += 1;
                    migration.progress.completed_batches.push(key.clone());
                    migration.progress.high_water_mark = Some(key);
                    migration.progress.last_sync = Some(Utc::now());
                    update_migration(&state.pool, &migration).await?;
                }
                Err(e) => {
                    migration.status = MigrationStatus::Failed;
                    migration.progress.message = Some(format!("Relationship batch failed: {}", e));
                    update_migration(&state.pool, &migration).await?;
                    return Err(AppError::Migrate(e));
                }
            }
        }
    }

    migration.status = MigrationStatus::Completed;
    migration.progress.last_sync = Some(Utc::now());
    let mode = migration
        .progress
        .sync_mode
        .clone()
        .unwrap_or_else(|| "full".to_string());
    migration.progress.message = Some(format!(
        "Migrated {} node(s) and {} relationship(s) in {} batch(es) [mode: {}{}]",
        migration.progress.nodes_migrated,
        migration.progress.relationships_migrated,
        migration.progress.batches_done,
        mode,
        if unstable_edges > 0 {
            format!(
                "; {} edge(s) use hash-fallback uids and will duplicate on property change — add a source id or stable key",
                unstable_edges
            )
        } else {
            String::new()
        }
    ));
    update_migration(&state.pool, &migration).await?;
    Ok(Json(migration))
}

pub async fn verify_migration_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<VerificationReport>, AppError> {
    let project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let mut migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;

    if migration.status != MigrationStatus::Completed
        && migration.status != MigrationStatus::Verified
    {
        return Err(AppError::BadRequest(
            "Migration must complete execution before verification".to_string(),
        ));
    }

    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;
    let backend = build_backend(&conn, None)?;

    let graph_path = state.project_graph_path(&project_id);
    let label_counts = state
        .graph
        .get_label_counts(&graph_path)
        .unwrap_or_default();
    let rel_counts = state
        .graph
        .get_rel_type_counts(&graph_path)
        .unwrap_or_default();

    // Per-label content hashes over the sorted source uids. Labels over the
    // hashing cap skip hashing explicitly (counts still verify).
    let mut label_hashes = Vec::new();
    let mut hash_skipped_labels = Vec::new();
    for (label, count) in &label_counts {
        if *count > MAX_HASH_UIDS_PER_LABEL {
            hash_skipped_labels.push(label.clone());
            continue;
        }
        let query = build_uid_list_query(label);
        let page = state
            .graph
            .execute_read(&graph_path, &query, HashMap::new(), None)?;
        let uids: Vec<String> = page
            .rows
            .iter()
            .filter_map(|row| row.get("uid").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        label_hashes.push((label.clone(), uid_list_hash(&uids)));
    }

    // Relationship-degree expectations: outgoing edge counts per
    // (label, relationship type) pair. Schema-bounded; capped so a huge
    // schema cannot turn verification into a query flood.
    const MAX_DEGREE_PAIRS: usize = 500;
    let mut degrees = Vec::new();
    'pairs: for (label, _) in &label_counts {
        for (rel_type, _) in &rel_counts {
            if degrees.len() >= MAX_DEGREE_PAIRS {
                break 'pairs;
            }
            let query = format!(
                "MATCH (n:{})-[r:{}]->() RETURN count(r) AS c",
                escape_cypher_label(label),
                escape_cypher_label(rel_type)
            );
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let out_count = page
                .rows
                .first()
                .and_then(|row| row.values().next())
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            degrees.push(DegreeExpectation {
                label: label.clone(),
                rel_type: rel_type.clone(),
                out_count,
            });
        }
    }

    let source = SourceCounts {
        labels: label_counts,
        rel_types: rel_counts,
        label_hashes,
        hash_skipped_labels,
        degrees,
        allow_target_superset: migration.progress.sync_mode.as_deref() == Some("delta"),
    };

    // Sample up to 20 migrated nodes for property comparison.
    let mut samples = Vec::new();
    if let Ok(page) = state.graph.execute_read(
        &graph_path,
        "MATCH (n) WHERE n.uid IS NOT NULL RETURN n AS node LIMIT 20",
        HashMap::new(),
        None,
    ) {
        for row in page.rows.iter().take(20) {
            if let Some(node) = row.get("node") {
                let label = node
                    .get("labels")
                    .and_then(|l| l.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("Entity")
                    .to_string();
                if let Some(props) = node.get("properties") {
                    if let Some(uid) = props.get("uid").and_then(|v| v.as_str()) {
                        samples.push(SampleExpectation {
                            label,
                            uid: uid.to_string(),
                            expected_props: props.clone(),
                        });
                    }
                }
            }
        }
    }

    let report = verify_migration(&backend, &source, &samples).await?;
    migration.report = serde_json::json!({ "verification": report });
    if report.passed {
        migration.status = MigrationStatus::Verified;
    }
    update_migration(&state.pool, &migration).await?;

    let out: VerificationReport = serde_json::from_value(
        migration
            .report
            .get("verification")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| AppError::Internal(format!("Failed to read verification: {}", e)))?;
    Ok(Json(out))
}

pub async fn rollback_migration_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<RollbackOutcome>, AppError> {
    let _project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let mut migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;

    match migration.status {
        MigrationStatus::Completed | MigrationStatus::Verified | MigrationStatus::Failed => {}
        other => {
            return Err(AppError::BadRequest(format!(
                "Only completed, verified, or failed migrations can be rolled back (current status: {:?})",
                other
            )));
        }
    }

    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;
    let backend = build_backend(&conn, None)?;
    // Fail fast before any remote delete when credentials are wrong.
    backend.test_connection().await?;

    let outcome = rollback_migration(&backend, &mid.to_string()).await?;
    migration.status = MigrationStatus::RolledBack;
    migration.progress.message = Some(format!(
        "Rolled back migration {}: deleted {} node(s) and {} relationship(s) it created. \
         Property updates on pre-existing target records were not reverted.",
        mid, outcome.nodes_deleted, outcome.relationships_deleted
    ));
    update_migration(&state.pool, &migration).await?;
    Ok(Json(outcome))
}

pub async fn list_migrations_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<Migration>>, AppError> {
    let project_id = parse_project(&id_str)?;
    let migrations = db_list_migrations(&state.pool, &project_id).await?;
    Ok(Json(migrations))
}

pub async fn get_migration_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<Migration>, AppError> {
    let _project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;
    Ok(Json(migration))
}

pub async fn get_migration_report_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;

    let migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;

    Ok(Json(serde_json::json!({
        "migration_id": migration.id.to_string(),
        "project_id": migration.project_id.to_string(),
        "connection_id": migration.connection_id.to_string(),
        "status": migration.status,
        "plan": migration.plan,
        "progress": migration.progress,
        "report": migration.report,
        "created_at": migration.created_at,
        "updated_at": migration.updated_at,
    })))
}

#[derive(Deserialize, Default)]
pub struct SyncQuery {
    #[serde(default)]
    pub propagate_deletes: bool,
}

/// Continuous sync: delta re-execute of a completed migration plus optional
/// explicit delete propagation. Merge-upsert never deletes on its own, so
/// source deletes only reach the target when `propagate_deletes=true`.
pub async fn sync_migration_handler(
    State(state): State<AppState>,
    Extension(actor): Extension<ActorContext>,
    Path((id_str, mid_str)): Path<(String, String)>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<Migration>, AppError> {
    let executed = execute_migration_handler(
        State(state.clone()),
        Extension(actor),
        Path((id_str.clone(), mid_str.clone())),
    )
    .await?;
    if !query.propagate_deletes {
        return Ok(executed);
    }
    let mut migration = executed.0;
    let project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;
    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;
    let backend = build_backend(&conn, None)?;
    let plan = parse_plan(&migration)?;
    let graph_path = state.project_graph_path(&project_id);
    let mid_tag = mid.to_string();
    let mut deleted_nodes = 0i64;
    let mut deleted_rels = 0i64;
    for batch in &plan.node_batches {
        let query = build_uid_list_query(&batch.name);
        let page = state
            .graph
            .execute_read(&graph_path, &query, HashMap::new(), None)?;
        let source_uids: Vec<String> = page
            .rows
            .iter()
            .filter_map(|row| row.get("uid").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        let outcome = propagate_node_deletes(&backend, &mid_tag, &batch.name, &source_uids).await?;
        deleted_nodes += outcome.nodes_deleted;
    }
    for batch in &plan.rel_batches {
        let mut source_uids = Vec::new();
        for chunk_idx in 0..batch.batch_count {
            let query =
                build_rel_page_query(&batch.name, chunk_idx * batch.batch_size, batch.batch_size);
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let (rows, _) = rel_rows_from_page(&batch.name, &page.rows);
            for row in rows {
                if let Some(BoltValue::String(uid)) = row.get("uid") {
                    source_uids.push(uid.clone());
                }
            }
        }
        let outcome = propagate_rel_deletes(&backend, &mid_tag, &batch.name, &source_uids).await?;
        deleted_rels += outcome.relationships_deleted;
    }
    let prior = migration.progress.message.clone().unwrap_or_default();
    migration.progress.message = Some(format!(
        "{} | delete propagation removed {} node(s) and {} relationship(s)",
        prior, deleted_nodes, deleted_rels
    ));
    update_migration(&state.pool, &migration).await?;
    Ok(Json(migration))
}

/// Scheduled drift detector: runs the same count/hash/degree checks as
/// verify without changing migration status. Pair with the shadow
/// `/shadow/runs` endpoint for query-level diffs.
pub async fn drift_migration_handler(
    State(state): State<AppState>,
    Path((id_str, mid_str)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let project_id = parse_project(&id_str)?;
    let mid = MigrationId::parse(&mid_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Migration UUID '{}': {}", mid_str, e))
    })?;
    let migration = db_get_migration(&state.pool, &mid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Migration '{}' not found", mid)))?;
    match migration.status {
        MigrationStatus::Completed | MigrationStatus::Verified => {}
        other => {
            return Err(AppError::BadRequest(format!(
                "Drift detection needs a completed migration (current status: {:?})",
                other
            )));
        }
    }
    let conn = get_connection(&state.pool, &migration.project_id, &migration.connection_id)
        .await
        .map_err(|_| AppError::NotFound("Migration connection no longer exists".to_string()))?;
    let backend = build_backend(&conn, None)?;
    let graph_path = state.project_graph_path(&project_id);
    let label_counts = state
        .graph
        .get_label_counts(&graph_path)
        .unwrap_or_default();
    let rel_counts = state
        .graph
        .get_rel_type_counts(&graph_path)
        .unwrap_or_default();
    let mut label_hashes = Vec::new();
    let mut hash_skipped_labels = Vec::new();
    for (label, count) in &label_counts {
        if *count > MAX_HASH_UIDS_PER_LABEL {
            hash_skipped_labels.push(label.clone());
            continue;
        }
        let query = build_uid_list_query(label);
        let page = state
            .graph
            .execute_read(&graph_path, &query, HashMap::new(), None)?;
        let uids: Vec<String> = page
            .rows
            .iter()
            .filter_map(|row| row.get("uid").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        label_hashes.push((label.clone(), uid_list_hash(&uids)));
    }
    let source = SourceCounts {
        labels: label_counts,
        rel_types: rel_counts,
        label_hashes,
        hash_skipped_labels,
        degrees: Vec::new(),
        allow_target_superset: false,
    };
    let report = verify_migration(&backend, &source, &[]).await?;
    Ok(Json(serde_json::json!({
        "migration_id": mid.to_string(),
        "drifted": !report.passed,
        "checked_at": Utc::now().to_rfc3339(),
        "report": report,
    })))
}

#[derive(Deserialize, Default)]
pub struct ChangesQuery {
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// CDC change feed: nodes changed since a watermark (RFC 3339 `_updated_at`).
/// Backs incremental-sync schedulers; `since` omitted returns the oldest page.
pub async fn changes_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    let graph_path = state.project_graph_path(&project_id);
    let page = state.graph.changes_since(
        &graph_path,
        query.since.as_deref(),
        query.limit.or(Some(100)),
    )?;
    Ok(Json(serde_json::json!({
        "project_id": project_id.to_string(),
        "since": query.since,
        "row_count": page.row_count,
        "rows": page.rows,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stable_scan_queries_order_deterministically() {
        let node = build_node_page_query("Person", 10, 5);
        assert!(node.contains("ORDER BY n.uid"), "{}", node);
        assert!(node.contains("SKIP 10 LIMIT 5"), "{}", node);
        let rel = build_rel_page_query("KNOWS", 0, 100);
        assert!(rel.contains("ORDER BY s.uid, t.uid"), "{}", rel);
        let uids = build_uid_list_query("Person");
        assert!(uids.contains("ORDER BY uid"), "{}", uids);
    }

    #[test]
    fn test_stamp_version_preserves_source_and_defaults() {
        let mut props = BTreeMap::new();
        let source: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        stamp_version(&mut props, &source);
        assert_eq!(props.get(VERSION_PROPERTY), Some(&BoltValue::Int(1)));
        assert!(props.contains_key(UPDATED_AT_PROPERTY));

        let mut keep = BTreeMap::new();
        let mut src = serde_json::Map::new();
        src.insert(VERSION_PROPERTY.to_string(), serde_json::json!(7));
        src.insert(
            UPDATED_AT_PROPERTY.to_string(),
            serde_json::json!("2026-01-01T00:00:00Z"),
        );
        stamp_version(&mut keep, &src);
        assert_eq!(keep.get(VERSION_PROPERTY), Some(&BoltValue::Int(7)));
    }

    #[test]
    fn test_node_rows_stamp_version_and_filter_tag() {
        let mut inner = serde_json::Map::new();
        inner.insert("uid".to_string(), serde_json::json!("Person:1"));
        inner.insert(
            MIGRATION_TAG_PROPERTY.to_string(),
            serde_json::json!("forged"),
        );
        let mut row = HashMap::new();
        row.insert("node".to_string(), serde_json::json!({"properties": inner}));
        let (rows, skipped) = node_rows_from_page(&[row]);
        assert_eq!(skipped, 0);
        let props = match rows[0].get("props") {
            Some(BoltValue::Map(m)) => m,
            other => panic!("expected props map, got {:?}", other),
        };
        assert!(!props.contains_key(MIGRATION_TAG_PROPERTY));
        assert_eq!(props.get(VERSION_PROPERTY), Some(&BoltValue::Int(1)));
        assert!(props.contains_key(UPDATED_AT_PROPERTY));
    }
}
