use crate::error::GraphError;
use crate::schema::{
    CytoscapeEdge, CytoscapeEdgeData, CytoscapeElements, CytoscapeGraph, CytoscapeNode,
    CytoscapeNodeData, GraphQueryResult, GraphSchemaInfo, GraphWriteResult,
};
use crate::value_converter::{json_to_relate, relate_to_json};
use relate::{Database, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tracing::debug;

pub trait GraphBackend: Send + Sync {
    fn ensure_store(&self, store_path: &Path) -> Result<(), GraphError>;
    fn execute_read(
        &self,
        store_path: &Path,
        cypher: &str,
        params: HashMap<String, serde_json::Value>,
        limit: Option<u32>,
    ) -> Result<GraphQueryResult, GraphError>;
    fn execute_write(
        &self,
        store_path: &Path,
        cypher: &str,
        params: HashMap<String, serde_json::Value>,
    ) -> Result<GraphWriteResult, GraphError>;
    fn get_schema(&self, store_path: &Path) -> Result<GraphSchemaInfo, GraphError>;
    fn get_cytoscape_elements(
        &self,
        store_path: &Path,
        limit: Option<u32>,
    ) -> Result<CytoscapeGraph, GraphError>;
}

#[derive(Clone, Default)]
pub struct RelateBackend;

impl RelateBackend {
    pub fn new() -> Self {
        Self
    }

    fn to_relate_params(params: HashMap<String, serde_json::Value>) -> BTreeMap<String, Value> {
        let mut map = BTreeMap::new();
        for (k, v) in params {
            map.insert(k, json_to_relate(v));
        }
        map
    }

    fn ensure_parent_dir(path: &Path) -> Result<(), GraphError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    GraphError::Storage(format!("Failed to create graph store directory: {}", e))
                })?;
            }
        }
        Ok(())
    }
}

impl GraphBackend for RelateBackend {
    fn ensure_store(&self, store_path: &Path) -> Result<(), GraphError> {
        Self::ensure_parent_dir(store_path)?;
        let uri = store_path
            .to_str()
            .ok_or_else(|| GraphError::Storage("Invalid non-UTF8 path".to_string()))?;

        // Opening in writable mode creates the store if missing
        let db = Database::open(uri)?;
        let _conn = db.connect()?;
        debug!("Verified graph store exists at: {}", uri);
        Ok(())
    }

    fn execute_read(
        &self,
        store_path: &Path,
        cypher: &str,
        params: HashMap<String, serde_json::Value>,
        limit: Option<u32>,
    ) -> Result<GraphQueryResult, GraphError> {
        let uri = store_path
            .to_str()
            .ok_or_else(|| GraphError::Storage("Invalid non-UTF8 path".to_string()))?;

        if !store_path.exists() {
            return Err(GraphError::StoreNotFound(format!(
                "Store does not exist at {}",
                uri
            )));
        }

        // Apply read-only mode at engine level
        let db = Database::open_read_only(uri)?;
        let mut conn = db.connect()?;

        // Bound query with LIMIT if requested and not present
        let mut final_cypher = cypher.trim().to_string();
        if let Some(lim) = limit {
            let lower = final_cypher.to_lowercase();
            if !lower.contains("limit ") {
                final_cypher.push_str(&format!(" LIMIT {}", lim));
            }
        }

        let relate_params = Self::to_relate_params(params);
        let start_time = std::time::Instant::now();
        let query_result = conn.query_with_params(&final_cypher, relate_params)?;
        let elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;

        let columns = query_result.header.columns.clone();
        let mut rows = Vec::with_capacity(query_result.rows.len());

        for row in query_result.rows {
            let mut row_map = HashMap::new();
            for (i, val) in row.into_iter().enumerate() {
                if let Some(col_name) = columns.get(i) {
                    row_map.insert(col_name.clone(), relate_to_json(&val));
                }
            }
            rows.push(row_map);
        }

        let row_count = rows.len();
        Ok(GraphQueryResult {
            columns,
            rows,
            elapsed_ms,
            row_count,
        })
    }

    fn execute_write(
        &self,
        store_path: &Path,
        cypher: &str,
        params: HashMap<String, serde_json::Value>,
    ) -> Result<GraphWriteResult, GraphError> {
        Self::ensure_parent_dir(store_path)?;
        let uri = store_path
            .to_str()
            .ok_or_else(|| GraphError::Storage("Invalid non-UTF8 path".to_string()))?;

        let db = Database::open(uri)?;
        let mut conn = db.connect()?;

        let relate_params = Self::to_relate_params(params);
        let start_time = std::time::Instant::now();
        let query_result = conn.query_with_params(cypher, relate_params)?;
        let elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;

        Ok(GraphWriteResult {
            nodes_created: query_result.stats.nodes_created,
            nodes_deleted: query_result.stats.nodes_deleted,
            relationships_created: query_result.stats.relationships_created,
            relationships_deleted: query_result.stats.relationships_deleted,
            properties_set: query_result.stats.properties_set,
            elapsed_ms,
        })
    }

