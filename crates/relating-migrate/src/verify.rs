use crate::backend::Neo4jBackend;
use crate::error::MigrateError;
use crate::packstream::{bolt_to_json, BoltValue};
use crate::plan::escape_identifier;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCounts {
    pub labels: Vec<(String, u64)>,
    pub rel_types: Vec<(String, u64)>,
    /// Per-label content hashes: (label, sha256 over the sorted source uids).
    /// Empty for callers that only verify counts.
    #[serde(default)]
    pub label_hashes: Vec<(String, String)>,
    /// Labels whose hash was deliberately skipped (over the hashing cap).
    #[serde(default)]
    pub hash_skipped_labels: Vec<String>,
    /// Per-(label, relationship type) outgoing edge counts from the source.
    #[serde(default)]
    pub degrees: Vec<DegreeExpectation>,
    /// When true, count checks pass if the target is a superset of the
    /// source (delta sync without delete propagation leaves tombstoned
    /// source deletes on the target). Hash checks still require exact
    /// membership unless the caller skips them.
    #[serde(default)]
    pub allow_target_superset: bool,
}

/// Expected outgoing edge count for one (label, relationship type) pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeExpectation {
    pub label: String,
    pub rel_type: String,
    pub out_count: u64,
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
pub struct HashCheck {
    pub label: String,
    pub source_hash: String,
    pub target_hash: String,
    pub matched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeCheck {
    pub label: String,
    pub rel_type: String,
    pub source_count: u64,
    pub target_count: u64,
    pub matched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub checks: Vec<CountCheck>,
    #[serde(default)]
    pub hash_checks: Vec<HashCheck>,
    #[serde(default)]
    pub degree_checks: Vec<DegreeCheck>,
    pub samples_checked: usize,
    pub samples_matched: usize,
    pub passed: bool,
    pub details: Vec<String>,
}

/// Labels bigger than this skip content hashing (the uid listing would
/// dwarf the verify call); counts still verify and the skip is reported.
pub const MAX_HASH_UIDS_PER_LABEL: u64 = 200_000;

/// sha256 hex over the sorted uid list. Both sides sort before hashing so
/// row order never matters, only membership.
pub fn uid_list_hash(uids: &[String]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort();
    let mut hasher = Sha256::new();
    for uid in &sorted {
        hasher.update(uid.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
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

async fn target_count(dest: &dyn Neo4jBackend, query: String) -> Result<u64, MigrateError> {
    let rows = dest.run_read(&query, BTreeMap::new()).await?;
    Ok(rows
        .first()
        .and_then(|row| row.first())
        .and_then(|v| v.as_int())
        .unwrap_or(0)
        .max(0) as u64)
}

pub async fn verify_migration(
    dest: &dyn Neo4jBackend,
    source: &SourceCounts,
    samples: &[SampleExpectation],
) -> Result<VerificationReport, MigrateError> {
    let mut checks = Vec::new();
    let mut hash_checks = Vec::new();
    let mut degree_checks = Vec::new();
    let mut details = Vec::new();

    for (label, source_count) in &source.labels {
        let query = format!(
            "MATCH (n:{}) RETURN count(n) AS c",
            escape_identifier(label)
        );
        let target_count = target_count(dest, query).await?;
        let matched = if source.allow_target_superset {
            target_count >= *source_count
        } else {
            target_count == *source_count
        };
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
        let target_count = target_count(dest, query).await?;
        let matched = if source.allow_target_superset {
            target_count >= *source_count
        } else {
            target_count == *source_count
        };
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

    // Per-label content hashes: equal counts with different members fail here.
    for (label, source_hash) in &source.label_hashes {
        let query = format!("MATCH (n:{}) RETURN n.uid AS uid", escape_identifier(label));
        let rows = dest.run_read(&query, BTreeMap::new()).await?;
        let target_uids: Vec<String> = rows
            .iter()
            .filter_map(|row| row.first().and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        let target_hash = uid_list_hash(&target_uids);
        let matched = target_hash == *source_hash;
        if !matched {
            details.push(format!(
                "Label '{}': content hash differs ({} target uids listed); membership drift despite counts",
                label,
                target_uids.len()
            ));
        }
        hash_checks.push(HashCheck {
            label: label.clone(),
            source_hash: source_hash.clone(),
            target_hash,
            matched,
        });
    }
    for label in &source.hash_skipped_labels {
        details.push(format!(
            "Label '{}': content hash skipped (over the {} uid hashing cap); counts still verified",
            label, MAX_HASH_UIDS_PER_LABEL
        ));
    }

    // Relationship-degree comparison: edges attached to the wrong labels
    // fail here even when per-type totals match.
    for degree in &source.degrees {
        let query = format!(
            "MATCH (n:{})-[r:{}]->() RETURN count(r) AS c",
            escape_identifier(&degree.label),
            escape_identifier(&degree.rel_type)
        );
        let target_count = target_count(dest, query).await?;
        let matched = target_count == degree.out_count;
        if !matched {
            details.push(format!(
                "Degree '{}'-[:{}]->: source has {} edges but target has {}",
                degree.label, degree.rel_type, degree.out_count, target_count
            ));
        }
        degree_checks.push(DegreeCheck {
            label: degree.label.clone(),
            rel_type: degree.rel_type.clone(),
            source_count: degree.out_count,
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

    let passed = checks.iter().all(|c| c.matched)
        && hash_checks.iter().all(|c| c.matched)
        && degree_checks.iter().all(|c| c.matched)
        && samples_matched == samples.len();
    Ok(VerificationReport {
        checks,
        hash_checks,
        degree_checks,
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

    fn source_counts(labels: Vec<(String, u64)>, rel_types: Vec<(String, u64)>) -> SourceCounts {
        SourceCounts {
            labels,
            rel_types,
            label_hashes: Vec::new(),
            hash_skipped_labels: Vec::new(),
            degrees: Vec::new(),
            allow_target_superset: false,
        }
    }

    #[tokio::test]
    async fn test_verify_detects_match_and_mismatch() {
        let dest = MockNeo4jBackend::new();
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String("Person:1".to_string()));
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
        migrate_node_batch(&dest, "Person", vec![row])
            .await
            .unwrap();

        let source = source_counts(
            vec![("Person".to_string(), 1), ("Ghost".to_string(), 3)],
            vec![],
        );
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

    #[test]
    fn test_uid_list_hash_ignores_order() {
        let a = uid_list_hash(&["b".to_string(), "a".to_string()]);
        let b = uid_list_hash(&["a".to_string(), "b".to_string()]);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, uid_list_hash(&["a".to_string()]));
    }

    #[tokio::test]
    async fn test_hash_check_catches_membership_drift_behind_equal_counts() {
        let dest = MockNeo4jBackend::new();
        // Target holds a DIFFERENT uid than the source, with the same count.
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String("Person:2".to_string()));
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
        migrate_node_batch(&dest, "Person", vec![row])
            .await
            .unwrap();

        let source = SourceCounts {
            labels: vec![("Person".to_string(), 1)],
            rel_types: vec![],
            label_hashes: vec![(
                "Person".to_string(),
                uid_list_hash(&["Person:1".to_string()]),
            )],
            hash_skipped_labels: Vec::new(),
            degrees: Vec::new(),
            allow_target_superset: false,
        };
        let report = verify_migration(&dest, &source, &[]).await.unwrap();
        assert!(report.checks[0].matched);
        assert!(!report.hash_checks[0].matched);
        assert!(!report.passed);
        assert!(report.details.iter().any(|d| d.contains("content hash")));
    }

    #[tokio::test]
    async fn test_round_trip_migrate_then_read_back_then_verify() {
        use crate::execute::migrate_relationship_batch;
        use std::collections::BTreeMap as Map;

        let dest = MockNeo4jBackend::new();
        // Migrate: two nodes plus the edge between them.
        for (uid, name) in [("Person:1", "Ada"), ("Person:2", "Bob")] {
            let mut props = Map::new();
            props.insert("name".to_string(), BoltValue::String(name.to_string()));
            props.insert("uid".to_string(), BoltValue::String(uid.to_string()));
            let mut row = Map::new();
            row.insert("uid".to_string(), BoltValue::String(uid.to_string()));
            row.insert("props".to_string(), BoltValue::Map(props));
            migrate_node_batch(&dest, "Person", vec![row])
                .await
                .unwrap();
        }
        let mut rel = Map::new();
        rel.insert(
            "uid".to_string(),
            BoltValue::String("KNOWS:Person:1->Person:2#rel:7".to_string()),
        );
        rel.insert(
            "suid".to_string(),
            BoltValue::String("Person:1".to_string()),
        );
        rel.insert(
            "tuid".to_string(),
            BoltValue::String("Person:2".to_string()),
        );
        rel.insert("props".to_string(), BoltValue::Map(Map::new()));
        migrate_relationship_batch(&dest, "KNOWS", vec![rel])
            .await
            .unwrap();

        // Read back: counts, hashes, degrees, and a property sample.
        let source = SourceCounts {
            labels: vec![("Person".to_string(), 2)],
            rel_types: vec![("KNOWS".to_string(), 1)],
            label_hashes: vec![(
                "Person".to_string(),
                uid_list_hash(&["Person:1".to_string(), "Person:2".to_string()]),
            )],
            hash_skipped_labels: Vec::new(),
            degrees: vec![DegreeExpectation {
                label: "Person".to_string(),
                rel_type: "KNOWS".to_string(),
                out_count: 1,
            }],
            allow_target_superset: false,
        };
        let samples = vec![SampleExpectation {
            label: "Person".to_string(),
            uid: "Person:1".to_string(),
            expected_props: serde_json::json!({"name": "Ada"}),
        }];
        let report = verify_migration(&dest, &source, &samples).await.unwrap();
        assert!(report.passed, "{:?}", report.details);
        assert_eq!(report.samples_matched, 1);
        assert_eq!(report.hash_checks.len(), 1);
        assert_eq!(report.degree_checks.len(), 1);
    }
}
