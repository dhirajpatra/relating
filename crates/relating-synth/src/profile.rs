use crate::error::SynthError;
use relating_graph::GraphBackend;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyProfile {
    pub name: String,
    /// Declared storage type sampled from live data.
    pub data_type: String,
    /// Fraction of sampled nodes carrying the property (0.0-1.0).
    pub coverage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelProfile {
    pub label: String,
    pub count: u64,
    pub properties: Vec<PropertyProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelSample {
    pub source_label: String,
    pub target_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelProfile {
    pub rel_type: String,
    pub count: u64,
    /// Endpoint label distribution sampled from live relationships.
    pub endpoints: Vec<RelSample>,
    /// Relationship property shapes (uid excluded).
    #[serde(default)]
    pub properties: Vec<PropertyProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphProfile {
    pub labels: Vec<LabelProfile>,
    pub rel_types: Vec<RelProfile>,
    pub total_nodes: u64,
    pub total_relationships: u64,
}

pub const PROFILE_SAMPLE_LIMIT: u32 = 500;

fn storage_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::String(_) => "string",
        serde_json::Value::Number(n) if n.is_i64() => "integer",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
        serde_json::Value::Null => "null",
    }
}

fn escape(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Profile the source graph: per-label counts and property types,
/// per-relationship counts and endpoint label samples. Sampling keeps this
/// cheap on large graphs; exact totals come from count queries.
pub fn profile_graph(
    graph: &dyn GraphBackend,
    store_path: &Path,
) -> Result<GraphProfile, SynthError> {
    let label_counts = graph.get_label_counts(store_path)?;
    let rel_counts = graph.get_rel_type_counts(store_path)?;

    let mut labels = Vec::new();
    let mut total_nodes = 0u64;
    for (label, count) in &label_counts {
        total_nodes += count;
        // Sample property shapes (uid excluded: synthetic uids are generated).
        let mut type_votes: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut sampled = 0usize;
        let query = format!(
            "MATCH (n:{}) RETURN n LIMIT {}",
            escape(label),
            PROFILE_SAMPLE_LIMIT
        );
        if let Ok(page) = graph.execute_read(store_path, &query, HashMap::new(), None) {
            for row in page.rows.iter().take(PROFILE_SAMPLE_LIMIT as usize) {
                let Some(props) = row.get("n").and_then(|n| n.get("properties")) else {
                    continue;
                };
                let Some(map) = props.as_object() else {
                    continue;
                };
                sampled += 1;
                for (key, value) in map {
                    if key == "uid" || key.starts_with('_') {
                        continue;
                    }
                    type_votes
                        .entry(key.clone())
                        .or_default()
                        .entry(storage_type(value).to_string())
                        .and_modify(|c| *c += 1)
                        .or_insert(1);
                }
            }
        }
        let mut properties: Vec<PropertyProfile> = type_votes
            .into_iter()
            .map(|(name, votes)| {
                let total_votes: usize = votes.values().sum();
                let data_type = votes
                    .into_iter()
                    .max_by_key(|(_, c)| *c)
                    .map(|(t, _)| t)
                    .unwrap_or_else(|| "string".to_string());
                PropertyProfile {
                    name,
                    data_type,
                    coverage: if sampled > 0 {
                        total_votes as f32 / sampled as f32
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        properties.sort_by(|a, b| a.name.cmp(&b.name));
        labels.push(LabelProfile {
            label: label.clone(),
            count: *count,
            properties,
        });
    }

    let mut rel_types = Vec::new();
    let mut total_relationships = 0u64;
    for (rel_type, count) in &rel_counts {
        total_relationships += count;
        let mut endpoints = Vec::new();
        let mut type_votes: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut sampled = 0usize;
        let query = format!(
            "MATCH (s)-[r:{}]->(t) RETURN s AS s, t AS t, r AS r LIMIT {}",
            escape(rel_type),
            PROFILE_SAMPLE_LIMIT
        );
        if let Ok(page) = graph.execute_read(store_path, &query, HashMap::new(), None) {
            for row in page.rows.iter().take(PROFILE_SAMPLE_LIMIT as usize) {
                let source_label = row
                    .get("s")
                    .and_then(|n| n.get("labels"))
                    .and_then(|l| l.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("Entity")
                    .to_string();
                let target_label = row
                    .get("t")
                    .and_then(|n| n.get("labels"))
                    .and_then(|l| l.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("Entity")
                    .to_string();
                endpoints.push(RelSample {
                    source_label,
                    target_label,
                });
                if let Some(map) = row
                    .get("r")
                    .and_then(|r| r.get("properties"))
                    .and_then(|p| p.as_object())
                {
                    sampled += 1;
                    for (key, value) in map {
                        if key == "uid" || key.starts_with('_') {
                            continue;
                        }
                        type_votes
                            .entry(key.clone())
                            .or_default()
                            .entry(storage_type(value).to_string())
                            .and_modify(|c| *c += 1)
                            .or_insert(1);
                    }
                }
            }
        }
        let mut properties: Vec<PropertyProfile> = type_votes
            .into_iter()
            .map(|(name, votes)| {
                let total_votes: usize = votes.values().sum();
                let data_type = votes
                    .into_iter()
                    .max_by_key(|(_, c)| *c)
                    .map(|(t, _)| t)
                    .unwrap_or_else(|| "string".to_string());
                PropertyProfile {
                    name,
                    data_type,
                    coverage: if sampled > 0 {
                        total_votes as f32 / sampled as f32
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        properties.sort_by(|a, b| a.name.cmp(&b.name));
        rel_types.push(RelProfile {
            rel_type: rel_type.clone(),
            count: *count,
            endpoints,
            properties,
        });
    }

    // Stable order for deterministic generation.
    labels.sort_by(|a, b| a.label.cmp(&b.label));
    rel_types.sort_by(|a, b| a.rel_type.cmp(&b.rel_type));

    Ok(GraphProfile {
        labels,
        rel_types,
        total_nodes,
        total_relationships,
    })
}

/// Distinct values check helper (used by tests and the advisor later).
pub fn property_names(profile: &GraphProfile) -> BTreeMap<String, Vec<String>> {
    profile
        .labels
        .iter()
        .map(|l| {
            (
                l.label.clone(),
                l.properties.iter().map(|p| p.name.clone()).collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_graph::RelateBackend;
    use tempfile::tempdir;

    #[test]
    fn test_profile_captures_distributions() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        backend
            .execute_write(
                &store,
                "CREATE (a:Person {uid: 'Person:1', name: 'Ada', age: 36})-[:KNOWS]->(b:Person {uid: 'Person:2', name: 'Bob'})",
                HashMap::new(),
            )
            .unwrap();

        let profile = profile_graph(&backend, &store).unwrap();
        assert_eq!(profile.total_nodes, 2);
        assert_eq!(profile.total_relationships, 1);
        let person = profile.labels.iter().find(|l| l.label == "Person").unwrap();
        assert_eq!(person.count, 2);
        let names: Vec<&str> = person.properties.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"age"));
        assert!(!names.contains(&"uid")); // synthetic uids are generated, not profiled
        let age = person.properties.iter().find(|p| p.name == "age").unwrap();
        assert_eq!(age.data_type, "integer");
        assert!((age.coverage - 0.5).abs() < 1e-5);

        let knows = profile
            .rel_types
            .iter()
            .find(|r| r.rel_type == "KNOWS")
            .unwrap();
        assert_eq!(knows.count, 1);
        assert_eq!(knows.endpoints.len(), 1);
        assert_eq!(knows.endpoints[0].source_label, "Person");
    }

    #[test]
    fn test_profile_empty_graph() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        let profile = profile_graph(&backend, &store).unwrap();
        assert_eq!(profile.total_nodes, 0);
        assert!(profile.labels.is_empty());
    }
}
