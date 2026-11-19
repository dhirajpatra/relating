use crate::backend::Neo4jBackend;
use crate::error::MigrateError;
use crate::packstream::{bolt_to_json, json_to_bolt, BoltValue};
use relating_graph::{GraphQueryResult, GraphWriteResult};
use std::collections::{BTreeMap, HashMap};

/// Append a LIMIT bound when the caller asked for one and the statement does
/// not already carry a LIMIT clause. Same rule as the local Relate backend.
pub fn apply_proxy_limit(cypher: &str, limit: Option<u32>) -> String {
    let mut out = cypher.trim().to_string();
    if let Some(lim) = limit {
        if !out.to_lowercase().contains("limit ") {
            out.push_str(&format!(" LIMIT {}", lim));
        }
    }
    out
}

fn to_bolt_params(params: HashMap<String, serde_json::Value>) -> BTreeMap<String, BoltValue> {
    params
        .into_iter()
        .map(|(k, v)| (k, json_to_bolt(&v)))
        .collect()
}

/// Run one read-only-shaped statement against an Aura/Neo4j target and return
/// it in the exact shape of a local [`GraphQueryResult`].
///
/// This is the backend flip: application code keeps its Cypher text and bound
/// parameters and only changes which target the call is routed to. Values are
/// always bound as parameters, never interpolated.
pub async fn proxied_read(
    backend: &dyn Neo4jBackend,
    cypher: &str,
    params: HashMap<String, serde_json::Value>,
    limit: Option<u32>,
) -> Result<GraphQueryResult, MigrateError> {
    let final_cypher = apply_proxy_limit(cypher, limit);
    let start = std::time::Instant::now();
    let records = backend
        .run_query(&final_cypher, to_bolt_params(params))
        .await?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    let mut rows = Vec::with_capacity(records.rows.len());
    for row in records.rows {
        let mut row_map = HashMap::new();
        for (i, value) in row.into_iter().enumerate() {
            let name = records
                .columns
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("col_{}", i));
            row_map.insert(name, bolt_to_json(&value));
        }
        rows.push(row_map);
    }

    Ok(GraphQueryResult {
        columns: records.columns,
        row_count: rows.len(),
        rows,
        elapsed_ms,
    })
}

fn write_counter(meta: &BTreeMap<String, BoltValue>, key: &str) -> i64 {
    match meta.get("stats") {
        Some(BoltValue::Map(stats)) => stats.get(key).and_then(|v| v.as_int()).unwrap_or(0),
        _ => meta.get(key).and_then(|v| v.as_int()).unwrap_or(0),
    }
}

/// Shape a remote write's SUCCESS metadata as a local [`GraphWriteResult`].
/// Accepts both the nested `stats` map and flat counters.
pub fn summarize_write_meta(
    meta: &BTreeMap<String, BoltValue>,
    elapsed_ms: f64,
) -> GraphWriteResult {
    GraphWriteResult {
        nodes_created: write_counter(meta, "nodes-created"),
        nodes_deleted: write_counter(meta, "nodes-deleted"),
        relationships_created: write_counter(meta, "relationships-created"),
        relationships_deleted: write_counter(meta, "relationships-deleted"),
        properties_set: write_counter(meta, "properties-set"),
        elapsed_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockNeo4jBackend;

    #[test]
    fn test_apply_proxy_limit_matches_local_rule() {
        assert_eq!(
            apply_proxy_limit("MATCH (n) RETURN n", Some(10)),
            "MATCH (n) RETURN n LIMIT 10"
        );
        assert_eq!(
            apply_proxy_limit("MATCH (n) RETURN n LIMIT 5", Some(10)),
            "MATCH (n) RETURN n LIMIT 5"
        );
        assert_eq!(
            apply_proxy_limit("MATCH (n) RETURN n", None),
            "MATCH (n) RETURN n"
        );
    }

    #[tokio::test]
    async fn test_proxied_read_shapes_rows_like_local() {
        let backend = MockNeo4jBackend::new();
        backend
            .script_columns("RETURN n", vec!["name".to_string(), "age".to_string()])
            .await;
        backend
            .script_read(
                "RETURN n",
                vec![
                    vec![BoltValue::String("Ada".to_string()), BoltValue::Int(36)],
                    vec![BoltValue::String("Bob".to_string()), BoltValue::Int(41)],
                ],
            )
            .await;

        let result = proxied_read(
            &backend,
            "MATCH (n:Person) RETURN n",
            HashMap::from([("label".to_string(), serde_json::json!("Person"))]),
            Some(100),
        )
        .await
        .unwrap();

        assert_eq!(result.columns, vec!["name", "age"]);
        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0]["name"], "Ada");
        assert_eq!(result.rows[0]["age"], 36);
        assert_eq!(result.rows[1]["name"], "Bob");
    }

    #[test]
    fn test_summarize_write_meta_reads_nested_stats() {
        let mut stats = BTreeMap::new();
        stats.insert("nodes-created".to_string(), BoltValue::Int(2));
        stats.insert("properties-set".to_string(), BoltValue::Int(7));
        let mut meta = BTreeMap::new();
        meta.insert("stats".to_string(), BoltValue::Map(stats));
        let summary = summarize_write_meta(&meta, 12.5);
        assert_eq!(summary.nodes_created, 2);
        assert_eq!(summary.properties_set, 7);
        assert_eq!(summary.relationships_created, 0);
        assert_eq!(summary.elapsed_ms, 12.5);
    }

    #[tokio::test]
    async fn test_proxied_read_synthesizes_columns_when_unnamed() {
        let backend = MockNeo4jBackend::new();
        backend
            .script_read("RETURN 1", vec![vec![BoltValue::Int(1)]])
            .await;
        let result = proxied_read(&backend, "RETURN 1", HashMap::new(), None)
            .await
            .unwrap();
        assert_eq!(result.columns, vec!["col_0"]);
        assert_eq!(result.rows[0]["col_0"], 1);
    }
}
