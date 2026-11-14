use crate::error::IngestError;
use relating_core::{ExtractedNode, ExtractedRelationship};
use relating_graph::{GraphBackend, GraphWriteResult};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

pub fn load_extracted_to_graph(
    graph_backend: &dyn GraphBackend,
    store_path: &Path,
    nodes: &[ExtractedNode],
    relationships: &[ExtractedRelationship],
) -> Result<GraphWriteResult, IngestError> {
    graph_backend.ensure_store(store_path)?;

    let mut total_nodes_created = 0;
    let mut total_rels_created = 0;
    let mut total_props_set = 0;
    let start_time = std::time::Instant::now();

    // 1. Insert/Merge nodes
    for node in nodes {
        let label = node.labels.first().cloned().unwrap_or_else(|| "Entity".to_string());
        let cypher = format!(
            "MERGE (n:{} {{uid: $uid}}) SET n += $props",
            label
        );

        let mut params = HashMap::new();
        params.insert("uid".to_string(), serde_json::Value::String(node.id.clone()));
        params.insert(
            "props".to_string(),
            serde_json::Value::Object(
                node.properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );

        let res = graph_backend.execute_write(store_path, &cypher, params)?;
        total_nodes_created += res.nodes_created;
        total_props_set += res.properties_set;
    }

    // 2. Insert/Merge relationships
    for rel in relationships {
        let cypher = format!(
            "MATCH (s {{uid: $source_uid}}), (t {{uid: $target_uid}}) MERGE (s)-[r:{}]->(t) SET r += $props",
            rel.rel_type
        );

        let mut params = HashMap::new();
        params.insert(
            "source_uid".to_string(),
            serde_json::Value::String(rel.source_node_id.clone()),
        );
        params.insert(
            "target_uid".to_string(),
            serde_json::Value::String(rel.target_node_id.clone()),
        );
        params.insert(
            "props".to_string(),
            serde_json::Value::Object(
                rel.properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );

        let res = graph_backend.execute_write(store_path, &cypher, params)?;
        total_rels_created += res.relationships_created;
        total_props_set += res.properties_set;
    }

    let elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
    info!(
        "Graph load finished: {} nodes created, {} rels created in {:.2}ms",
        total_nodes_created, total_rels_created, elapsed_ms
    );

    Ok(GraphWriteResult {
        nodes_created: total_nodes_created,
        nodes_deleted: 0,
        relationships_created: total_rels_created,
        relationships_deleted: 0,
        properties_set: total_props_set,
        elapsed_ms,
    })
}
