use crate::bolt::{BoltClient, BoltConfig};
use crate::error::MigrateError;
use crate::packstream::BoltValue;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetInfo {
    pub server_agent: String,
    pub bolt_major: u8,
    pub bolt_minor: u8,
    pub database: Option<String>,
}

/// Column-named result rows for proxy mode: the same Cypher text returns the
/// same shaped result whether it runs on the local Relate store or on Aura.
#[derive(Debug, Clone, Default)]
pub struct QueryRecords {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<BoltValue>>,
}

#[async_trait]
pub trait Neo4jBackend: Send + Sync {
    async fn test_connection(&self) -> Result<TargetInfo, MigrateError>;
    async fn run_read(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<Vec<Vec<BoltValue>>, MigrateError>;
    /// Returns the SUCCESS metadata (including `stats` counters on writes).
    async fn run_write(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<BTreeMap<String, BoltValue>, MigrateError>;
    /// RUN + PULL with column names, used by proxy mode so Aura results match
    /// the local result shape. Existing `run_read` callers are unaffected.
    async fn run_query(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<QueryRecords, MigrateError>;
}

pub struct BoltBackend {
    config: BoltConfig,
    client: Mutex<Option<BoltClient>>,
}

impl BoltBackend {
    pub fn new(config: BoltConfig) -> Self {
        Self {
            config,
            client: Mutex::new(None),
        }
    }

    async fn ensure_connected(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<BoltClient>>,
    ) -> Result<(), MigrateError> {
        if guard.is_none() {
            let client = BoltClient::connect(&self.config).await?;
            **guard = Some(client);
        }
        Ok(())
    }

    fn drop_poisoned(guard: &mut tokio::sync::MutexGuard<'_, Option<BoltClient>>) {
        // Drop a poisoned connection so the next call reconnects.
        **guard = None;
    }
}

#[async_trait]
impl Neo4jBackend for BoltBackend {
    async fn test_connection(&self) -> Result<TargetInfo, MigrateError> {
        // Always a fresh connection: validates URI, TLS, and credentials.
        let mut client = BoltClient::connect(&self.config).await?;
        // A bare HELLO never selects a database, so a wrong database name
        // would report "connected" here and fail later at first write.
        // Validate it now with an explicit transaction that reads nothing.
        if let Some(database) = self.config.database.clone() {
            if let Err(e) = client.begin(Some(&database)).await {
                let msg = e.to_string();
                if msg.contains("DatabaseNotFound") {
                    return Err(MigrateError::Protocol(format!(
                        "Database '{}' not found on the target (the instance ID is not a database name; use the database name from the Aura console, usually 'neo4j')",
                        database
                    )));
                }
                return Err(e);
            }
            let _ = client.rollback().await;
        }
        Ok(TargetInfo {
            server_agent: client.server_agent.clone(),
            bolt_major: client.negotiated_major,
            bolt_minor: client.negotiated_minor,
            database: self.config.database.clone(),
        })
    }

    async fn run_read(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<Vec<Vec<BoltValue>>, MigrateError> {
        let mut guard = self.client.lock().await;
        self.ensure_connected(&mut guard).await?;
        let client = guard.as_mut().expect("client just connected");
        // Mirror run_write: with a database configured, reads must select
        // it too, otherwise they silently hit the server default database.
        let result = if self.config.database.is_some() {
            let database = self.config.database.clone();
            match client.begin(database.as_deref()).await {
                Ok(()) => match client.run(query, params).await {
                    Ok((rows, _)) => client.commit().await.map(|_| rows),
                    Err(e) => {
                        let _ = client.rollback().await;
                        Err(e)
                    }
                },
                Err(e) => Err(e),
            }
        } else {
            client.run(query, params).await.map(|(rows, _)| rows)
        };
        if matches!(
            result,
            Err(MigrateError::Connection(_)) | Err(MigrateError::Protocol(_))
        ) {
            Self::drop_poisoned(&mut guard);
        }
        result
    }

    async fn run_write(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<BTreeMap<String, BoltValue>, MigrateError> {
        let mut guard = self.client.lock().await;
        self.ensure_connected(&mut guard).await?;
        let client = guard.as_mut().expect("client just connected");
        let result = if let Some(database) = self.config.database.clone() {
            client.begin(Some(&database)).await?;
            match client.run(query, params).await {
                Ok((_, meta)) => client.commit().await.map(|_| meta),
                Err(e) => {
                    let _ = client.rollback().await;
                    Err(e)
                }
            }
        } else {
            client.run(query, params).await.map(|(_, meta)| meta)
        };
        if matches!(
            result,
            Err(MigrateError::Connection(_)) | Err(MigrateError::Protocol(_))
        ) {
            Self::drop_poisoned(&mut guard);
        }
        result
    }

    async fn run_query(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<QueryRecords, MigrateError> {
        let mut guard = self.client.lock().await;
        self.ensure_connected(&mut guard).await?;
        let result = guard
            .as_mut()
            .expect("client just connected")
            .run_records_with_fields(query, params)
            .await
            .map(|(columns, rows, _)| QueryRecords { columns, rows });
        if matches!(
            result,
            Err(MigrateError::Connection(_)) | Err(MigrateError::Protocol(_))
        ) {
            Self::drop_poisoned(&mut guard);
        }
        result
    }
}

#[derive(Debug, Default)]
struct MockState {
    writes: Vec<(String, BTreeMap<String, BoltValue>)>,
    node_uids: HashMap<String, std::collections::HashSet<String>>,
    rel_counts: HashMap<String, u64>,
    scripted_reads: HashMap<String, Vec<Vec<BoltValue>>>,
    scripted_columns: HashMap<String, Vec<String>>,
}

/// In-memory backend for tests: records writes and answers count queries
/// from what was written (deduplicated by uid, like MERGE semantics).
pub struct MockNeo4jBackend {
    state: Mutex<MockState>,
    pub fail_reads: bool,
}

impl MockNeo4jBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockState::default()),
            fail_reads: false,
        }
    }

    pub async fn script_read(&self, query_contains: &str, rows: Vec<Vec<BoltValue>>) {
        let mut state = self.state.lock().await;
        state
            .scripted_reads
            .insert(query_contains.to_string(), rows);
    }

    pub async fn script_columns(&self, query_contains: &str, columns: Vec<String>) {
        let mut state = self.state.lock().await;
        state
            .scripted_columns
            .insert(query_contains.to_string(), columns);
    }

    pub async fn write_count(&self) -> usize {
        self.state.lock().await.writes.len()
    }

    pub async fn node_uids(&self, label: &str) -> Vec<String> {
        let state = self.state.lock().await;
        state
            .node_uids
            .get(label)
            .map(|set| {
                let mut out: Vec<String> = set.iter().cloned().collect();
                out.sort();
                out
            })
            .unwrap_or_default()
    }
}

impl Default for MockNeo4jBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_label(query: &str) -> Option<String> {
    // Matches `MERGE (n:`Label` ...`, `MATCH (n:Label) ...`, `MATCH ()-[r:`T`]->...`.
    // Backtick-quoted identifiers win: property maps like {uid: ...} also
    // contain colons but are never quoted.
    let bytes = query.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'`' {
                j += 1;
            }
            if j < bytes.len() {
                return Some(query[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'`' {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'`' {
                    i += 1;
                }
                return Some(query[start..i].to_string());
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i > start {
                return Some(query[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

#[async_trait]
impl Neo4jBackend for MockNeo4jBackend {
    async fn test_connection(&self) -> Result<TargetInfo, MigrateError> {
        Ok(TargetInfo {
            server_agent: "Neo4j/Mock-5.0".to_string(),
            bolt_major: 4,
            bolt_minor: 4,
            database: Some("neo4j".to_string()),
        })
    }

    async fn run_read(
        &self,
        query: &str,
        _params: BTreeMap<String, BoltValue>,
    ) -> Result<Vec<Vec<BoltValue>>, MigrateError> {
        if self.fail_reads {
            return Err(MigrateError::Connection("mock read failure".to_string()));
        }
        let state = self.state.lock().await;
        for (needle, rows) in &state.scripted_reads {
            if query.contains(needle) {
                return Ok(rows.clone());
            }
        }
        // Answer count queries from recorded MERGE batches.
        if query.to_lowercase().contains("count(") {
            if let Some(label) = extract_label(query) {
                let is_rel = query.contains("-[");
                let count = if is_rel {
                    state.rel_counts.get(&label).copied().unwrap_or(0)
                } else {
                    state
                        .node_uids
                        .get(&label)
                        .map(|s| s.len() as u64)
                        .unwrap_or(0)
                };
                return Ok(vec![vec![BoltValue::Int(count as i64)]]);
            }
        }
        Ok(Vec::new())
    }

    async fn run_write(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<BTreeMap<String, BoltValue>, MigrateError> {
        let mut state = self.state.lock().await;
        state.writes.push((query.to_string(), params.clone()));

        let mut stats = BTreeMap::new();
        let batch_rows = match params.get("batch") {
            Some(BoltValue::List(rows)) => rows.len(),
            _ => 0,
        };
        if query.contains("UNWIND") && query.contains("MERGE (n:") {
            if let Some(label) = extract_label(query) {
                let set = state.node_uids.entry(label).or_default();
                let before = set.len();
                if let Some(BoltValue::List(rows)) = params.get("batch") {
                    for row in rows {
                        if let BoltValue::Map(map) = row {
                            if let Some(BoltValue::String(uid)) = map.get("uid") {
                                set.insert(uid.clone());
                            }
                        }
                    }
                }
                stats.insert(
                    "nodes-created".to_string(),
                    BoltValue::Int((set.len() - before) as i64),
                );
            }
        } else if query.contains("UNWIND") && query.contains("MERGE (s)-[") {
            if let Some(rel_type) = extract_label(query) {
                let entry = state.rel_counts.entry(rel_type).or_insert(0);
                *entry += batch_rows as u64;
                stats.insert(
                    "relationships-created".to_string(),
                    BoltValue::Int(batch_rows as i64),
                );
            }
        }
        let mut meta = BTreeMap::new();
        meta.insert("stats".to_string(), BoltValue::Map(stats));
        Ok(meta)
    }

    async fn run_query(
        &self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<QueryRecords, MigrateError> {
        let rows = self.run_read(query, params).await?;
        let state = self.state.lock().await;
        for (needle, columns) in &state.scripted_columns {
            if query.contains(needle) {
                return Ok(QueryRecords {
                    columns: columns.clone(),
                    rows,
                });
            }
        }
        // No scripted names: synthesize positional columns from the row width.
        let width = rows.first().map(|r| r.len()).unwrap_or(0);
        Ok(QueryRecords {
            columns: (0..width).map(|i| format!("col_{}", i)).collect(),
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_label() {
        assert_eq!(
            extract_label("MERGE (n:`Person` {uid: $uid})"),
            Some("Person".to_string())
        );
        assert_eq!(
            extract_label("MATCH ()-[r:KNOWS]->() RETURN count(r)"),
            Some("KNOWS".to_string())
        );
        assert_eq!(extract_label("RETURN 1"), None);
    }

    #[tokio::test]
    async fn test_mock_backend_dedupes_by_uid() {
        let backend = MockNeo4jBackend::new();
        let mut row = BTreeMap::new();
        row.insert("uid".to_string(), BoltValue::String("Person:1".to_string()));
        let mut params = BTreeMap::new();
        params.insert(
            "batch".to_string(),
            BoltValue::List(vec![BoltValue::Map(row)]),
        );
        let query = "UNWIND $batch AS row MERGE (n:`Person` {uid: row.uid})";
        let meta = backend.run_write(query, params.clone()).await.unwrap();
        assert_eq!(
            meta["stats"],
            BoltValue::Map(BTreeMap::from([(
                "nodes-created".to_string(),
                BoltValue::Int(1)
            )]))
        );
        // Replay the same batch: MERGE creates nothing new.
        let meta = backend.run_write(query, params).await.unwrap();
        assert_eq!(
            meta["stats"],
            BoltValue::Map(BTreeMap::from([(
                "nodes-created".to_string(),
                BoltValue::Int(0)
            )]))
        );
        let rows = backend
            .run_read("MATCH (n:Person) RETURN count(n)", BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(rows, vec![vec![BoltValue::Int(1)]]);
    }
}
