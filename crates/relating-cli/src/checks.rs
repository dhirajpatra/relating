use crate::error::CliError;
use relating_core::{Project, ProjectId};
use relating_graph::{GraphBackend, RelateBackend};
use relating_metadata::{get_latest_schema_revision, get_project, DbPool};
use relating_migrate::{analyze_project, CapabilityVerdict};
use relating_schema::{build_live_document, check_conformance, SchemaDocument};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct Context {
    pub pool: DbPool,
    pub graph: Arc<RelateBackend>,
    pub project: Project,
    pub project_id: ProjectId,
    pub graph_path: PathBuf,
    pub data_dir: PathBuf,
}

impl Context {
    pub async fn load(
        pool: DbPool,
        data_dir: PathBuf,
        project_id: &ProjectId,
    ) -> Result<Self, CliError> {
        let project = get_project(&pool, project_id)
            .await?
            .ok_or_else(|| CliError::Project(format!("Project '{}' not found", project_id)))?;
        let graph_path = data_dir
            .join("projects")
            .join(project_id.to_string())
            .join("graph.rl");
        Ok(Self {
            pool,
            graph: Arc::new(RelateBackend::new()),
            project,
            project_id: project_id.clone(),
            graph_path,
            data_dir,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub name: String,
    pub passed: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl CheckOutcome {
    pub fn pass(name: &str) -> Self {
        Self {
            name: name.to_string(),
            passed: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn fail(name: &str, errors: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            passed: false,
            errors,
            warnings: Vec::new(),
        }
    }
}

pub fn latest_document(revision_doc: &serde_json::Value) -> Result<SchemaDocument, CliError> {
    let document: SchemaDocument = serde_json::from_value(revision_doc.clone())
        .map_err(|e| CliError::Check(format!("Stored revision is corrupt: {}", e)))?;
    document.validate()?;
    Ok(document)
}

/// Gate 1+3: schema conformance and graph contract tests against the latest
/// revision (or the live document when nothing is versioned yet).
pub async fn check_conformance_gate(ctx: &Context, label: &str) -> Result<CheckOutcome, CliError> {
    let revision = get_latest_schema_revision(&ctx.pool, &ctx.project_id).await?;
    let document = match revision {
        Some(revision) => latest_document(&revision.document)?,
        None => {
            let mut outcome = CheckOutcome::pass(label);
            outcome
                .warnings
                .push("No schema revisions yet; checked live state instead".to_string());
            let live = build_live_document(
                &ctx.pool,
                ctx.graph.as_ref(),
                &ctx.project_id,
                &ctx.graph_path,
                Some(ctx.project.name.clone()),
            )
            .await?;
            let report = check_conformance(ctx.graph.as_ref(), &ctx.graph_path, &live, 200)?;
            if !report.passed {
                return Ok(CheckOutcome::fail(
                    label,
                    report
                        .checks
                        .into_iter()
                        .filter(|c| !c.passed)
                        .map(|c| c.detail)
                        .collect(),
                ));
            }
            return Ok(outcome);
        }
    };

    let report = check_conformance(ctx.graph.as_ref(), &ctx.graph_path, &document, 200)?;
    if report.passed {
        Ok(CheckOutcome::pass(label))
    } else {
        Ok(CheckOutcome::fail(
            label,
            report
                .checks
                .into_iter()
                .filter(|c| !c.passed)
                .map(|c| format!("{}: {}", c.subject, c.detail))
                .collect(),
        ))
    }
}

/// Gate 4: data integrity — uid coverage, duplicate merge keys, orphans.
pub async fn check_integrity(ctx: &Context) -> Result<CheckOutcome, CliError> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Nodes missing uid merge keys break idempotent migration.
    match ctx.graph.execute_read(
        &ctx.graph_path,
        "MATCH (n) WHERE n.uid IS NULL RETURN count(n) AS c",
        HashMap::new(),
        None,
    ) {
        Ok(page) => {
            let missing = page
                .rows
                .first()
                .and_then(|r| r.get("c"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if missing > 0 {
                errors.push(format!(
                    "{} node(s) lack uid merge keys; backfill uid before migrating",
                    missing
                ));
            }
        }
        Err(e) => errors.push(format!("uid coverage check failed: {}", e)),
    }

    // Duplicate uids per label.
    let schema = ctx.graph.get_schema(&ctx.graph_path).unwrap_or_default();
    for label in &schema.labels {
        let query = format!(
            "MATCH (n:`{}`) RETURN count(n) AS total, count(DISTINCT n.uid) AS distinct",
            label.replace('`', "``")
        );
        match ctx
            .graph
            .execute_read(&ctx.graph_path, &query, HashMap::new(), None)
        {
            Ok(page) => {
                let row = page.rows.first();
                let total = row
                    .and_then(|r| r.get("total"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let distinct = row
                    .and_then(|r| r.get("distinct"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if total != distinct {
                    errors.push(format!(
                        "Label '{}': {} nodes but {} distinct uids (duplicates exist)",
                        label, total, distinct
                    ));
                }
            }
            Err(e) => errors.push(format!("Duplicate check for '{}' failed: {}", label, e)),
        }
    }

    // Orphan nodes are a warning; engines without the pattern get a pass-through note.
    match ctx.graph.execute_read(
        &ctx.graph_path,
        "MATCH (n) WHERE NOT (n)--() RETURN count(n) AS c",
        HashMap::new(),
        None,
    ) {
        Ok(page) => {
            let orphans = page
                .rows
                .first()
                .and_then(|r| r.get("c"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if orphans > 0 {
                warnings.push(format!("{} orphan node(s) with no relationships", orphans));
            }
        }
        Err(e) => warnings.push(format!("Orphan check unsupported by engine: {}", e)),
    }

    Ok(CheckOutcome {
        name: "data-integrity".to_string(),
        passed: errors.is_empty(),
        errors,
        warnings,
    })
}

/// Embedding index health: profiles exist, chunks exist, dimensions agree.
pub async fn check_index_health(ctx: &Context) -> Result<CheckOutcome, CliError> {
    use relating_metadata::{
        count_chunks_for_profile, list_chunks_for_profile, list_embedding_profiles,
    };

    let mut warnings = Vec::new();
    let profiles = list_embedding_profiles(&ctx.pool, &ctx.project_id).await?;
    if profiles.is_empty() {
        warnings.push("No embedding index; GraphRAG features stay disabled".to_string());
        return Ok(CheckOutcome {
            name: "index-health".to_string(),
            passed: true,
            errors: Vec::new(),
            warnings,
        });
    }

    let mut errors = Vec::new();
    for profile in &profiles {
        let count = count_chunks_for_profile(&ctx.pool, &profile.id).await?;
        if count == 0 {
            warnings.push(format!(
                "Embedding profile '{}' has no chunks",
                profile.name
            ));
            continue;
        }
        let chunks = list_chunks_for_profile(&ctx.pool, &profile.id).await?;
        let bad = chunks
            .iter()
            .filter(|c| c.embedding.len() != profile.dimensions)
            .count();
        if bad > 0 {
            errors.push(format!(
                "Profile '{}': {} chunk(s) disagree with {} dims (index corruption?)",
                profile.name, bad, profile.dimensions
            ));
        }
    }

    // Retrieval config drift vs schema-as-code.
    if let Some(revision) = get_latest_schema_revision(&ctx.pool, &ctx.project_id).await? {
        if let Ok(document) = latest_document(&revision.document) {
            let stored =
                relating_metadata::get_retrieval_config(&ctx.pool, &ctx.project_id).await?;
            let (strategy, top_k) = match stored {
                Some(c) => (c.default_strategy, c.default_top_k),
                None => (relating_core::RetrievalStrategy::Hybrid, 5),
            };
            if strategy != document.retrieval.default_strategy
                || top_k != document.retrieval.default_top_k
            {
                warnings
                    .push("Retrieval defaults drifted from the latest schema revision".to_string());
            }
        }
    }

    Ok(CheckOutcome {
        name: "index-health".to_string(),
        passed: errors.is_empty(),
        errors,
        warnings,
    })
}

/// Gate 6: Aura compatibility (source always; target when connected).
pub async fn check_aura_compat(
    ctx: &Context,
    connection: Option<&relating_core::AuraConnection>,
) -> Result<CheckOutcome, CliError> {
    let report = analyze_project(
        &ctx.pool,
        ctx.graph.as_ref(),
        &ctx.project_id,
        &ctx.graph_path,
        connection,
    )
    .await?;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for finding in report
        .source_findings
        .iter()
        .chain(report.target_findings.iter())
    {
        match finding.verdict {
            CapabilityVerdict::Unsupported => {
                errors.push(format!("{}: {}", finding.subject, finding.detail))
            }
            CapabilityVerdict::RequiresManualReview => {
                warnings.push(format!("{}: {}", finding.subject, finding.detail))
            }
            _ => {}
        }
    }
    if connection.is_some() && !report.target_reachable {
        errors.push("Aura target is unreachable".to_string());
    }

    Ok(CheckOutcome {
        name: "aura-compat".to_string(),
        passed: errors.is_empty(),
        errors,
        warnings,
    })
}

pub fn project_graph_path(data_dir: &Path, project_id: &relating_core::ProjectId) -> PathBuf {
    data_dir
        .join("projects")
        .join(project_id.to_string())
        .join("graph.rl")
}
