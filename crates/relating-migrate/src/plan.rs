use crate::backend::TargetInfo;
use crate::compat::parse_neo4j_major_version;
use crate::error::MigrateError;
use relating_core::OntologySchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BATCH_SIZE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    Nodes,
    Relationships,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPlan {
    pub kind: BatchKind,
    pub name: String,
    pub count: u64,
    pub batch_size: usize,
    pub batch_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub node_batches: Vec<BatchPlan>,
    pub rel_batches: Vec<BatchPlan>,
    pub ddl_statements: Vec<String>,
    pub warnings: Vec<String>,
    pub total_nodes: u64,
    pub total_relationships: u64,
    pub total_batches: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunReport {
    pub plan: MigrationPlan,
    pub target_reachable: bool,
    pub target_agent: Option<String>,
    pub remote_writes: u64,
    pub blockers: Vec<String>,
}

pub fn escape_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

pub fn constraint_ddl(label: &str, property: &str, major_version: Option<u32>) -> String {
    let label = escape_identifier(label);
    let property = escape_identifier(property);
    match major_version {
        Some(4) => format!(
            "CREATE CONSTRAINT IF NOT EXISTS ON (n:{}) ASSERT n.{} IS UNIQUE",
            label, property
        ),
        _ => format!(
            "CREATE CONSTRAINT IF NOT EXISTS FOR (n:{}) REQUIRE n.{} IS UNIQUE",
            label, property
        ),
    }
}

pub fn index_ddl(label: &str, properties: &[String]) -> Option<String> {
    if properties.is_empty() {
        return None;
    }
    let keys = properties
        .iter()
        .map(|p| format!("n.{}", escape_identifier(p)))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "CREATE INDEX IF NOT EXISTS FOR (n:{}) ON ({})",
        escape_identifier(label),
        keys
    ))
}

pub fn build_plan(
    label_counts: &[(String, u64)],
    rel_counts: &[(String, u64)],
    ontology: Option<&OntologySchema>,
    target_agent: Option<&str>,
    batch_size: usize,
) -> Result<MigrationPlan, MigrateError> {
    if batch_size == 0 {
        return Err(MigrateError::Plan(
            "Batch size must be at least 1".to_string(),
        ));
    }
    let major_version = target_agent.and_then(parse_neo4j_major_version);

    let mut node_batches = Vec::new();
    let mut total_nodes = 0u64;
    for (label, count) in label_counts {
        total_nodes += count;
        node_batches.push(BatchPlan {
            kind: BatchKind::Nodes,
            name: label.clone(),
            count: *count,
            batch_size,
            batch_count: (*count as usize).div_ceil(batch_size),
        });
    }

    let mut rel_batches = Vec::new();
    let mut total_relationships = 0u64;
    for (rel_type, count) in rel_counts {
        total_relationships += count;
        rel_batches.push(BatchPlan {
            kind: BatchKind::Relationships,
            name: rel_type.clone(),
            count: *count,
            batch_size,
            batch_count: (*count as usize).div_ceil(batch_size),
        });
    }

    let mut ddl_statements = Vec::new();
    let mut warnings = Vec::new();

    // Stable uid merge keys always get unique constraints.
    for (label, _) in label_counts {
        ddl_statements.push(constraint_ddl(label, "uid", major_version));
    }

    match ontology {
        Some(schema) => {
            for entity in &schema.entities {
                for prop in &entity.properties {
                    if prop.is_primary_key && prop.name != "uid" {
                        ddl_statements.push(constraint_ddl(
                            &entity.label,
                            &prop.name,
                            major_version,
                        ));
                    }
                }
            }
        }
        None => warnings.push(
            "No approved ontology: only uid constraints are generated; domain primary keys are skipped".to_string(),
        ),
    }

    if major_version.is_none() {
        warnings.push(
            "Target version unknown: DDL uses Neo4j 5 syntax; re-plan after testing the connection for version-specific DDL".to_string(),
        );
    }

    let total_batches = node_batches.iter().map(|b| b.batch_count).sum::<usize>()
        + rel_batches.iter().map(|b| b.batch_count).sum::<usize>();

    Ok(MigrationPlan {
        node_batches,
        rel_batches,
        ddl_statements,
        warnings,
        total_nodes,
        total_relationships,
        total_batches,
    })
}

/// Pure dry-run evaluation: plans everything, writes nothing remotely.
/// `target` is the outcome of a read-only connection test.
pub fn dry_run(plan: MigrationPlan, target: &Result<TargetInfo, MigrateError>) -> DryRunReport {
    let mut blockers = Vec::new();
    let (target_reachable, target_agent) = match target {
        Ok(info) => (true, Some(info.server_agent.clone())),
        Err(e) => {
            blockers.push(format!("Target unreachable: {}", e));
            (false, None)
        }
    };

    if plan.total_nodes == 0 && plan.total_relationships == 0 {
        blockers.push("Source graph is empty: nothing to migrate".to_string());
    }

    DryRunReport {
        plan,
        target_reachable,
        target_agent,
        remote_writes: 0,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_identifier() {
        assert_eq!(escape_identifier("Person"), "`Person`");
        assert_eq!(escape_identifier("We`ird"), "`We``ird`");
    }

    #[test]
    fn test_build_plan_batches_and_ddl() {
        let plan = build_plan(
            &[("Person".to_string(), 1200)],
            &[("KNOWS".to_string(), 50)],
            None,
            Some("Neo4j/5.18.0"),
            500,
        )
        .unwrap();
        assert_eq!(plan.total_nodes, 1200);
        assert_eq!(plan.total_relationships, 50);
        assert_eq!(plan.node_batches[0].batch_count, 3);
        assert_eq!(plan.rel_batches[0].batch_count, 1);
        assert_eq!(plan.total_batches, 4);
        assert!(plan
            .ddl_statements
            .iter()
            .any(|s| s.contains("FOR (n:`Person`)") && s.contains("n.`uid` IS UNIQUE")));
        assert!(!plan.warnings.is_empty()); // no ontology warning
    }

    #[test]
    fn test_build_plan_legacy_ddl_for_neo4j_4() {
        let plan = build_plan(
            &[("Person".to_string(), 1)],
            &[],
            None,
            Some("Neo4j/4.4.31"),
            500,
        )
        .unwrap();
        assert!(plan
            .ddl_statements
            .iter()
            .any(|s| s.contains("ASSERT n.`uid` IS UNIQUE")));
    }

    #[test]
    fn test_build_plan_rejects_zero_batch() {
        assert!(build_plan(&[], &[], None, None, 0).is_err());
    }

    #[test]
    fn test_dry_run_never_writes() {
        let plan = build_plan(&[], &[], None, None, 100).unwrap();
        let report = dry_run(plan, &Err(MigrateError::Connection("down".to_string())));
        assert_eq!(report.remote_writes, 0);
        assert!(!report.target_reachable);
        assert_eq!(report.blockers.len(), 2); // unreachable + empty source
    }
}
