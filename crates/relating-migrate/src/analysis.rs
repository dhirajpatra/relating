use crate::aura_tiers::{
    check_vector_fit, parse_tier, vector_dimension_blockers, AuraTier, VectorWorkload,
};
use crate::backend::{BoltBackend, Neo4jBackend, TargetInfo};
use crate::bolt::{BoltConfig, BoltUri};
use crate::compat::{analyze_source, analyze_target, CompatFinding};
use crate::error::MigrateError;
use crate::plan::{build_plan, dry_run, DryRunReport, MigrationPlan};
use relating_core::{AuraConnection, ProjectId};
use relating_graph::{GraphBackend, GraphSchemaInfo};
use relating_metadata::{
    count_chunks_for_profile, get_approved_ontology, list_embedding_profiles, DbPool,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub source_findings: Vec<CompatFinding>,
    pub target_findings: Vec<CompatFinding>,
    pub target_reachable: bool,
    pub target_agent: Option<String>,
}

fn unreachable_findings(connection_name: &str, reason: String) -> Vec<CompatFinding> {
    vec![CompatFinding {
        category: "target_connection".to_string(),
        subject: connection_name.to_string(),
        verdict: crate::compat::CapabilityVerdict::RequiresManualReview,
        detail: format!("Target is unreachable: {}", reason),
        remediation: Some(
            "Check the URI, credentials, and network access, then re-run analysis".to_string(),
        ),
    }]
}

fn backend_for(
    conn: &AuraConnection,
    password_override: Option<String>,
) -> Result<BoltBackend, MigrateError> {
    let uri = BoltUri::parse(&conn.uri)?;
    let password = match password_override {
        Some(p) => p,
        None => {
            relating_core::resolve_secret(&conn.auth_env_var).map_err(MigrateError::Connection)?
        }
    };
    let mut config = BoltConfig::new(uri, conn.username.clone(), password);
    config.database = conn.database.clone();
    Ok(BoltBackend::new(config))
}

pub async fn test_target(
    conn: &AuraConnection,
    password_override: Option<String>,
) -> Result<TargetInfo, MigrateError> {
    backend_for(conn, password_override)?
        .test_connection()
        .await
}

/// Optional knobs for [`analyze_project_with_options`]. All fields are optional;
/// defaults reproduce the historical [`analyze_project`] behavior plus
/// tier-independent vector dimension checks.
#[derive(Debug, Clone, Default)]
pub struct AnalysisOptions {
    /// Target Aura tier spec (see [`parse_tier`]). Enables per-tier vector
    /// RAM checks; without it, dimension limits are still checked but RAM
    /// fit is reported as needing a tier selection.
    pub aura_tier: Option<String>,
}

pub async fn analyze_project(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    project_id: &ProjectId,
    graph_path: &Path,
    connection: Option<&AuraConnection>,
) -> Result<AnalysisReport, MigrateError> {
    analyze_project_with_options(
        pool,
        graph,
        project_id,
        graph_path,
        connection,
        AnalysisOptions::default(),
    )
    .await
}

/// Vector workloads for every embedding profile on the project, used for
/// dimension and RAM-fit checks.
pub async fn vector_workloads_for_project(
    pool: &DbPool,
    project_id: &ProjectId,
) -> Result<Vec<VectorWorkload>, MigrateError> {
    let profiles = list_embedding_profiles(pool, project_id).await?;
    let mut workloads = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let count = count_chunks_for_profile(pool, &profile.id).await?;
        workloads.push(VectorWorkload {
            profile_name: profile.name,
            dimensions: profile.dimensions,
            vector_count: count,
        });
    }
    Ok(workloads)
}

/// Deterministic dry-run blockers: embedding dimensions Aura cannot index.
/// Tier-independent, so the approve gate enforces it for every migration.
pub async fn vector_blockers_for_project(
    pool: &DbPool,
    project_id: &ProjectId,
) -> Result<Vec<String>, MigrateError> {
    let workloads = vector_workloads_for_project(pool, project_id).await?;
    Ok(vector_dimension_blockers(&workloads))
}

pub async fn analyze_project_with_options(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    project_id: &ProjectId,
    graph_path: &Path,
    connection: Option<&AuraConnection>,
    options: AnalysisOptions,
) -> Result<AnalysisReport, MigrateError> {
    let schema: GraphSchemaInfo = graph.get_schema(graph_path).unwrap_or_default();
    let approved = get_approved_ontology(pool, project_id).await?;
    let profiles = list_embedding_profiles(pool, project_id).await?;
    let mut source_findings =
        analyze_source(&schema, approved.as_ref().map(|o| &o.schema), &profiles);

    // Vector index fit: dimension limits always, tier RAM fit when a tier is given.
    let tier: Option<AuraTier> = options
        .aura_tier
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_tier)
        .transpose()?;
    if !profiles.is_empty() {
        let workloads = vector_workloads_for_project(pool, project_id).await?;
        source_findings.extend(check_vector_fit(&workloads, tier.as_ref()));
    }

    let mut target_findings = Vec::new();
    let mut target_reachable = false;
    let mut target_agent = None;

    if let Some(conn) = connection {
        match test_target(conn, None).await {
            Ok(info) => {
                target_reachable = true;
                target_agent = Some(info.server_agent.clone());
                target_findings = analyze_target(&info, conn.uri.contains("+s://"));
            }
            Err(e) => {
                target_findings = unreachable_findings(&conn.name, e.to_string());
            }
        }
    }

    Ok(AnalysisReport {
        source_findings,
        target_findings,
        target_reachable,
        target_agent,
    })
}

pub async fn preview_migration(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    project_id: &ProjectId,
    graph_path: &Path,
    connection: &AuraConnection,
    batch_size: usize,
) -> Result<(MigrationPlan, DryRunReport), MigrateError> {
    let label_counts = graph.get_label_counts(graph_path).unwrap_or_default();
    let rel_counts = graph.get_rel_type_counts(graph_path).unwrap_or_default();
    let approved = get_approved_ontology(pool, project_id).await?;

    let target_check = test_target(connection, None).await;
    let target_agent = target_check.as_ref().ok().map(|i| i.server_agent.clone());

    let plan = build_plan(
        &label_counts,
        &rel_counts,
        approved.as_ref().map(|o| &o.schema),
        target_agent.as_deref(),
        batch_size,
    )?;
    let report = dry_run(plan.clone(), &target_check);
    Ok((plan, report))
}