    fn get_schema(&self, store_path: &Path) -> Result<GraphSchemaInfo, GraphError> {
        let uri = store_path
            .to_str()
            .ok_or_else(|| GraphError::Storage("Invalid non-UTF8 path".to_string()))?;

        if !store_path.exists() {
            return Ok(GraphSchemaInfo::default());
        }

        let db = Database::open_read_only(uri)?;
        let mut conn = db.connect()?;

        // Count nodes
        let node_count = match conn.query("MATCH (n) RETURN count(n) AS c") {
            Ok(res) => {
                if let Some(first_row) = res.rows.first() {
                    match first_row.first() {
                        Some(Value::Int(c)) => *c as u64,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            Err(_) => 0,
        };

        // Count relationships
        let relationship_count = match conn.query("MATCH ()-[r]->() RETURN count(r) AS c") {
            Ok(res) => {
                if let Some(first_row) = res.rows.first() {
                    match first_row.first() {
                        Some(Value::Int(c)) => *c as u64,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            Err(_) => 0,
        };

        // Get labels
        let mut labels = Vec::new();
        if let Ok(res) = conn.query("MATCH (n) UNWIND labels(n) AS l RETURN DISTINCT l AS label LIMIT 100") {
            for row in res.rows {
                if let Some(Value::String(s)) = row.first() {
                    labels.push(s.to_string());
                }
            }
        }

        // Get relationship types
        let mut relationship_types = Vec::new();
        if let Ok(res) = conn.query("MATCH ()-[r]->() RETURN DISTINCT type(r) AS rel_type LIMIT 100") {
            for row in res.rows {
                if let Some(Value::String(s)) = row.first() {
                    relationship_types.push(s.to_string());
                }
            }
        }

        Ok(GraphSchemaInfo {
            labels,
            relationship_types,
            node_count,
            relationship_count,
            property_keys: Vec::new(),
        })
    }

    fn get_cytoscape_elements(
        &self,
        store_path: &Path,
        limit: Option<u32>,
    ) -> Result<CytoscapeGraph, GraphError> {
        let uri = store_path
            .to_str()
            .ok_or_else(|| GraphError::Storage("Invalid non-UTF8 path".to_string()))?;

        if !store_path.exists() {
            return Ok(CytoscapeGraph::default());
        }

        let db = Database::open_read_only(uri)?;
        let mut conn = db.connect()?;

        let node_limit = limit.unwrap_or(300);
        let edge_limit = limit.unwrap_or(500);

        // Fetch nodes
        let node_query = format!("MATCH (n) RETURN n LIMIT {}", node_limit);
        let mut nodes = Vec::new();
        if let Ok(res) = conn.query(&node_query) {
            for row in res.rows {
                if let Some(Value::Node(node)) = row.first() {
                    let id_str = node.id.to_string();
                    let label = node.labels.first().map(|l| l.to_string()).unwrap_or_else(|| "Node".to_string());

                    let mut props = HashMap::new();
                    for (k, v) in node.properties.iter() {
                        props.insert(k.clone(), relate_to_json(v));
                    }

                    let name = props
                        .get("name")
                        .and_then(|v| v.as_str())
                        .or_else(|| props.get("title").and_then(|v| v.as_str()))
                        .unwrap_or(&label)
                        .to_string();

                    nodes.push(CytoscapeNode {
                        data: CytoscapeNodeData {
                            id: id_str,
                            label,
                            name,
                            properties: props,
                        },
                    });
                }
            }
        }

        // Fetch relationships
        let edge_query = format!("MATCH (s)-[r]->(t) RETURN r LIMIT {}", edge_limit);
        let mut edges = Vec::new();
        if let Ok(res) = conn.query(&edge_query) {
            for row in res.rows {
                if let Some(Value::Relationship(rel)) = row.first() {
                    let id_str = rel.id.to_string();
                    let source_str = rel.start.to_string();
                    let target_str = rel.end.to_string();
                    let label = rel.rel_type.to_string();

                    let mut props = HashMap::new();
                    for (k, v) in rel.properties.iter() {
                        props.insert(k.clone(), relate_to_json(v));
                    }

                    edges.push(CytoscapeEdge {
                        data: CytoscapeEdgeData {
                            id: id_str,
                            source: source_str,
                            target: target_str,
                            label,
                            properties: props,
                        },
                    });
                }
            }
        }

        Ok(CytoscapeGraph {
            elements: CytoscapeElements { nodes, edges },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_relate_backend_isolated_stores() {
        let dir = tempdir().unwrap();
        let store_a = dir.path().join("proj_a").join("graph.rl");
        let store_b = dir.path().join("proj_b").join("graph.rl");

        let backend = RelateBackend::new();
        backend.ensure_store(&store_a).unwrap();
        backend.ensure_store(&store_b).unwrap();

        // Write node in Store A
        let write_a = backend.execute_write(
            &store_a,
            "CREATE (n:Person {name: $name, age: $age})",
            HashMap::from([
                ("name".to_string(), serde_json::json!("Alice")),
                ("age".to_string(), serde_json::json!(30)),
            ]),
        ).unwrap();
        assert_eq!(write_a.nodes_created, 1);

        // Read from Store A
        let read_a = backend.execute_read(
            &store_a,
            "MATCH (n:Person) RETURN n.name AS name, n.age AS age",
            HashMap::new(),
            None,
        ).unwrap();
        assert_eq!(read_a.row_count, 1);
        assert_eq!(read_a.rows[0]["name"], "Alice");
        assert_eq!(read_a.rows[0]["age"], 30);

        // Verify Store B has 0 nodes (complete multi-tenant project isolation)
        let schema_b = backend.get_schema(&store_b).unwrap();
        assert_eq!(schema_b.node_count, 0);

        let read_b = backend.execute_read(
            &store_b,
            "MATCH (n) RETURN n",
            HashMap::new(),
            None,
        ).unwrap();
        assert_eq!(read_b.row_count, 0);
    }

    #[test]
    fn test_read_only_violation() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");

        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();

        // Attempting to write inside execute_read must fail because store is opened read-only
        let res = backend.execute_read(
            &store,
            "CREATE (n:Malicious)",
            HashMap::new(),
            None,
        );
        assert!(res.is_err(), "Write in read-only query must fail");
    }
}
