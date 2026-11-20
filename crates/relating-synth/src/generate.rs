use crate::error::SynthError;
use crate::profile::{GraphProfile, PropertyProfile};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeCase {
    Duplicates,
    Orphans,
    Cycle,
    InvalidValues,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthConfig {
    /// Fraction of source volume to generate (0.0-1.0, default 1.0).
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub edge_cases: Vec<EdgeCase>,
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
}

fn default_scale() -> f64 {
    1.0
}

fn default_max_nodes() -> usize {
    50_000
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
            seed: 0,
            edge_cases: Vec::new(),
            max_nodes: 50_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthNode {
    pub labels: Vec<String>,
    pub props: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthRel {
    pub rel_type: String,
    pub source_uid: String,
    pub target_uid: String,
    pub props: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthStats {
    pub node_count: usize,
    pub rel_count: usize,
    pub per_label: BTreeMap<String, u64>,
    pub per_type: BTreeMap<String, u64>,
    pub edge_cases_applied: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthDataset {
    pub nodes: Vec<SynthNode>,
    pub rels: Vec<SynthRel>,
    pub stats: SynthStats,
    pub notes: Vec<String>,
}

fn synth_value(
    rng: &mut StdRng,
    label: &str,
    prop: &PropertyProfile,
    index: usize,
) -> Option<serde_json::Value> {
    if rng.random_range(0.0..1.0) > prop.coverage {
        return None;
    }
    match prop.data_type.as_str() {
        "string" => Some(serde_json::Value::String(format!(
            "{}_{}_{}",
            label.to_lowercase(),
            prop.name,
            index
        ))),
        "integer" => Some(serde_json::Value::Number(
            (rng.random_range(1..10000) as i64).into(),
        )),
        "float" => {
            let raw: f64 = rng.random_range(0.0..10000.0);
            let value = (raw * 100.0).round() / 100.0;
            serde_json::Number::from_f64(value).map(serde_json::Value::Number)
        }
        "boolean" => Some(serde_json::Value::Bool(index.is_multiple_of(2))),
        // Complex shapes are skipped openly (see notes) rather than faked.
        _ => None,
    }
}

fn scaled_count(source_count: u64, scale: f64) -> usize {
    if scale <= 0.0 || source_count == 0 {
        return 0;
    }
    ((source_count as f64 * scale).round() as usize).max(1)
}

pub fn generate_dataset(
    profile: &GraphProfile,
    config: &SynthConfig,
) -> Result<SynthDataset, SynthError> {
    if !(0.0..=1.0).contains(&config.scale) {
        return Err(SynthError::Invalid(
            "scale must be between 0.0 and 1.0".to_string(),
        ));
    }
    if config.max_nodes == 0 {
        return Err(SynthError::Invalid(
            "max_nodes must be at least 1".to_string(),
        ));
    }

    let mut rng = StdRng::seed_from_u64(config.seed);
    let mut notes = Vec::new();
    let mut complex_skipped = false;

    // Cap total nodes honestly when scale exceeds the safety budget.
    let desired: usize = profile
        .labels
        .iter()
        .map(|l| scaled_count(l.count, config.scale))
        .sum();
    let node_budget = config.max_nodes;
    let mut allowance: HashMap<String, usize> = HashMap::new();
    if desired > node_budget {
        notes.push(format!(
            "Requested ~{} nodes but max_nodes is {}; scaling down proportionally",
            desired, node_budget
        ));
        let factor = node_budget as f64 / desired as f64;
        for label in &profile.labels {
            let scaled = scaled_count(label.count, config.scale);
            allowance.insert(
                label.label.clone(),
                ((scaled as f64 * factor).round() as usize).max(1.min(scaled)),
            );
        }
    } else {
        for label in &profile.labels {
            allowance.insert(label.label.clone(), scaled_count(label.count, config.scale));
        }
    }

    // Nodes with stable synthetic uids.
    let mut nodes = Vec::new();
    let mut uids_by_label: HashMap<String, Vec<String>> = HashMap::new();
    for label in &profile.labels {
        let count = allowance.get(&label.label).copied().unwrap_or(0);
        for i in 0..count {
            let uid = format!("SYNTH:{}:{:06}", label.label, i);
            let mut props = BTreeMap::new();
            props.insert("uid".to_string(), serde_json::Value::String(uid.clone()));
            props.insert("_synthetic".to_string(), serde_json::Value::Bool(true));
            for prop in &label.properties {
                match synth_value(&mut rng, &label.label, prop, i) {
                    Some(value) => {
                        props.insert(prop.name.clone(), value);
                    }
                    None if matches!(prop.data_type.as_str(), "array" | "object" | "null") => {
                        complex_skipped = true;
                    }
                    None => {}
                }
            }
            uids_by_label
                .entry(label.label.clone())
                .or_default()
                .push(uid);
            nodes.push(SynthNode {
                labels: vec![label.label.clone()],
                props,
            });
        }
    }
    if complex_skipped {
        notes.push(
            "Array/object/null properties are not synthesized; scalar shapes only".to_string(),
        );
    }

    // Relationships mirroring sampled endpoint distributions.
    let mut rels = Vec::new();
    for rel_profile in &profile.rel_types {
        let count = scaled_count(rel_profile.count, config.scale);
        if rel_profile.endpoints.is_empty() {
            if count > 0 {
                notes.push(format!(
                    "No endpoint samples for '{}'; relationships skipped",
                    rel_profile.rel_type
                ));
            }
            continue;
        }
        for _ in 0..count {
            let sample = &rel_profile.endpoints[rng.random_range(0..rel_profile.endpoints.len())];
            let sources = uids_by_label.get(&sample.source_label);
            let targets = uids_by_label.get(&sample.target_label);
            let (Some(sources), Some(targets)) = (sources, targets) else {
                continue;
            };
            if sources.is_empty() || targets.is_empty() {
                continue;
            }
            let source_uid = sources[rng.random_range(0..sources.len())].clone();
            let target_uid = targets[rng.random_range(0..targets.len())].clone();
            let mut props = BTreeMap::new();
            props.insert("_synthetic".to_string(), serde_json::Value::Bool(true));
            for prop in &rel_profile.properties {
                if let Some(value) = synth_value(&mut rng, &rel_profile.rel_type, prop, rels.len())
                {
                    props.insert(prop.name.clone(), value);
                }
            }
            rels.push(SynthRel {
                rel_type: rel_profile.rel_type.clone(),
                source_uid,
                target_uid,
                props,
            });
        }
    }

    // Explicit edge-case scenarios.
    let mut applied = Vec::new();
    for edge_case in &config.edge_cases {
        match edge_case {
            EdgeCase::Duplicates => {
                if let Some(node) = nodes.first().cloned() {
                    let mut dup_props = node.props.clone();
                    let label = node
                        .labels
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "Entity".to_string());
                    let dup_uid = format!("SYNTH:{}:dup", label);
                    dup_props.insert(
                        "uid".to_string(),
                        serde_json::Value::String(dup_uid.clone()),
                    );
                    uids_by_label
                        .entry(label.clone())
                        .or_default()
                        .push(dup_uid);
                    nodes.push(SynthNode {
                        labels: node.labels,
                        props: dup_props,
                    });
                    applied.push("duplicates: cloned first node under a fresh uid".to_string());
                } else {
                    notes.push("duplicates: skipped (no nodes generated)".to_string());
                }
            }
            EdgeCase::Orphans => {
                if let Some(label) = profile.labels.first() {
                    for i in 0..3 {
                        let uid = format!("SYNTH:{}:orphan{}", label.label, i);
                        let mut props = BTreeMap::new();
                        props.insert("uid".to_string(), serde_json::Value::String(uid.clone()));
                        props.insert("_synthetic".to_string(), serde_json::Value::Bool(true));
                        uids_by_label
                            .entry(label.label.clone())
                            .or_default()
                            .push(uid);
                        nodes.push(SynthNode {
                            labels: vec![label.label.clone()],
                            props,
                        });
                    }
                    applied.push("orphans: 3 nodes with no relationships".to_string());
                } else {
                    notes.push("orphans: skipped (no labels profiled)".to_string());
                }
            }
            EdgeCase::Cycle => {
                let cyclic = profile
                    .rel_types
                    .iter()
                    .find(|r| r.endpoints.iter().any(|e| e.source_label == e.target_label));
                match cyclic {
                    Some(rel_profile) => {
                        let label = rel_profile
                            .endpoints
                            .iter()
                            .find(|e| e.source_label == e.target_label)
                            .map(|e| e.source_label.clone())
                            .unwrap();
                        let mut cycle_uids = Vec::new();
                        for i in 0..3 {
                            let uid = format!("SYNTH:{}:cycle{}", label, i);
                            let mut props = BTreeMap::new();
                            props.insert("uid".to_string(), serde_json::Value::String(uid.clone()));
                            props.insert("_synthetic".to_string(), serde_json::Value::Bool(true));
                            uids_by_label
                                .entry(label.clone())
                                .or_default()
                                .push(uid.clone());
                            nodes.push(SynthNode {
                                labels: vec![label.clone()],
                                props,
                            });
                            cycle_uids.push(uid);
                        }
                        for i in 0..3 {
                            rels.push(SynthRel {
                                rel_type: rel_profile.rel_type.clone(),
                                source_uid: cycle_uids[i].clone(),
                                target_uid: cycle_uids[(i + 1) % 3].clone(),
                                props: BTreeMap::from([(
                                    "_synthetic".to_string(),
                                    serde_json::Value::Bool(true),
                                )]),
                            });
                        }
                        applied.push(format!(
                            "cycle: 3-node '{}' loop on '{}'",
                            rel_profile.rel_type, label
                        ));
                    }
                    None => notes.push(
                        "cycle: skipped (no self-referential relationship type sampled)"
                            .to_string(),
                    ),
                }
            }
            EdgeCase::InvalidValues => {
                let target = profile.labels.iter().find_map(|l| {
                    l.properties
                        .iter()
                        .find(|p| p.data_type == "integer" || p.data_type == "float")
                        .map(|p| (l.label.clone(), p.name.clone()))
                });
                match target {
                    Some((label, prop)) => {
                        let uid = format!("SYNTH:{}:invalid", label);
                        let mut props = BTreeMap::new();
                        props.insert("uid".to_string(), serde_json::Value::String(uid.clone()));
                        props.insert("_synthetic".to_string(), serde_json::Value::Bool(true));
                        props.insert(
                            prop.clone(),
                            serde_json::Value::String("NOT_A_NUMBER".to_string()),
                        );
                        uids_by_label.entry(label.clone()).or_default().push(uid);
                        nodes.push(SynthNode {
                            labels: vec![label.clone()],
                            props,
                        });
                        applied.push(format!(
                            "invalid_values: string injected into numeric '{}.{}'",
                            label, prop
                        ));
                    }
                    None => notes
                        .push("invalid_values: skipped (no numeric property sampled)".to_string()),
                }
            }
        }
    }

    let mut per_label: BTreeMap<String, u64> = BTreeMap::new();
    for node in &nodes {
        if let Some(label) = node.labels.first() {
            *per_label.entry(label.clone()).or_insert(0) += 1;
        }
    }
    let mut per_type: BTreeMap<String, u64> = BTreeMap::new();
    for rel in &rels {
        *per_type.entry(rel.rel_type.clone()).or_insert(0) += 1;
    }

    Ok(SynthDataset {
        stats: SynthStats {
            node_count: nodes.len(),
            rel_count: rels.len(),
            per_label,
            per_type,
            edge_cases_applied: applied,
        },
        nodes,
        rels,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{GraphProfile, LabelProfile, PropertyProfile, RelProfile, RelSample};

    fn test_profile() -> GraphProfile {
        GraphProfile {
            labels: vec![
                LabelProfile {
                    label: "Person".to_string(),
                    count: 100,
                    properties: vec![
                        PropertyProfile {
                            name: "name".to_string(),
                            data_type: "string".to_string(),
                            coverage: 1.0,
                        },
                        PropertyProfile {
                            name: "age".to_string(),
                            data_type: "integer".to_string(),
                            coverage: 0.5,
                        },
                    ],
                },
                LabelProfile {
                    label: "Company".to_string(),
                    count: 10,
                    properties: vec![PropertyProfile {
                        name: "name".to_string(),
                        data_type: "string".to_string(),
                        coverage: 1.0,
                    }],
                },
            ],
            rel_types: vec![RelProfile {
                rel_type: "WORKS_AT".to_string(),
                count: 80,
                endpoints: vec![RelSample {
                    source_label: "Person".to_string(),
                    target_label: "Company".to_string(),
                }],
                properties: Vec::new(),
            }],
            total_nodes: 110,
            total_relationships: 80,
        }
    }

    #[test]
    fn test_generation_is_deterministic() {
        let profile = test_profile();
        let config = SynthConfig {
            scale: 0.5,
            seed: 42,
            edge_cases: vec![EdgeCase::Duplicates, EdgeCase::Orphans],
            max_nodes: 50_000,
        };
        let a = generate_dataset(&profile, &config).unwrap();
        let b = generate_dataset(&profile, &config).unwrap();
        assert_eq!(
            serde_json::to_string(&a.nodes).unwrap(),
            serde_json::to_string(&b.nodes).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&a.rels).unwrap(),
            serde_json::to_string(&b.rels).unwrap()
        );
    }

    #[test]
    fn test_scale_and_distributions() {
        let profile = test_profile();
        let config = SynthConfig {
            scale: 0.5,
            seed: 7,
            ..Default::default()
        };
        let dataset = generate_dataset(&profile, &config).unwrap();
        assert_eq!(dataset.stats.per_label["Person"], 50);
        assert_eq!(dataset.stats.per_label["Company"], 5);
        assert_eq!(dataset.stats.per_type["WORKS_AT"], 40);
        // Every relationship endpoint exists.
        let uids: std::collections::HashSet<String> = dataset
            .nodes
            .iter()
            .filter_map(|n| {
                n.props
                    .get("uid")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        for rel in &dataset.rels {
            assert!(uids.contains(&rel.source_uid));
            assert!(uids.contains(&rel.target_uid));
        }
    }

    #[test]
    fn test_edge_cases() {
        let profile = test_profile();
        let config = SynthConfig {
            scale: 0.1,
            seed: 1,
            edge_cases: vec![
                EdgeCase::Duplicates,
                EdgeCase::Orphans,
                EdgeCase::Cycle,
                EdgeCase::InvalidValues,
            ],
            max_nodes: 50_000,
        };
        let dataset = generate_dataset(&profile, &config).unwrap();
        // Cycle skipped: WORKS_AT is not self-referential.
        assert!(dataset.notes.iter().any(|n| n.contains("cycle: skipped")));
        assert_eq!(dataset.stats.edge_cases_applied.len(), 3);
        assert!(dataset.nodes.iter().any(|n| n
            .props
            .get("uid")
            .and_then(|v| v.as_str())
            .map(|s| s.contains(":dup"))
            .unwrap_or(false)));
    }

    #[test]
    fn test_invalid_config_rejected() {
        let profile = test_profile();
        assert!(generate_dataset(
            &profile,
            &SynthConfig {
                scale: 2.0,
                ..Default::default()
            }
        )
        .is_err());
        assert!(generate_dataset(
            &profile,
            &SynthConfig {
                max_nodes: 0,
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn test_max_nodes_cap_is_honest() {
        let profile = test_profile();
        let config = SynthConfig {
            scale: 1.0,
            seed: 1,
            max_nodes: 10,
            ..Default::default()
        };
        let dataset = generate_dataset(&profile, &config).unwrap();
        assert!(dataset.nodes.len() <= 10);
        assert!(dataset.notes.iter().any(|n| n.contains("max_nodes")));
    }
}
