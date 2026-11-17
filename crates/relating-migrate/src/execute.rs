use crate::backend::Neo4jBackend;
use crate::error::MigrateError;
use crate::packstream::BoltValue;
use crate::plan::escape_identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutcome {
    pub attempted: usize,
    pub nodes_created: i64,
    pub relationships_created: i64,
    pub properties_set: i64,
    pub elapsed_ms: f64,
}

fn counter(meta: &BTreeMap<String, BoltValue>, key: &str) -> i64 {
    match meta.get("stats") {
        Some(BoltValue::Map(stats)) => stats.get(key).and_then(|v| v.as_int()).unwrap_or(0),
        _ => meta.get(key).and_then(|v| v.as_int()).unwrap_or(0),
    }
}

fn rows_param(rows: Vec<BTreeMap<String, BoltValue>>) -> BTreeMap<String, BoltValue> {
    let mut params = BTreeMap::new();
    params.insert(
        "batch".to_string(),
        BoltValue::List(rows.into_iter().map(BoltValue::Map).collect()),
    );
    params
}

/// Idempotent node batch: MERGE on the stable uid, then set properties.
/// Safe to replay after an interruption.
pub async fn migrate_node_batch(
    dest: &dyn Neo4jBackend,
    label: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
) -> Result<BatchOutcome, MigrateError> {
    let start = std::time::Instant::now();
    let attempted = rows.len();
    if attempted == 0 {
        return Ok(BatchOutcome {
            attempted: 0,
            nodes_created: 0,
            relationships_created: 0,
            properties_set: 0,
            elapsed_ms: 0.0,
        });
    }
    let query = format!(
        "UNWIND $batch AS row MERGE (n:{} {{uid: row.uid}}) SET n += row.props",
        escape_identifier(label)
    );
    let meta = dest.run_write(&query, rows_param(rows)).await?;
    Ok(BatchOutcome {
        attempted,
        nodes_created: counter(&meta, "nodes-created"),
        relationships_created: 0,
        properties_set: counter(&meta, "properties-set"),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Idempotent relationship batch: endpoints must already exist (matched by
/// uid, never fabricated); the relationship itself is MERGEd.
pub async fn migrate_relationship_batch(
    dest: &dyn Neo4jBackend,
    rel_type: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
) -> Result<BatchOutcome, MigrateError> {
    let start = std::time::Instant::now();
    let attempted = rows.len();
    if attempted == 0 {
        return Ok(BatchOutcome {
            attempted: 0,
            nodes_created: 0,
            relationships_created: 0,
            properties_set: 0,
            elapsed_ms: 0.0,
        });
    }
    let query = format!(
        "UNWIND $batch AS row MATCH (s {{uid: row.suid}}), (t {{uid: row.tuid}}) MERGE (s)-[r:{}]->(t) SET r += row.props",
        escape_identifier(rel_type)
    );
    let meta = dest.run_write(&query, rows_param(rows)).await?;
    Ok(BatchOutcome {
        attempted,
        nodes_created: 0,
        relationships_created: counter(&meta, "relationships-created"),
        properties_set: counter(&meta, "properties-set"),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockNeo4jBackend;

    fn node_row(uid: &str, name: &str) -> BTreeMap<String, BoltValue> {
        let mut props = BTreeMap::new();
        props.insert("name".to_string(), BoltValue::String(name.to_string()));
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String(uid.to_string()));
        row.insert("props".to_string(), BoltValue::Map(props));
        row
    }

    #[tokio::test]
    async fn test_node_batch_is_idempotent() {
        let dest = MockNeo4jBackend::new();
        let rows = vec![node_row("Person:1", "Ada"), node_row("Person:2", "Bob")];
        let first = migrate_node_batch(&dest, "Person", rows.clone())
            .await
            .unwrap();
        assert_eq!(first.attempted, 2);
        assert_eq!(first.nodes_created, 2);

        let replay = migrate_node_batch(&dest, "Person", rows).await.unwrap();
        assert_eq!(replay.nodes_created, 0);
        assert_eq!(dest.write_count().await, 2);
    }

    #[tokio::test]
    async fn test_empty_batch_is_noop() {
        let dest = MockNeo4jBackend::new();
        let outcome = migrate_node_batch(&dest, "Person", Vec::new())
            .await
            .unwrap();
        assert_eq!(outcome.attempted, 0);
        assert_eq!(dest.write_count().await, 0);
    }

    #[tokio::test]
    async fn test_relationship_batch_counts() {
        let dest = MockNeo4jBackend::new();
        let mut row = BTreeMap::new();
        row.insert(
            "suid".to_string(),
            BoltValue::String("Person:1".to_string()),
        );
        row.insert(
            "tuid".to_string(),
            BoltValue::String("Person:2".to_string()),
        );
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
        let outcome = migrate_relationship_batch(&dest, "KNOWS", vec![row])
            .await
            .unwrap();
        assert_eq!(outcome.relationships_created, 1);
    }
}
