use crate::error::SynthError;
use crate::generate::SynthDataset;
use relating_graph::GraphBackend;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeOutcome {
    pub nodes_created: i64,
    pub relationships_created: i64,
    pub properties_set: i64,
}

fn escape(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Load a synthetic dataset into a graph store with idempotent MERGE batches.
/// Every node and relationship carries `_synthetic: true` so test data is
/// always distinguishable from real data.
pub fn materialize_dataset(
    graph: &dyn GraphBackend,
    store_path: &Path,
    dataset: &SynthDataset,
) -> Result<MaterializeOutcome, SynthError> {
    graph.ensure_store(store_path)?;

    let mut nodes_created = 0i64;
    let mut relationships_created = 0i64;
    let mut properties_set = 0i64;

    // Group nodes by label for batched MERGE.
    let mut by_label: HashMap<String, Vec<&crate::generate::SynthNode>> = HashMap::new();
    for node in &dataset.nodes {
        if let Some(label) = node.labels.first() {
            by_label.entry(label.clone()).or_default().push(node);
        }
    }
    let mut labels: Vec<String> = by_label.keys().cloned().collect();
    labels.sort();
    for label in labels {
        let group = &by_label[&label];
        for chunk in group.chunks(500) {
            let batch: Vec<serde_json::Value> = chunk
                .iter()
                .map(|node| {
                    serde_json::Value::Object(
                        node.props
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    )
                })
                .collect();
            let query = format!(
                "UNWIND $batch AS row MERGE (n:{} {{uid: row.uid}}) SET n += row",
                escape(&label)
            );
            let mut params = HashMap::new();
            params.insert("batch".to_string(), serde_json::Value::Array(batch));
            let outcome = graph.execute_write(store_path, &query, params)?;
            nodes_created += outcome.nodes_created;
            properties_set += outcome.properties_set;
        }
    }

    // Relationships in type batches; endpoints were materialized above.
    let mut by_type: HashMap<String, Vec<&crate::generate::SynthRel>> = HashMap::new();
    for rel in &dataset.rels {
        by_type.entry(rel.rel_type.clone()).or_default().push(rel);
    }
    let mut types: Vec<String> = by_type.keys().cloned().collect();
    types.sort();
    for rel_type in types {
        let group = &by_type[&rel_type];
        for chunk in group.chunks(500) {
            let batch: Vec<serde_json::Value> = chunk
                .iter()
                .map(|rel| {
                    serde_json::json!({
                        "suid": rel.source_uid,
                        "tuid": rel.target_uid,
                        "props": rel.props,
                    })
                })
                .collect();
            let query = format!(
                "UNWIND $batch AS row MATCH (s {{uid: row.suid}}), (t {{uid: row.tuid}}) MERGE (s)-[r:{}]->(t) SET r += row.props",
                escape(&rel_type)
            );
            let mut params = HashMap::new();
            params.insert("batch".to_string(), serde_json::Value::Array(batch));
            let outcome = graph.execute_write(store_path, &query, params)?;
            relationships_created += outcome.relationships_created;
            properties_set += outcome.properties_set;
        }
    }

    Ok(MaterializeOutcome {
        nodes_created,
        relationships_created,
        properties_set,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::{generate_dataset, SynthConfig};
    use crate::profile::{GraphProfile, LabelProfile, PropertyProfile, RelProfile, RelSample};
    use relating_graph::RelateBackend;
    use tempfile::tempdir;

    #[test]
    fn test_materialize_round_trip() {
        let profile = GraphProfile {
            labels: vec![LabelProfile {
                label: "Person".to_string(),
                count: 4,
                properties: vec![PropertyProfile {
                    name: "name".to_string(),
                    data_type: "string".to_string(),
                    coverage: 1.0,
                }],
            }],
            rel_types: vec![RelProfile {
                rel_type: "KNOWS".to_string(),
                count: 3,
                endpoints: vec![RelSample {
                    source_label: "Person".to_string(),
                    target_label: "Person".to_string(),
                }],
                properties: Vec::new(),
            }],
            total_nodes: 4,
            total_relationships: 3,
        };
        let dataset = generate_dataset(&profile, &SynthConfig::default()).unwrap();

        let dir = tempdir().unwrap();
        let store = dir.path().join("synth.rl");
        let backend = RelateBackend::new();
        let outcome = materialize_dataset(&backend, &store, &dataset).unwrap();
        assert_eq!(outcome.nodes_created, 4);
        assert_eq!(outcome.relationships_created, 3);

        // Re-materializing is idempotent.
        let replay = materialize_dataset(&backend, &store, &dataset).unwrap();
        assert_eq!(replay.nodes_created, 0);

        // Synthetic marker is queryable.
        let page = backend
            .execute_read(
                &store,
                "MATCH (n) WHERE n._synthetic = true RETURN count(n) AS c",
                HashMap::new(),
                None,
            )
            .unwrap();
        assert_eq!(page.rows[0]["c"], serde_json::json!(4));
    }
}
