use crate::aura_tiers::{
    capacity_blockers, check_capacity_fit, check_vector_fit, parse_tier, vector_dimension_blockers,
    AuraTier, VectorWorkload,
};
use crate::backend::{BoltBackend, Neo4jBackend, TargetInfo};
use crate::bolt::{BoltConfig, BoltUri};
use crate::compat::{analyze_source, analyze_target, CapabilityVerdict, CompatFinding};
use crate::error::MigrateError;
use crate::plan::{build_plan, dry_run, DryRunReport, MigrationPlan};
use relating_core::{AuraConnection, ProjectId};
use relating_graph::{GraphBackend, GraphSchemaInfo};
use relating_metadata::{
    count_chunks_for_profile, get_approved_ontology, list_embedding_profiles, DbPool,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
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

fn auth_findings(connection_name: &str, reason: String) -> Vec<CompatFinding> {
    vec![CompatFinding {
        category: "target_connection".to_string(),
        subject: connection_name.to_string(),
        verdict: crate::compat::CapabilityVerdict::RequiresManualReview,
        detail: format!("Target rejected the credentials: {}", reason),
        remediation: Some(
            "The instance is reachable, but the username/password failed. Use the DBMS user (usually 'neo4j') with the instance password from the Aura console, update AURA_USERNAME/AURA_PASSWORD, then re-run analysis"
                .to_string(),
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

    // Property shapes Neo4j cannot store, sampled from the source graph:
    // nested maps surface as TRANSFORM, mixed-type lists as UNSUPPORTED.
    let shapes = scan_property_shapes(graph, graph_path, PROPERTY_SCAN_SAMPLE_LIMIT);
    source_findings.extend(property_shape_findings(&shapes));

    // Tier capacity pre-flight: source size vs the tier's published caps.
    if let Some(tier) = tier.as_ref() {
        source_findings.extend(check_capacity_fit(
            schema.node_count,
            schema.relationship_count,
            tier,
        ));
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
            // Authentication failures are credential problems on a reachable
            // target — never label them "unreachable".
            Err(MigrateError::Auth(reason)) => {
                target_findings = auth_findings(&conn.name, reason);
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
    preview_migration_with_options(
        pool, graph, project_id, graph_path, connection, batch_size, None,
    )
    .await
}

/// [`preview_migration`] with an optional target Aura tier for capacity
/// pre-flight. The tier spec is stored on the plan so later dry-runs can
/// re-check capacity deterministically.
pub async fn preview_migration_with_options(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    project_id: &ProjectId,
    graph_path: &Path,
    connection: &AuraConnection,
    batch_size: usize,
    aura_tier: Option<String>,
) -> Result<(MigrationPlan, DryRunReport), MigrateError> {
    let label_counts = graph.get_label_counts(graph_path).unwrap_or_default();
    let rel_counts = graph.get_rel_type_counts(graph_path).unwrap_or_default();
    let approved = get_approved_ontology(pool, project_id).await?;

    let target_check = test_target(connection, None).await;
    let target_agent = target_check.as_ref().ok().map(|i| i.server_agent.clone());

    let mut plan = build_plan(
        &label_counts,
        &rel_counts,
        approved.as_ref().map(|o| &o.schema),
        target_agent.as_deref(),
        batch_size,
    )?;

    // Merge-key coverage: nodes without a uid are skipped at execute time.
    let uidless = uidless_node_count(graph, graph_path);
    if uidless > 0 {
        if plan.total_nodes > 0 && uidless >= plan.total_nodes {
            // Blocker appended to the report below.
        } else {
            plan.warnings.push(format!(
                "{} of {} node(s) have no uid merge key and will be skipped at execute time; backfill uid before migrating",
                uidless, plan.total_nodes
            ));
        }
    }

    // Property shapes Neo4j cannot store: nested maps warn (TRANSFORM),
    // heterogeneous lists block (UNSUPPORTED).
    let shapes = scan_property_shapes(graph, graph_path, PROPERTY_SCAN_SAMPLE_LIMIT);
    for finding in property_shape_findings(&shapes) {
        if finding.verdict == CapabilityVerdict::Transform {
            plan.warnings
                .push(format!("{}: {}", finding.subject, finding.detail));
        }
    }

    // Tier capacity pre-flight, when the caller names a tier.
    let tier = aura_tier
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_tier)
        .transpose()?;
    if let Some(tier) = tier.as_ref() {
        plan.aura_tier = Some(tier.id.clone());
        for finding in check_capacity_fit(plan.total_nodes, plan.total_relationships, tier) {
            if finding.verdict == CapabilityVerdict::RequiresManualReview {
                plan.warnings
                    .push(format!("{}: {}", finding.subject, finding.detail));
            }
        }
    }

    let mut report = dry_run(plan.clone(), &target_check);
    if plan.total_nodes > 0 && uidless >= plan.total_nodes {
        report.blockers.push(
            "No node carries a uid merge key: every node batch would be skipped. Backfill uid (or re-ingest) before migrating"
                .to_string(),
        );
    }
    report.blockers.extend(property_shape_blockers(&shapes));
    if let Some(tier) = tier.as_ref() {
        report.blockers.extend(capacity_blockers(
            plan.total_nodes,
            plan.total_relationships,
            tier,
        ));
    }
    Ok((plan, report))
}

/// Deterministic dry-run blockers that need the source graph: property
/// shapes Neo4j cannot store, missing merge keys, and tier capacity.
/// The dry-run handler re-runs this on every call so re-drives stay honest.
pub fn source_blockers_for_dry_run(
    graph: &dyn GraphBackend,
    graph_path: &Path,
    plan: &MigrationPlan,
) -> Vec<String> {
    let mut blockers = Vec::new();
    let shapes = scan_property_shapes(graph, graph_path, PROPERTY_SCAN_SAMPLE_LIMIT);
    blockers.extend(property_shape_blockers(&shapes));
    let uidless = uidless_node_count(graph, graph_path);
    if plan.total_nodes > 0 && uidless >= plan.total_nodes {
        blockers.push(
            "No node carries a uid merge key: every node batch would be skipped. Backfill uid (or re-ingest) before migrating"
                .to_string(),
        );
    }
    if let Some(spec) = plan.aura_tier.as_deref() {
        if let Ok(tier) = parse_tier(spec) {
            blockers.extend(capacity_blockers(
                plan.total_nodes,
                plan.total_relationships,
                &tier,
            ));
        }
    }
    blockers
}

fn count_from_rows(rows: &[HashMap<String, serde_json::Value>]) -> u64 {
    rows.first()
        .and_then(|row| row.values().next())
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Nodes without a uid merge key. Best effort: an unreadable store reports
/// zero here and surfaces through the empty-source blocker instead.
pub fn uidless_node_count(graph: &dyn GraphBackend, graph_path: &Path) -> u64 {
    graph
        .execute_read(
            graph_path,
            "MATCH (n) WHERE n.uid IS NULL RETURN count(n) AS c",
            HashMap::new(),
            None,
        )
        .map(|page| count_from_rows(&page.rows))
        .unwrap_or(0)
}

/// Sample size for the property-shape pre-flight scan.
pub const PROPERTY_SCAN_SAMPLE_LIMIT: usize = 200;
/// Cap per shape class so one messy label cannot flood the findings.
pub const PROPERTY_SHAPE_CAP: usize = 25;

/// Property shapes sampled from the source that Neo4j cannot store as-is.
#[derive(Debug, Clone, Default)]
pub struct PropertyShapeReport {
    pub nodes_scanned: usize,
    pub rels_scanned: usize,
    /// `Label.prop` entries holding a nested map (or a list containing one).
    pub nested_maps: Vec<String>,
    /// `Label.prop` entries holding a mixed-type list.
    pub heterogeneous_lists: Vec<String>,
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        // Integers and floats share one kind: JSON round-trips blur 1 vs 1.0
        // and flagging every mixed int/float list would be a false positive.
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "list",
        serde_json::Value::Object(_) => "map",
    }
}

fn classify_properties(
    owner: &str,
    props: &serde_json::Map<String, serde_json::Value>,
    nested: &mut BTreeSet<String>,
    hetero: &mut BTreeSet<String>,
) {
    for (name, value) in props {
        let subject = format!("{}.{}", owner, name);
        match value {
            serde_json::Value::Object(_) => {
                nested.insert(subject);
            }
            serde_json::Value::Array(items) => {
                if items.iter().any(|item| item.is_object() || item.is_array()) {
                    nested.insert(subject.clone());
                }
                let mut kinds: BTreeSet<&str> = items.iter().map(json_kind).collect();
                kinds.remove("null");
                kinds.remove("list");
                kinds.remove("map");
                if kinds.len() > 1 {
                    hetero.insert(subject);
                }
            }
            _ => {}
        }
    }
}

/// Sample source properties for shapes Neo4j cannot store: nested maps
/// (TRANSFORM: flatten or JSON-encode) and heterogeneous lists
/// (UNSUPPORTED: Neo4j lists must hold a single type).
pub fn scan_property_shapes(
    graph: &dyn GraphBackend,
    graph_path: &Path,
    sample_limit: usize,
) -> PropertyShapeReport {
    let mut nested = BTreeSet::new();
    let mut hetero = BTreeSet::new();
    let mut nodes_scanned = 0usize;
    let mut rels_scanned = 0usize;

    let node_query = format!("MATCH (n) RETURN n AS node LIMIT {}", sample_limit.max(1));
    if let Ok(page) = graph.execute_read(graph_path, &node_query, HashMap::new(), None) {
        for row in &page.rows {
            let Some(node) = row.get("node") else {
                continue;
            };
            nodes_scanned += 1;
            let label = node
                .get("labels")
                .and_then(|l| l.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("Entity");
            if let Some(props) = node.get("properties").and_then(|p| p.as_object()) {
                classify_properties(label, props, &mut nested, &mut hetero);
            }
        }
    }

    // Proven typed pattern (the same shape execute uses), per rel type.
    let rel_types = graph.get_rel_type_counts(graph_path).unwrap_or_default();
    let per_type = sample_limit.div_ceil(rel_types.len().max(1)).clamp(1, 50);
    for (rel_type, _) in &rel_types {
        let rel_query = format!(
            "MATCH ()-[r:`{}`]->() RETURN r AS rel LIMIT {}",
            rel_type.replace('`', "``"),
            per_type
        );
        let Ok(page) = graph.execute_read(graph_path, &rel_query, HashMap::new(), None) else {
            continue;
        };
        for row in &page.rows {
            let Some(rel) = row.get("rel") else { continue };
            rels_scanned += 1;
            if let Some(props) = rel.get("properties").and_then(|p| p.as_object()) {
                classify_properties(
                    &format!("{}-rel", rel_type),
                    props,
                    &mut nested,
                    &mut hetero,
                );
            }
        }
    }

    PropertyShapeReport {
        nodes_scanned,
        rels_scanned,
        nested_maps: nested.into_iter().take(PROPERTY_SHAPE_CAP).collect(),
        heterogeneous_lists: hetero.into_iter().take(PROPERTY_SHAPE_CAP).collect(),
    }
}

/// Analyzer findings for a shape report: nested maps are TRANSFORM,
/// heterogeneous lists are UNSUPPORTED, a clean scan is Supported.
pub fn property_shape_findings(report: &PropertyShapeReport) -> Vec<CompatFinding> {
    let mut findings = Vec::new();
    for subject in &report.nested_maps {
        findings.push(CompatFinding {
            category: "property_types".to_string(),
            subject: format!("{} (nested map)", subject),
            verdict: CapabilityVerdict::Transform,
            detail: "Neo4j properties cannot store nested maps; flatten to scalar properties or JSON-encode to a string before migrating".to_string(),
            remediation: Some(
                "Flatten the map into dotted scalar properties, or store it as one JSON string".to_string(),
            ),
        });
    }
    for subject in &report.heterogeneous_lists {
        findings.push(CompatFinding {
            category: "property_types".to_string(),
            subject: format!("{} (mixed-type list)", subject),
            verdict: CapabilityVerdict::Unsupported,
            detail: "Neo4j lists must hold a single type; this property mixes types and the write would fail on Aura".to_string(),
            remediation: Some(
                "Normalize the list to one type (or split it into typed properties) before migrating".to_string(),
            ),
        });
    }
    if findings.is_empty() {
        findings.push(CompatFinding {
            category: "property_types".to_string(),
            subject: format!(
                "property scan ({} nodes, {} rels sampled)",
                report.nodes_scanned, report.rels_scanned
            ),
            verdict: CapabilityVerdict::Supported,
            detail: "Sampled properties are Neo4j-storable scalars and homogeneous lists"
                .to_string(),
            remediation: None,
        });
    }
    findings
}

/// Deterministic blockers: heterogeneous lists fail the Aura write, so they
/// gate approval. Nested maps warn (TRANSFORM) instead of blocking.
pub fn property_shape_blockers(report: &PropertyShapeReport) -> Vec<String> {
    report
        .heterogeneous_lists
        .iter()
        .map(|subject| {
            format!(
                "Property '{}' holds a mixed-type list; Neo4j requires homogeneous lists",
                subject
            )
        })
        .collect()
}
