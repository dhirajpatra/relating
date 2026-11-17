use crate::backend::TargetInfo;
use relating_core::{EmbeddingProfile, OntologySchema};
use relating_graph::GraphSchemaInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityVerdict {
    Supported,
    Recreate,
    Transform,
    Unsupported,
    RequiresManualReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatFinding {
    pub category: String,
    pub subject: String,
    pub verdict: CapabilityVerdict,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

fn finding(
    category: &str,
    subject: impl Into<String>,
    verdict: CapabilityVerdict,
    detail: impl Into<String>,
    remediation: Option<&str>,
) -> CompatFinding {
    CompatFinding {
        category: category.to_string(),
        subject: subject.into(),
        verdict,
        detail: detail.into(),
        remediation: remediation.map(|s| s.to_string()),
    }
}

pub fn analyze_source(
    schema: &GraphSchemaInfo,
    ontology: Option<&OntologySchema>,
    embedding_profiles: &[EmbeddingProfile],
) -> Vec<CompatFinding> {
    let mut findings = Vec::new();

    for label in &schema.labels {
        findings.push(finding(
            "labels",
            label,
            CapabilityVerdict::Supported,
            format!("Node label '{}' maps natively to Neo4j", label),
            None,
        ));
    }

    for rel_type in &schema.relationship_types {
        findings.push(finding(
            "relationship_types",
            rel_type,
            CapabilityVerdict::Supported,
            format!("Relationship type '{}' maps natively to Neo4j", rel_type),
            None,
        ));
    }

    findings.push(finding(
        "identity",
        "uid merge keys",
        CapabilityVerdict::Supported,
        "Every migrated node carries a stable 'uid' property used for idempotent MERGE",
        None,
    ));

    match ontology {
        Some(schema) => findings.push(finding(
            "ontology",
            "approved ontology",
            CapabilityVerdict::Supported,
            format!(
                "Ontology defines {} entities and {} relationships; primary keys drive unique constraints",
                schema.entities.len(),
                schema.relationships.len()
            ),
            None,
        )),
        None => findings.push(finding(
            "ontology",
            "approved ontology",
            CapabilityVerdict::RequiresManualReview,
            "No approved ontology; labels transfer as-is without primary-key constraints",
            Some("Propose and approve an ontology before migrating"),
        )),
    }

    findings.push(finding(
        "schema_objects",
        "constraints and indexes",
        CapabilityVerdict::Recreate,
        "Uniqueness constraints and indexes are generated as Neo4j DDL from ontology primary keys and uid merge keys",
        Some("Review the generated DDL in the migration plan"),
    ));

    if embedding_profiles.is_empty() {
        findings.push(finding(
            "embeddings",
            "vector index",
            CapabilityVerdict::Supported,
            "No embedding index exists; nothing to port",
            None,
        ));
    } else {
        for profile in embedding_profiles {
            findings.push(finding(
                "embeddings",
                format!("profile '{}' ({} dims)", profile.name, profile.dimensions),
                CapabilityVerdict::Transform,
                "Embeddings live in Relating's local index and must be recreated as a Neo4j vector index; chunk data exports with the migration",
                Some("Create a Neo4j vector index with matching dimensions after migration"),
            ));
        }
    }

    findings.push(finding(
        "properties",
        "property types",
        CapabilityVerdict::RequiresManualReview,
        "Property values transfer as-is; temporal and spatial types need post-migration verification",
        Some("Run verification sampling after migration and inspect flagged properties"),
    ));

    findings.push(finding(
        "application",
        "application Cypher",
        CapabilityVerdict::RequiresManualReview,
        "Queries embedded in application code are analysed by the Portability Compiler (R3), not by data migration",
        None,
    ));

    findings
}

pub fn parse_neo4j_major_version(server_agent: &str) -> Option<u32> {
    // Server agents look like "Neo4j/5.18.0" or "Neo4j/Mock-5.0".
    let version_part = server_agent.split('/').nth(1)?;
    let digits: String = version_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    // Handle "Mock-5.0" style test agents by scanning for the first digit run.
    let major_str = if digits.is_empty() {
        let mut runs = version_part.split(|c: char| !c.is_ascii_digit());
        runs.find(|run| !run.is_empty())?.to_string()
    } else {
        digits
    };
    major_str.parse::<u32>().ok()
}

pub fn analyze_target(info: &TargetInfo, used_tls: bool) -> Vec<CompatFinding> {
    let mut findings = Vec::new();

    match parse_neo4j_major_version(&info.server_agent) {
        Some(major) if major >= 5 => findings.push(finding(
            "target_version",
            info.server_agent.clone(),
            CapabilityVerdict::Supported,
            "Neo4j 5 supports the generated DDL (IF NOT EXISTS constraints and indexes)",
            None,
        )),
        Some(4) => findings.push(finding(
            "target_version",
            info.server_agent.clone(),
            CapabilityVerdict::Transform,
            "Neo4j 4.4 is supported but DDL uses legacy constraint syntax",
            Some("Regenerate the plan against this target to get 4.x DDL"),
        )),
        Some(major) => findings.push(finding(
            "target_version",
            info.server_agent.clone(),
            CapabilityVerdict::Unsupported,
            format!("Neo4j {} is below the minimum supported version 4.4", major),
            Some("Upgrade the target database before migrating"),
        )),
        None => findings.push(finding(
            "target_version",
            info.server_agent.clone(),
            CapabilityVerdict::RequiresManualReview,
            "Could not parse the target Neo4j version from the server agent string",
            None,
        )),
    }

    if used_tls {
        findings.push(finding(
            "transport",
            "encrypted Bolt",
            CapabilityVerdict::Supported,
            "Connection uses TLS with verified certificates",
            None,
        ));
    } else {
        findings.push(finding(
            "transport",
            "plaintext Bolt",
            CapabilityVerdict::RequiresManualReview,
            "Connection is unencrypted; Neo4j Aura requires an encrypted (+s) URI",
            Some("Switch to a neo4j+s:// URI for Aura destinations"),
        ));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_schema() -> GraphSchemaInfo {
        GraphSchemaInfo {
            labels: vec!["Person".to_string()],
            relationship_types: vec!["KNOWS".to_string()],
            node_count: 10,
            relationship_count: 4,
            property_keys: vec!["name".to_string()],
        }
    }

    #[test]
    fn test_analyze_source_covers_core_categories() {
        let findings = analyze_source(&test_schema(), None, &[]);
        let categories: Vec<&str> = findings.iter().map(|f| f.category.as_str()).collect();
        for expected in [
            "labels",
            "relationship_types",
            "identity",
            "ontology",
            "schema_objects",
            "embeddings",
            "properties",
            "application",
        ] {
            assert!(categories.contains(&expected), "missing {}", expected);
        }
        assert!(findings
            .iter()
            .any(|f| f.subject == "Person" && f.verdict == CapabilityVerdict::Supported));
    }

    #[test]
    fn test_analyze_source_flags_embeddings_for_transform() {
        let profile = EmbeddingProfile {
            id: relating_core::EmbeddingProfileId::new(),
            project_id: relating_core::ProjectId::new(),
            name: "default".to_string(),
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            model_version: None,
            dimensions: 768,
            modality: "mixed".to_string(),
            created_at: chrono::Utc::now(),
        };
        let findings = analyze_source(&test_schema(), None, &[profile]);
        assert!(findings
            .iter()
            .any(|f| f.category == "embeddings" && f.verdict == CapabilityVerdict::Transform));
    }

    #[test]
    fn test_parse_neo4j_major_version() {
        assert_eq!(parse_neo4j_major_version("Neo4j/5.18.0"), Some(5));
        assert_eq!(parse_neo4j_major_version("Neo4j/4.4.31"), Some(4));
        assert_eq!(parse_neo4j_major_version("Neo4j/Mock-5.0"), Some(5));
        assert_eq!(parse_neo4j_major_version("garbage"), None);
    }

    #[test]
    fn test_analyze_target_versions() {
        let info = TargetInfo {
            server_agent: "Neo4j/5.18.0".to_string(),
            bolt_major: 4,
            bolt_minor: 4,
            database: None,
        };
        let findings = analyze_target(&info, true);
        assert!(findings
            .iter()
            .any(|f| f.category == "target_version" && f.verdict == CapabilityVerdict::Supported));

        let info = TargetInfo {
            server_agent: "Neo4j/3.5.0".to_string(),
            bolt_major: 3,
            bolt_minor: 0,
            database: None,
        };
        let findings = analyze_target(&info, false);
        assert!(
            findings
                .iter()
                .any(|f| f.category == "target_version"
                    && f.verdict == CapabilityVerdict::Unsupported)
        );
        assert!(findings
            .iter()
            .any(|f| f.category == "transport"
                && f.verdict == CapabilityVerdict::RequiresManualReview));
    }
}
