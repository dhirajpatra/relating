use crate::error::AppError;
use crate::routes::aura::{build_backend, get_connection};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_core::{
    AuraConnectionId, Migration, MigrationId, MigrationProgress, MigrationStatus, ProjectId,
};
use relating_graph::GraphBackend;
use relating_metadata::{
    get_migration as db_get_migration, get_project as db_get_project, insert_migration,
    list_migrations as db_list_migrations, update_migration,
};
use relating_migrate::{
    analyze_project_with_options, dry_run, json_to_bolt, migrate_node_batch,
    migrate_relationship_batch, preview_migration, vector_blockers_for_project, verify_migration,
    AnalysisOptions, BoltValue, CompatFinding, DryRunReport, MigrationPlan, Neo4jBackend,
    SampleExpectation, SourceCounts, VerificationReport, DEFAULT_BATCH_SIZE,
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

    let graph_path = state.project_graph_path(&project_id);
    let batch_size = payload
        .batch_size
        .unwrap_or(DEFAULT_BATCH_SIZE)
        .clamp(1, 5000);
    let (plan, mut report) = preview_migration(
        &state.pool,
        state.graph.as_ref(),
        &project_id,
        &graph_path,
        &conn,
        batch_size,
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
    let mut report = dry_run(plan, &target_check);
    report
        .blockers
        .extend(vector_blockers_for_project(&state.pool, &migration.project_id).await?);

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
        migration.progress.message = Some("Approval revoked by user".to_string());
        update_migration(&state.pool, &migration).await?;
        return Ok(Json(migration));
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
    migration.progress.message = Some("Approved by user; ready to execute".to_string());
    update_migration(&state.pool, &migration).await?;
    Ok(Json(migration))
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
            bolt_props.insert(k.clone(), json_to_bolt(v));
        }
        let mut bolt_row = BTreeMap::new();
        bolt_row.insert("uid".to_string(), BoltValue::String(uid));
        bolt_row.insert("props".to_string(), BoltValue::Map(bolt_props));
        out.push(bolt_row);
    }
    (out, skipped)
}

fn rel_rows_from_page(
    rows: &[HashMap<String, serde_json::Value>],
) -> (Vec<BTreeMap<String, BoltValue>>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for row in rows {
        let suid = row.get("suid").and_then(|v| v.as_str());
        let tuid = row.get("tuid").and_then(|v| v.as_str());
        let props = row.get("rel").and_then(|r| r.get("properties"));
        match (suid, tuid, props) {
            (Some(s), Some(t), Some(serde_json::Value::Object(map))) => {
                let mut bolt_props = BTreeMap::new();
                for (k, v) in map {
                    bolt_props.insert(k.clone(), json_to_bolt(v));
                }
                let mut bolt_row = BTreeMap::new();
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
        MigrationStatus::Approved => {}
        MigrationStatus::Failed | MigrationStatus::Paused => {
            // Resume: completed batches are skipped below.
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
    migration.status = MigrationStatus::Running;
    migration.progress.batches_total = plan.total_batches;
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
            let query = format!(
                "MATCH (n:{}) RETURN n AS node SKIP {} LIMIT {}",
                escape_cypher_label(&batch.name),
                chunk_idx * batch.batch_size,
                batch.batch_size
            );
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let (rows, skipped) = node_rows_from_page(&page.rows);
            match migrate_node_batch(&backend, &batch.name, rows).await {
                Ok(outcome) => {
                    migration.progress.nodes_migrated += outcome.nodes_created.max(0) as u64;
                    migration.progress.batches_done += 1;
                    migration.progress.completed_batches.push(key);
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
    for batch in &plan.rel_batches {
        for chunk_idx in 0..batch.batch_count {
            let key = format!("rels:{}:{}", batch.name, chunk_idx);
            if migration.progress.completed_batches.contains(&key) {
                continue;
            }
            let query = format!(
                "MATCH (s)-[r:{}]->(t) RETURN s.uid AS suid, t.uid AS tuid, r AS rel SKIP {} LIMIT {}",
                escape_cypher_label(&batch.name),
                chunk_idx * batch.batch_size,
                batch.batch_size
            );
            let page = state
                .graph
                .execute_read(&graph_path, &query, HashMap::new(), None)?;
            let (rows, _skipped) = rel_rows_from_page(&page.rows);
            match migrate_relationship_batch(&backend, &batch.name, rows).await {
                Ok(outcome) => {
                    migration.progress.relationships_migrated +=
                        outcome.relationships_created.max(0) as u64;
                    migration.progress.batches_done += 1;
                    migration.progress.completed_batches.push(key);
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
    migration.progress.message = Some(format!(
        "Migrated {} node(s) and {} relationship(s) in {} batch(es)",
        migration.progress.nodes_migrated,
        migration.progress.relationships_migrated,
        migration.progress.batches_done
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
    let source = SourceCounts {
        labels: label_counts,
        rel_types: rel_counts,
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
