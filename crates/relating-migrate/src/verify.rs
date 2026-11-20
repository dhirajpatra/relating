use crate::backend::Neo4jBackend;
use crate::error::MigrateError;
use crate::packstream::{bolt_to_json, BoltValue};
use crate::plan::escape_identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCounts {
    pub labels: Vec<(String, u64)>,
    pub rel_types: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountCheck {
    pub kind: String,
    pub name: String,
    pub source_count: u64,
    pub target_count: u64,
    pub matched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub checks: Vec<CountCheck>,
    pub samples_checked: usize,
    pub samples_matched: usize,
    pub passed: bool,
    pub details: Vec<String>,
}

/// Expected property subset for one migrated node, keyed by label + uid.
#[derive(Debug, Clone)]
pub struct SampleExpectation {
    pub label: String,
    pub uid: String,
    pub expected_props: serde_json::Value,
}

fn props_match(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::Object(actual_map), serde_json::Value::Object(expected_map)) => {
            expected_map
                .iter()
                .all(|(k, v)| actual_map.get(k) == Some(v))
        }
        _ => actual == expected,
    }
}

pub async fn verify_migration(
    dest: &dyn Neo4jBackend,
    source: &SourceCounts,
    samples: &[SampleExpectation],
) -> Result<VerificationReport, MigrateError> {
    let mut checks = Vec::new();
    let mut details = Vec::new();

    for (label, source_count) in &source.labels {
        let query = format!(
            "MATCH (n:{}) RETURN count(n) AS c",
            escape_identifier(label)
        );
        let rows = dest.run_read(&query, BTreeMap::new()).await?;
        let target_count = rows
            .first()
            .and_then(|row| row.first())
            .and_then(|v| v.as_int())
            .unwrap_or(0)
            .max(0) as u64;
        let matched = target_count == *source_count;
        if !matched {
            details.push(format!(
                "Label '{}': source has {} nodes but target has {}",
                label, source_count, target_count
            ));
        }
        checks.push(CountCheck {
            kind: "nodes".to_string(),
            name: label.clone(),
            source_count: *source_count,
            target_count,
            matched,
        });
    }

    for (rel_type, source_count) in &source.rel_types {
        let query = format!(
            "MATCH ()-[r:{}]->() RETURN count(r) AS c",
            escape_identifier(rel_type)
        );
        let rows = dest.run_read(&query, BTreeMap::new()).await?;
        let target_count = rows
            .first()
            .and_then(|row| row.first())
            .and_then(|v| v.as_int())
            .unwrap_or(0)
            .max(0) as u64;
        let matched = target_count == *source_count;
        if !matched {
            details.push(format!(
                "Relationship '{}': source has {} but target has {}",
                rel_type, source_count, target_count
            ));
        }
        checks.push(CountCheck {
            kind: "relationships".to_string(),
            name: rel_type.clone(),
            source_count: *source_count,
            target_count,
            matched,
        });
    }

    let mut samples_matched = 0usize;
    for sample in samples {
        let query = format!(
            "MATCH (n:{} {{uid: $uid}}) RETURN n AS node",
            escape_identifier(&sample.label)
        );
        let mut params = BTreeMap::new();
        params.insert("uid".to_string(), BoltValue::String(sample.uid.clone()));
        let rows = dest.run_read(&query, params).await?;
        let actual = rows
            .first()
            .and_then(|row| row.first())
            .map(bolt_to_json)
            .and_then(|v| v.get("properties").cloned())
            .unwrap_or(serde_json::Value::Null);
        if props_match(&actual, &sample.expected_props) {
            samples_matched += 1;
        } else {
            details.push(format!(
                "Sample '{}' ({}) properties differ from source",
                sample.uid, sample.label
            ));
        }
    }

    let passed = checks.iter().all(|c| c.matched) && samples_matched == samples.len();
    Ok(VerificationReport {
        checks,
        samples_checked: samples.len(),
        samples_matched,
        passed,
        details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockNeo4jBackend;
    use crate::execute::migrate_node_batch;

    #[tokio::test]
    async fn test_verify_detects_match_and_mismatch() {
        let dest = MockNeo4jBackend::new();
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String("Person:1".to_string()));
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
        migrate_node_batch(&dest, "Person", vec![row])
            .await
            .unwrap();

        let source = SourceCounts {
            labels: vec![("Person".to_string(), 1), ("Ghost".to_string(), 3)],
            rel_types: vec![],
        };
        let report = verify_migration(&dest, &source, &[]).await.unwrap();
        assert!(!report.passed);
        assert!(report.checks[0].matched);
        assert!(!report.checks[1].matched);
        assert_eq!(report.details.len(), 1);
    }

    #[test]
    fn test_props_match_subset_semantics() {
        let actual = serde_json::json!({"name": "Ada", "age": 36});
        assert!(props_match(&actual, &serde_json::json!({"name": "Ada"})));
        assert!(!props_match(&actual, &serde_json::json!({"name": "Bob"})));
    }
}
