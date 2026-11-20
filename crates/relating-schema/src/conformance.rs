use crate::document::SchemaDocument;
use relating_graph::{GraphBackend, GraphError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCheck {
    pub subject: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub checks: Vec<ContractCheck>,
    pub passed: bool,
}

fn escape(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::String(_) => "string",
        serde_json::Value::Number(n) if n.is_i64() => "integer",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Null => "null",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Check that the live graph conforms to the contract: required properties
/// exist with declared types (sampled), and unique constraints hold.
/// A check that cannot run is reported as failed with the reason — never
/// silently passed.
pub fn check_conformance(
    graph: &dyn GraphBackend,
    store_path: &Path,
    document: &SchemaDocument,
    sample_limit: usize,
) -> Result<ConformanceReport, GraphError> {
    let mut checks = Vec::new();
    let sample_limit = sample_limit.clamp(1, 1000) as u32;

    for entity in &document.ontology.entities {
        // Sample nodes for property presence and types.
        let query = format!(
            "MATCH (n:{}) RETURN n LIMIT {}",
            escape(&entity.label),
            sample_limit
        );
        let page = match graph.execute_read(store_path, &query, HashMap::new(), None) {
            Ok(page) => page,
            Err(e) => {
                checks.push(ContractCheck {
                    subject: format!("sample {}", entity.label),
                    passed: false,
                    detail: format!("Could not sample nodes: {}", e),
                });
                continue;
            }
        };

        for prop in &entity.properties {
            let mut missing = 0usize;
            let mut mistyped = 0usize;
            let mut sampled = 0usize;
            for row in &page.rows {
                let props = match row.get("n").and_then(|n| n.get("properties")) {
                    Some(serde_json::Value::Object(map)) => map,
                    _ => continue,
                };
                sampled += 1;
                match props.get(&prop.name) {
                    None | Some(serde_json::Value::Null) => missing += 1,
                    Some(value) => {
                        let actual = json_kind(value);
                        let expected_ok = match prop.data_type.as_str() {
                            "string" => actual == "string",
                            "integer" => actual == "integer",
                            // Integers are valid floats for storage checks.
                            "float" => actual == "float" || actual == "integer",
                            "boolean" => actual == "boolean",
                            _ => true,
                        };
                        if !expected_ok {
                            mistyped += 1;
                        }
                    }
                }
            }
            let passed = missing == 0 && mistyped == 0;
            checks.push(ContractCheck {
                subject: format!("{}.{}", entity.label, prop.name),
                passed,
                detail: if sampled == 0 {
                    format!("No '{}' nodes sampled; property unchecked", entity.label)
                } else {
                    format!(
                        "{}/{} sampled nodes carry '{}' as {}",
                        sampled - missing - mistyped,
                        sampled,
                        prop.name,
                        prop.data_type
                    )
                },
            });
        }
    }

    // Uniqueness spot-checks via total vs distinct counts.
    for constraint in &document.constraints {
        if constraint.properties.len() != 1 {
            checks.push(ContractCheck {
                subject: format!(
                    "unique {}({})",
                    constraint.label,
                    constraint.properties.join(",")
                ),
                passed: false,
                detail: "Composite uniqueness checks are not supported yet".to_string(),
            });
            continue;
        }
        let prop = &constraint.properties[0];
        let query = format!(
            "MATCH (n:{}) RETURN count(n) AS total, count(DISTINCT n.{}) AS distinct",
            escape(&constraint.label),
            escape(prop)
        );
        match graph.execute_read(store_path, &query, HashMap::new(), None) {
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
                let passed = total == distinct;
                checks.push(ContractCheck {
                    subject: format!("unique {}.{}", constraint.label, prop),
                    passed,
                    detail: format!(
                        "{} nodes, {} distinct '{}' values{}",
                        total,
                        distinct,
                        prop,
                        if passed {
                            String::new()
                        } else {
                            " — duplicates exist".to_string()
                        }
                    ),
                });
            }
            Err(e) => checks.push(ContractCheck {
                subject: format!("unique {}.{}", constraint.label, prop),
                passed: false,
                detail: format!("Could not verify uniqueness: {}", e),
            }),
        }
    }

    let passed = checks.iter().all(|c| c.passed);
    Ok(ConformanceReport { checks, passed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_graph::RelateBackend;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn seed_graph() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        backend
            .execute_write(
                &store,
                "CREATE (a:Person {uid: 'Person:1', name: 'Ada', age: 36})",
                HashMap::new(),
            )
            .unwrap();
        backend
            .execute_write(
                &store,
                "CREATE (b:Person {uid: 'Person:2', name: 'Bob'})",
                HashMap::new(),
            )
            .unwrap();
        (dir, store)
    }

    fn person_doc() -> SchemaDocument {
        let mut doc = SchemaDocument::new(None);
        doc.ontology.entities.push(relating_core::EntityDefinition {
            label: "Person".to_string(),
            display_name: "Person".to_string(),
            properties: vec![
                relating_core::PropertyDefinition {
                    name: "uid".to_string(),
                    data_type: "string".to_string(),
                    is_primary_key: true,
                    description: None,
                },
                relating_core::PropertyDefinition {
                    name: "name".to_string(),
                    data_type: "string".to_string(),
                    is_primary_key: false,
                    description: None,
                },
            ],
            description: String::new(),
        });
        doc.constraints.push(crate::document::UniqueConstraint {
            label: "Person".to_string(),
            properties: vec!["uid".to_string()],
        });
        doc
    }

    #[test]
    fn test_conformance_passes_on_matching_graph() {
        let (_dir, store) = seed_graph();
        let backend = RelateBackend::new();
        let report = check_conformance(&backend, &store, &person_doc(), 100).unwrap();
        assert!(report.passed, "checks: {:?}", report.checks);
    }

    #[test]
    fn test_conformance_catches_missing_property() {
        let (_dir, store) = seed_graph();
        let backend = RelateBackend::new();
        let mut doc = person_doc();
        doc.ontology.entities[0]
            .properties
            .push(relating_core::PropertyDefinition {
                name: "email".to_string(),
                data_type: "string".to_string(),
                is_primary_key: false,
                description: None,
            });
        let report = check_conformance(&backend, &store, &doc, 100).unwrap();
        assert!(!report.passed);
        assert!(report
            .checks
            .iter()
            .any(|c| c.subject == "Person.email" && !c.passed));
    }

    #[test]
    fn test_conformance_catches_duplicates() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        // Two nodes sharing one uid violate the unique constraint.
        for name in ["Ada", "Bob"] {
            backend
                .execute_write(
                    &store,
                    "CREATE (n:Person {uid: 'Person:1', name: $name})",
                    HashMap::from([("name".to_string(), serde_json::json!(name))]),
                )
                .unwrap();
        }
        let report = check_conformance(&backend, &store, &person_doc(), 100).unwrap();
        assert!(!report.passed);
        assert!(report
            .checks
            .iter()
            .any(|c| c.subject == "unique Person.uid" && !c.passed));
    }
}
