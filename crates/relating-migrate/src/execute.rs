use crate::backend::Neo4jBackend;
use crate::error::MigrateError;
use crate::packstream::BoltValue;
use crate::plan::escape_identifier;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutcome {
    pub attempted: usize,
    pub nodes_created: i64,
    pub relationships_created: i64,
    pub properties_set: i64,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackOutcome {
    pub migration_id: String,
    pub nodes_deleted: i64,
    pub relationships_deleted: i64,
}

/// Property stamped (ON CREATE only) on every node and relationship a
/// migration creates, so `rollback_migration` can remove exactly that
/// migration's additions. Pre-existing target records matched by MERGE are
/// updated in place but never tagged, so rollback never deletes data the
/// migration did not create. Property updates on pre-existing records are
/// NOT reverted by rollback: snapshot the Aura instance first when the tier
/// supports it (see docs/api/migration.md).
pub const MIGRATION_TAG_PROPERTY: &str = "_relating_migration_id";

/// Query parameter carrying the migration id for ON CREATE tagging.
const MID_PARAM: &str = "mid";

/// Optimistic-concurrency version stamped on every migrated record.
/// Sources may carry their own `_version`; otherwise the server injects `1`.
pub const VERSION_PROPERTY: &str = "_version";
/// Last-write timestamp (RFC 3339) stamped alongside the version.
pub const UPDATED_AT_PROPERTY: &str = "_updated_at";

/// Stable business keys recognized inside relationship properties for CDC.
/// When the source exposes no record id, one of these keeps the edge uid
/// stable across property updates (unlike the legacy content hash).
const STABLE_REL_KEYS: &[&str] = &[
    "uid",
    "relationship_uid",
    "edge_uid",
    "id",
    "_uid",
    "key",
    "uuid",
];

fn stable_key_from_props(props_json: &serde_json::Value) -> Option<String> {
    let map = props_json.as_object()?;
    for key in STABLE_REL_KEYS {
        if let Some(v) = map.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
        // Numeric stable keys (e.g. source PKs) stringify deterministically.
        if let Some(v) = map.get(*key).and_then(|v| v.as_i64()) {
            return Some(v.to_string());
        }
    }
    None
}

/// True when a relationship uid came from the legacy content-hash fallback
/// (not update-stable: property changes re-hash to a new uid).
pub fn is_hash_fallback_uid(uid: &str) -> bool {
    uid.contains("#hash:")
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

fn rows_param_with_mid(
    rows: Vec<BTreeMap<String, BoltValue>>,
    migration_id: &str,
) -> BTreeMap<String, BoltValue> {
    let mut params = rows_param(rows);
    params.insert(
        MID_PARAM.to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    params
}

/// Stable relationship key with CDC-safe precedence:
/// 1. source record id (`rel:<n>`) — update-stable,
/// 2. stable business key inside properties (`key:<v>`) — update-stable,
/// 3. legacy content hash (`hash:<hex>`) — NOT update-stable (property
///    changes re-hash to a new uid, creating a duplicate on delta sync).
///
/// The prefix keeps the three provenances visibly apart in target data.
/// Prefer (1) or (2) for any edge that must survive incremental sync.
pub fn relationship_uid(
    rel_type: &str,
    suid: &str,
    tuid: &str,
    source_rel_id: Option<&str>,
    props_json: &serde_json::Value,
) -> String {
    if let Some(id) = source_rel_id.filter(|id| !id.is_empty()) {
        return format!("{}:{}->{}#rel:{}", rel_type, suid, tuid, id);
    }
    if let Some(key) = stable_key_from_props(props_json) {
        return format!("{}:{}->{}#key:{}", rel_type, suid, tuid, key);
    }
    let mut hasher = Sha256::new();
    hasher.update(rel_type.as_bytes());
    hasher.update([0]);
    hasher.update(suid.as_bytes());
    hasher.update([0]);
    hasher.update(tuid.as_bytes());
    hasher.update([0]);
    hasher.update(props_json.to_string().as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}:{}->{}#hash:{}", rel_type, suid, tuid, hex)
}

/// Idempotent node batch: MERGE on the stable uid, then set properties.
/// Safe to replay after an interruption.
pub async fn migrate_node_batch(
    dest: &dyn Neo4jBackend,
    label: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
) -> Result<BatchOutcome, MigrateError> {
    migrate_node_batch_with_options(dest, label, rows, None).await
}

/// Node batch with rollback tagging: created nodes are stamped with the
/// migration id (ON CREATE only — matched pre-existing nodes stay untagged).
pub async fn migrate_node_batch_with_options(
    dest: &dyn Neo4jBackend,
    label: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
    migration_id: Option<&str>,
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
    let version_guard = format!(
        "coalesce(row.props.{}, 1) > coalesce(n.{}, 0)",
        escape_identifier(VERSION_PROPERTY),
        escape_identifier(VERSION_PROPERTY)
    );
    let create_tag = match migration_id {
        Some(_) => format!(
            ", n.{} = ${}",
            escape_identifier(MIGRATION_TAG_PROPERTY),
            MID_PARAM
        ),
        None => String::new(),
    };
    let query = format!(
        "UNWIND $batch AS row MERGE (n:{} {{uid: row.uid}}) ON CREATE SET n += row.props{} ON MATCH SET n += CASE WHEN {} THEN row.props ELSE {{}} END",
        escape_identifier(label),
        create_tag,
        version_guard,
    );
    let params = match migration_id {
        Some(mid) => rows_param_with_mid(rows, mid),
        None => rows_param(rows),
    };
    let meta = dest.run_write(&query, params).await?;
    Ok(BatchOutcome {
        attempted,
        nodes_created: counter(&meta, "nodes-created"),
        relationships_created: 0,
        properties_set: counter(&meta, "properties-set"),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Idempotent relationship batch: endpoints must already exist (matched by
/// uid, never fabricated); each relationship is MERGEd by its own stable
/// uid so parallel edges between the same endpoints never collapse into one.
///
/// Every row must carry a `uid` (see [`relationship_uid`]); rows without
/// one fail the batch fast instead of merging unpredictably.
pub async fn migrate_relationship_batch(
    dest: &dyn Neo4jBackend,
    rel_type: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
) -> Result<BatchOutcome, MigrateError> {
    migrate_relationship_batch_with_options(dest, rel_type, rows, None).await
}

/// Relationship batch with rollback tagging (see [`migrate_node_batch_with_options`]).
pub async fn migrate_relationship_batch_with_options(
    dest: &dyn Neo4jBackend,
    rel_type: &str,
    rows: Vec<BTreeMap<String, BoltValue>>,
    migration_id: Option<&str>,
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
    for (index, row) in rows.iter().enumerate() {
        match row.get("uid").and_then(|v| v.as_str()) {
            Some(uid) if !uid.is_empty() => {}
            _ => {
                return Err(MigrateError::Plan(format!(
                    "Relationship batch '{}' row {} is missing its stable uid merge key; \
                     refusing to MERGE without a key (parallel edges would collapse)",
                    rel_type, index
                )));
            }
        }
    }
    let version_guard = format!(
        "coalesce(row.props.{}, 1) > coalesce(r.{}, 0)",
        escape_identifier(VERSION_PROPERTY),
        escape_identifier(VERSION_PROPERTY)
    );
    let create_tag = match migration_id {
        Some(_) => format!(
            ", r.{} = ${}",
            escape_identifier(MIGRATION_TAG_PROPERTY),
            MID_PARAM
        ),
        None => String::new(),
    };
    let query = format!(
        "UNWIND $batch AS row MATCH (s {{uid: row.suid}}), (t {{uid: row.tuid}}) MERGE (s)-[r:{} {{uid: row.uid}}]->(t) ON CREATE SET r += row.props{} ON MATCH SET r += CASE WHEN {} THEN row.props ELSE {{}} END",
        escape_identifier(rel_type),
        create_tag,
        version_guard,
    );
    let params = match migration_id {
        Some(mid) => rows_param_with_mid(rows, mid),
        None => rows_param(rows),
    };
    let meta = dest.run_write(&query, params).await?;
    Ok(BatchOutcome {
        attempted,
        nodes_created: 0,
        relationships_created: counter(&meta, "relationships-created"),
        properties_set: counter(&meta, "properties-set"),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Remove exactly what a tagged migration created: relationships stamped
/// with the migration id first, then stamped nodes (DETACH for safety).
///
/// Only records created by that migration carry the tag, so records the
/// migration merely updated are left untouched — including their updated
/// properties, which rollback cannot unwrite.
pub async fn rollback_migration(
    dest: &dyn Neo4jBackend,
    migration_id: &str,
) -> Result<RollbackOutcome, MigrateError> {
    if migration_id.trim().is_empty() {
        return Err(MigrateError::Plan(
            "Cannot roll back: migration id is empty".to_string(),
        ));
    }
    let mut params = BTreeMap::new();
    params.insert(
        MID_PARAM.to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    let tag = escape_identifier(MIGRATION_TAG_PROPERTY);

    let rel_query = format!("MATCH ()-[r {{{}: ${}}}]->() DELETE r", tag, MID_PARAM);
    let rel_meta = dest.run_write(&rel_query, params.clone()).await?;
    let relationships_deleted = counter(&rel_meta, "relationships-deleted");

    let node_query = format!("MATCH (n {{{}: ${}}}) DETACH DELETE n", tag, MID_PARAM);
    let node_meta = dest.run_write(&node_query, params).await?;
    let nodes_deleted = counter(&node_meta, "nodes-deleted");

    Ok(RollbackOutcome {
        migration_id: migration_id.to_string(),
        nodes_deleted,
        relationships_deleted,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePropagationOutcome {
    pub migration_id: String,
    pub label_or_type: String,
    pub nodes_deleted: i64,
    pub relationships_deleted: i64,
    pub missing_uids: Vec<String>,
}

/// Explicit delete propagation for CDC: remove tagged target records whose
/// uids no longer exist in the source. Merge-upsert never deletes, so delta
/// syncs call this only when the caller opts in (`propagate_deletes=true`).
/// Only records stamped with this migration id are eligible; untagged
/// target-native data is never touched.
pub async fn propagate_node_deletes(
    dest: &dyn Neo4jBackend,
    migration_id: &str,
    label: &str,
    source_uids: &[String],
) -> Result<DeletePropagationOutcome, MigrateError> {
    if migration_id.trim().is_empty() {
        return Err(MigrateError::Plan(
            "Cannot propagate deletes: migration id is empty".to_string(),
        ));
    }
    use std::collections::HashSet;
    let source: HashSet<&str> = source_uids.iter().map(|s| s.as_str()).collect();
    let tag = escape_identifier(MIGRATION_TAG_PROPERTY);
    let list_query = format!(
        "MATCH (n:{} {{{}: $mid}}) RETURN n.uid AS uid",
        escape_identifier(label),
        tag
    );
    let mut list_params = BTreeMap::new();
    list_params.insert(
        "mid".to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    let rows = dest.run_read(&list_query, list_params).await?;
    let mut missing = Vec::new();
    for row in rows {
        if let Some(uid) = row.first().and_then(|v| v.as_str()) {
            if !source.contains(uid) {
                missing.push(uid.to_string());
            }
        }
    }
    missing.sort();
    if missing.is_empty() {
        return Ok(DeletePropagationOutcome {
            migration_id: migration_id.to_string(),
            label_or_type: label.to_string(),
            nodes_deleted: 0,
            relationships_deleted: 0,
            missing_uids: Vec::new(),
        });
    }
    let delete_query = format!(
        "UNWIND $missing AS uid MATCH (n:{} {{{}: $mid, uid: uid}}) DETACH DELETE n",
        escape_identifier(label),
        tag
    );
    let mut del_params = BTreeMap::new();
    del_params.insert(
        "mid".to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    del_params.insert(
        "missing".to_string(),
        BoltValue::List(
            missing
                .iter()
                .map(|u| BoltValue::String(u.clone()))
                .collect(),
        ),
    );
    let meta = dest.run_write(&delete_query, del_params).await?;
    let nodes_deleted = counter(&meta, "nodes-deleted");
    Ok(DeletePropagationOutcome {
        migration_id: migration_id.to_string(),
        label_or_type: label.to_string(),
        nodes_deleted,
        relationships_deleted: 0,
        missing_uids: missing,
    })
}

/// Relationship counterpart to [`propagate_node_deletes`].
pub async fn propagate_rel_deletes(
    dest: &dyn Neo4jBackend,
    migration_id: &str,
    rel_type: &str,
    source_uids: &[String],
) -> Result<DeletePropagationOutcome, MigrateError> {
    if migration_id.trim().is_empty() {
        return Err(MigrateError::Plan(
            "Cannot propagate deletes: migration id is empty".to_string(),
        ));
    }
    use std::collections::HashSet;
    let source: HashSet<&str> = source_uids.iter().map(|s| s.as_str()).collect();
    let tag = escape_identifier(MIGRATION_TAG_PROPERTY);
    let list_query = format!(
        "MATCH ()-[r:{} {{{}: $mid}}]->() RETURN r.uid AS uid",
        escape_identifier(rel_type),
        tag
    );
    let mut list_params = BTreeMap::new();
    list_params.insert(
        "mid".to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    let rows = dest.run_read(&list_query, list_params).await?;
    let mut missing = Vec::new();
    for row in rows {
        if let Some(uid) = row.first().and_then(|v| v.as_str()) {
            if !source.contains(uid) {
                missing.push(uid.to_string());
            }
        }
    }
    missing.sort();
    if missing.is_empty() {
        return Ok(DeletePropagationOutcome {
            migration_id: migration_id.to_string(),
            label_or_type: rel_type.to_string(),
            nodes_deleted: 0,
            relationships_deleted: 0,
            missing_uids: Vec::new(),
        });
    }
    let delete_query = format!(
        "UNWIND $missing AS uid MATCH ()-[r:{} {{{}: $mid, uid: uid}}]->() DELETE r",
        escape_identifier(rel_type),
        tag
    );
    let mut del_params = BTreeMap::new();
    del_params.insert(
        "mid".to_string(),
        BoltValue::String(migration_id.to_string()),
    );
    del_params.insert(
        "missing".to_string(),
        BoltValue::List(
            missing
                .iter()
                .map(|u| BoltValue::String(u.clone()))
                .collect(),
        ),
    );
    let meta = dest.run_write(&delete_query, del_params).await?;
    let relationships_deleted = counter(&meta, "relationships-deleted");
    Ok(DeletePropagationOutcome {
        migration_id: migration_id.to_string(),
        label_or_type: rel_type.to_string(),
        nodes_deleted: 0,
        relationships_deleted,
        missing_uids: missing,
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

    fn rel_row(uid: &str, suid: &str, tuid: &str) -> BTreeMap<String, BoltValue> {
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String(uid.to_string()));
        row.insert("suid".to_string(), BoltValue::String(suid.to_string()));
        row.insert("tuid".to_string(), BoltValue::String(tuid.to_string()));
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
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
        let outcome = migrate_relationship_batch(
            &dest,
            "KNOWS",
            vec![rel_row(
                "KNOWS:Person:1->Person:2#rel:7",
                "Person:1",
                "Person:2",
            )],
        )
        .await
        .unwrap();
        assert_eq!(outcome.relationships_created, 1);
    }

    #[tokio::test]
    async fn test_parallel_edges_stay_distinct_and_replay_is_idempotent() {
        let dest = MockNeo4jBackend::new();
        let rows = vec![
            rel_row("KNOWS:A->B#rel:1", "A", "B"),
            rel_row("KNOWS:A->B#rel:2", "A", "B"),
        ];
        let first = migrate_relationship_batch(&dest, "KNOWS", rows.clone())
            .await
            .unwrap();
        assert_eq!(first.relationships_created, 2);

        // Same two edges again: MERGE matches both, creates nothing.
        let replay = migrate_relationship_batch(&dest, "KNOWS", rows)
            .await
            .unwrap();
        assert_eq!(replay.relationships_created, 0);
    }

    #[tokio::test]
    async fn test_relationship_batch_rejects_rows_without_uid() {
        let dest = MockNeo4jBackend::new();
        let mut row = BTreeMap::new();
        row.insert("suid".to_string(), BoltValue::String("A".to_string()));
        row.insert("tuid".to_string(), BoltValue::String("B".to_string()));
        row.insert("props".to_string(), BoltValue::Map(BTreeMap::new()));
        let err = migrate_relationship_batch(&dest, "KNOWS", vec![row])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("missing its stable uid"),
            "{}",
            err
        );
        assert_eq!(dest.write_count().await, 0);
    }

    #[test]
    fn test_relationship_uid_prefers_source_id_and_falls_back_to_hash() {
        let props = serde_json::json!({"since": 2020});
        let keyed = relationship_uid("KNOWS", "A", "B", Some("7"), &props);
        assert_eq!(keyed, "KNOWS:A->B#rel:7");

        let hashed = relationship_uid("KNOWS", "A", "B", None, &props);
        let hashed_again = relationship_uid("KNOWS", "A", "B", None, &props);
        assert_eq!(hashed, hashed_again);
        assert!(hashed.starts_with("KNOWS:A->B#hash:"), "{}", hashed);

        // Different properties hash differently: parallel edges stay distinct.
        let other = relationship_uid("KNOWS", "A", "B", None, &serde_json::json!({"since": 2021}));
        assert_ne!(hashed, other);
    }

    #[test]
    fn test_relationship_uid_prefers_stable_business_key_and_updates_in_place() {
        let before = serde_json::json!({"edge_uid": "e-1", "since": 2020});
        let after = serde_json::json!({"edge_uid": "e-1", "since": 2021});
        let first = relationship_uid("KNOWS", "A", "B", None, &before);
        let second = relationship_uid("KNOWS", "A", "B", None, &after);
        assert_eq!(first, second);
        assert_eq!(first, "KNOWS:A->B#key:e-1");
        assert!(!is_hash_fallback_uid(&first));

        let hashed = relationship_uid("KNOWS", "A", "B", None, &serde_json::json!({"since": 1}));
        assert!(is_hash_fallback_uid(&hashed));
    }

    #[tokio::test]
    async fn test_node_batch_uses_versioned_compare_and_set() {
        let dest = MockNeo4jBackend::new();
        migrate_node_batch(&dest, "Person", vec![node_row("Person:1", "Ada")])
            .await
            .unwrap();
        let query = dest.last_write_query().await.unwrap();
        assert!(query.contains("ON CREATE SET"), "{}", query);
        assert!(query.contains("ON MATCH SET"), "{}", query);
        assert!(query.contains("CASE WHEN"), "{}", query);
        assert!(query.contains("_version"), "{}", query);
    }

    #[tokio::test]
    async fn test_relationship_batch_uses_versioned_compare_and_set() {
        let dest = MockNeo4jBackend::new();
        migrate_relationship_batch(&dest, "KNOWS", vec![rel_row("K:A->B#rel:1", "A", "B")])
            .await
            .unwrap();
        let query = dest.last_write_query().await.unwrap();
        assert!(query.contains("ON CREATE SET"), "{}", query);
        assert!(query.contains("ON MATCH SET"), "{}", query);
        assert!(query.contains("_version"), "{}", query);
    }

    #[tokio::test]
    async fn test_rollback_removes_only_tagged_records() {
        let dest = MockNeo4jBackend::new();
        // Pre-existing untagged data (an earlier migration without tagging,
        // or native target data).
        migrate_node_batch(&dest, "Person", vec![node_row("Person:0", "Old")])
            .await
            .unwrap();
        // Tagged migration data.
        migrate_node_batch_with_options(
            &dest,
            "Person",
            vec![node_row("Person:1", "New")],
            Some("mid-1"),
        )
        .await
        .unwrap();
        migrate_relationship_batch_with_options(
            &dest,
            "KNOWS",
            vec![rel_row(
                "KNOWS:Person:1->Person:0#rel:9",
                "Person:1",
                "Person:0",
            )],
            Some("mid-1"),
        )
        .await
        .unwrap();

        let outcome = rollback_migration(&dest, "mid-1").await.unwrap();
        assert_eq!(outcome.nodes_deleted, 1);
        assert_eq!(outcome.relationships_deleted, 1);
        assert_eq!(dest.node_uids("Person").await, vec!["Person:0".to_string()]);

        // Rolling back twice is a no-op, not an error.
        let again = rollback_migration(&dest, "mid-1").await.unwrap();
        assert_eq!(again.nodes_deleted, 0);
        assert_eq!(again.relationships_deleted, 0);
    }

    #[tokio::test]
    async fn test_rollback_rejects_empty_migration_id() {
        let dest = MockNeo4jBackend::new();
        assert!(rollback_migration(&dest, "  ").await.is_err());
        assert_eq!(dest.write_count().await, 0);
    }
}
