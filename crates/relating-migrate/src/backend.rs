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

/// Render a `DatabaseNotFound` failure with the probe result attached.
///
/// `Some(names)` reflects what `SHOW DATABASES` returned for the configured
/// user; `None` means even that probe failed (for example the server default
/// database is itself unreachable), so only generic guidance is possible.
fn format_db_not_found(database: &str, visible: Option<&[String]>) -> String {
    // Note: Neo4j reports a database the user cannot access the same way as
    // a database that does not exist, so both causes are named every time.
    // (Aura database names are not always `neo4j`: verify with
    // `SHOW DATABASES` in the Aura console.)
    match visible {
        Some(names) if !names.is_empty() => format!(
            "Database '{}' not found or not accessible with the configured user. Databases visible to this user: {}. Set AURA_DATABASE to one of these.",
            database,
            names.join(", ")
        ),
        Some(_) => format!(
            "Database '{}' not found or not accessible, and no databases are visible to the configured user. Check AURA_DATABASE (run SHOW DATABASES in the Aura console) and that the Bolt user (AURA_USERNAME, usually 'neo4j') has access to it.",
            database
        ),
        None => format!(
            "Database '{}' not found or not accessible. Check AURA_DATABASE against SHOW DATABASES in the Aura console, and that the Bolt user has access to it.",
            database
        ),
    }
}

/// Build the enriched `DatabaseNotFound` error.
///
/// Best effort: asks the server which databases the current user can see via
/// auto-commit against the server default database. Safe to call right after
/// a failed `BEGIN` — the client already RESET the connection on FAILURE.
async fn db_not_found_error(client: &mut BoltClient, database: &str) -> MigrateError {
    let visible: Option<Vec<String>> = client
        .run("SHOW DATABASES YIELD name", BTreeMap::new())
        .await
        .ok()
        .map(|(rows, _)| {
            rows.iter()
                .filter_map(|row| row.first()?.as_str().map(|s| s.to_string()))
                .take(10)
                .collect()
        });
    MigrateError::Protocol(format_db_not_found(database, visible.as_deref()))
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
                if e.to_string().contains("DatabaseNotFound") {
                    return Err(db_not_found_error(&mut client, &database).await);
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
            let db_name = database.clone().unwrap_or_default();
            match client.begin(database.as_deref()).await {
                Ok(()) => match client.run(query, params).await {
                    Ok((rows, _)) => client.commit().await.map(|_| rows),
                    Err(e) => {
                        let _ = client.rollback().await;
                        Err(e)
                    }
                },
                Err(e) if e.to_string().contains("DatabaseNotFound") => {
                    Err(db_not_found_error(client, &db_name).await)
                }
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
            if let Err(e) = client.begin(Some(&database)).await {
                if e.to_string().contains("DatabaseNotFound") {
                    Err(db_not_found_error(client, &database).await)
                } else {
                    Err(e)
                }
            } else {
                match client.run(query, params).await {
                    Ok((_, meta)) => client.commit().await.map(|_| meta),
                    Err(e) => {
                        let _ = client.rollback().await;
                        Err(e)
                    }
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
        let client = guard.as_mut().expect("client just connected");
        // Mirror run_read: with a database configured, proxy reads must
        // select it too, otherwise they silently hit the server default
        // database (wrong results on multi-database servers, and confusing
        // DatabaseNotFound-free "empty" answers on Aura).
        let result = if self.config.database.is_some() {
            let database = self.config.database.clone();
            let db_name = database.clone().unwrap_or_default();
            match client.begin(database.as_deref()).await {
                Ok(()) => match client.run_records_with_fields(query, params).await {
                    Ok((columns, rows, _)) => client
                        .commit()
                        .await
                        .map(|_| QueryRecords { columns, rows }),
                    Err(e) => {
                        let _ = client.rollback().await;
                        Err(e)
                    }
                },
                Err(e) if e.to_string().contains("DatabaseNotFound") => {
                    Err(db_not_found_error(client, &db_name).await)
                }
                Err(e) => Err(e),
            }
        } else {
            client
                .run_records_with_fields(query, params)
                .await
                .map(|(columns, rows, _)| QueryRecords { columns, rows })
        };
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
    /// Stored node properties by (label, uid), for sample read-back.
    node_props: HashMap<(String, String), BTreeMap<String, BoltValue>>,
    /// Relationship uids by type: uid-keyed MERGE deduplicates like Neo4j.
    rel_uids: HashMap<String, std::collections::HashSet<String>>,
    /// Migration tags by (label, uid) and (type, uid) for rollback tests.
    node_tags: HashMap<(String, String), String>,
    rel_tags: HashMap<(String, String), String>,
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

    pub async fn last_write_query(&self) -> Option<String> {
        self.state
            .lock()
            .await
            .writes
            .last()
            .map(|(q, _)| q.clone())
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

/// Extract the relationship type from a `-[r:Type]->` pattern: the first
/// identifier after the `-[" bracket. Returns None for typeless patterns.
fn extract_rel_type(query: &str) -> Option<String> {
    let bracket = query.find("-[")?;
    let after = &query[bracket + 2..];
    let colon = after.find(':')?;
    let mut rest = after[colon + 1..].trim_start();
    if let Some(stripped) = rest.strip_prefix('`') {
        let end = stripped.find('`')?;
        return Some(stripped[..end].to_string());
    }
    rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some(rest[..end].to_string())
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
        params: BTreeMap<String, BoltValue>,
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
        // Sample read-back: one stored node by label + uid.
        if query.contains("RETURN n AS node") {
            if let (Some(label), Some(uid)) = (
                extract_label(query),
                params.get("uid").and_then(|v| v.as_str()),
            ) {
                if let Some(props) = state.node_props.get(&(label.clone(), uid.to_string())) {
                    let node = crate::packstream::BoltNode {
                        id: 1,
                        labels: vec![label],
                        props: props.clone(),
                    };
                    return Ok(vec![vec![BoltValue::Node(node)]]);
                }
            }
            return Ok(Vec::new());
        }
        // Label uid listing for content-hash verification.
        if query.contains("RETURN n.uid") {
            if let Some(label) = extract_label(query) {
                let mut uids: Vec<String> = state
                    .node_uids
                    .get(&label)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
                uids.sort();
                return Ok(uids
                    .into_iter()
                    .map(|uid| vec![BoltValue::String(uid)])
                    .collect());
            }
            return Ok(Vec::new());
        }
        // Relationship uid listing for delete propagation.
        if query.contains("RETURN r.uid") {
            if let Some(rel_type) = extract_rel_type(query).or_else(|| extract_label(query)) {
                let mut uids: Vec<String> = state
                    .rel_uids
                    .get(&rel_type)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
                uids.sort();
                return Ok(uids
                    .into_iter()
                    .map(|uid| vec![BoltValue::String(uid)])
                    .collect());
            }
            return Ok(Vec::new());
        }
        // Answer count queries from recorded MERGE batches.
        if query.to_lowercase().contains("count(") {
            if query.contains("-[") {
                // Relationship or degree count: keyed by relationship type.
                // (The mock stores no endpoint labels, so a per-label degree
                // count answers the type total — exact for single-label tests.)
                let rel_type = extract_rel_type(query).or_else(|| extract_label(query));
                if let Some(rel_type) = rel_type {
                    let count = state
                        .rel_uids
                        .get(&rel_type)
                        .map(|s| s.len() as u64)
                        .unwrap_or(0);
                    return Ok(vec![vec![BoltValue::Int(count as i64)]]);
                }
            } else if let Some(label) = extract_label(query) {
                let count = state
                    .node_uids
                    .get(&label)
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
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
        // Rollback deletes: remove records carrying the migration tag.
        // Propagate deletes carry $missing and remove only those uids.
        if query.contains("DELETE") {
            let mid = params.get("mid").and_then(|v| v.as_str()).unwrap_or("");
            let missing: Option<std::collections::HashSet<String>> = match params.get("missing") {
                Some(BoltValue::List(uids)) => Some(
                    uids.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                ),
                _ => None,
            };
            if query.contains("DETACH DELETE") {
                let tagged: Vec<(String, String)> = state
                    .node_tags
                    .iter()
                    .filter(|(_, tag)| tag.as_str() == mid)
                    .map(|(key, _)| key.clone())
                    .filter(|(_, uid)| missing.as_ref().map(|m| m.contains(uid)).unwrap_or(true))
                    .collect();
                let deleted = tagged.len() as i64;
                for (label, uid) in tagged {
                    state.node_tags.remove(&(label.clone(), uid.clone()));
                    state.node_props.remove(&(label.clone(), uid.clone()));
                    if let Some(set) = state.node_uids.get_mut(&label) {
                        set.remove(&uid);
                    }
                }
                stats.insert("nodes-deleted".to_string(), BoltValue::Int(deleted));
            } else {
                let tagged: Vec<(String, String)> = state
                    .rel_tags
                    .iter()
                    .filter(|(_, tag)| tag.as_str() == mid)
                    .map(|(key, _)| key.clone())
                    .filter(|(_, uid)| missing.as_ref().map(|m| m.contains(uid)).unwrap_or(true))
                    .collect();
                let deleted = tagged.len() as i64;
                for (rel_type, uid) in tagged {
                    state.rel_tags.remove(&(rel_type.clone(), uid.clone()));
                    if let Some(set) = state.rel_uids.get_mut(&rel_type) {
                        set.remove(&uid);
                    }
                }
                stats.insert("relationships-deleted".to_string(), BoltValue::Int(deleted));
            }
            let mut meta = BTreeMap::new();
            meta.insert("stats".to_string(), BoltValue::Map(stats));
            return Ok(meta);
        }
        if query.contains("UNWIND") && query.contains("MERGE (n:") {
            if let Some(label) = extract_label(query) {
                let mid = params
                    .get("mid")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let mut created = 0i64;
                if let Some(BoltValue::List(rows)) = params.get("batch") {
                    for row in rows {
                        if let BoltValue::Map(map) = row {
                            if let Some(BoltValue::String(uid)) = map.get("uid") {
                                let is_new = state
                                    .node_uids
                                    .entry(label.clone())
                                    .or_default()
                                    .insert(uid.clone());
                                if is_new {
                                    created += 1;
                                    // ON CREATE tagging: only fresh nodes are stamped.
                                    if let Some(mid) = mid.clone() {
                                        state.node_tags.insert((label.clone(), uid.clone()), mid);
                                    }
                                }
                                if let Some(BoltValue::Map(props)) = map.get("props") {
                                    state
                                        .node_props
                                        .insert((label.clone(), uid.clone()), props.clone());
                                }
                            }
                        }
                    }
                }
                stats.insert("nodes-created".to_string(), BoltValue::Int(created));
            }
        } else if query.contains("UNWIND") && query.contains("MERGE (s)-[") {
            if let Some(rel_type) = extract_label(query) {
                let mid = params
                    .get("mid")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let mut created = 0i64;
                if let Some(BoltValue::List(rows)) = params.get("batch") {
                    for row in rows {
                        if let BoltValue::Map(map) = row {
                            // Uid-keyed MERGE: only unseen uids are created.
                            // Rows without a uid keep the legacy count-each
                            // behavior so direct mock callers stay honest.
                            match map.get("uid").and_then(|v| v.as_str()) {
                                Some(uid) => {
                                    let is_new = state
                                        .rel_uids
                                        .entry(rel_type.clone())
                                        .or_default()
                                        .insert(uid.to_string());
                                    if is_new {
                                        created += 1;
                                        if let Some(mid) = mid.clone() {
                                            state
                                                .rel_tags
                                                .insert((rel_type.clone(), uid.to_string()), mid);
                                        }
                                    }
                                }
                                None => created += 1,
                            }
                        }
                    }
                }
                stats.insert("relationships-created".to_string(), BoltValue::Int(created));
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
    fn test_format_db_not_found_lists_visible_databases() {
        let msg = format_db_not_found(
            "wrongdb",
            Some(&["neo4j".to_string(), "analytics".to_string()]),
        );
        assert!(msg.contains("'wrongdb'"), "{}", msg);
        assert!(msg.contains("neo4j, analytics"), "{}", msg);
        assert!(msg.contains("AURA_DATABASE"), "{}", msg);

        let msg = format_db_not_found("wrongdb", Some(&[]));
        assert!(msg.contains("no databases are visible"), "{}", msg);

        let msg = format_db_not_found("wrongdb", None);
        assert!(msg.contains("'wrongdb'"), "{}", msg);
        assert!(msg.contains("SHOW DATABASES"), "{}", msg);
    }

    #[test]
    fn test_extract_rel_type_prefers_bracket_pattern() {
        assert_eq!(
            extract_rel_type("MATCH (n:`Person`)-[r:`KNOWS`]->() RETURN count(r)"),
            Some("KNOWS".to_string())
        );
        assert_eq!(
            extract_rel_type("MATCH ()-[r:KNOWS]->() RETURN count(r)"),
            Some("KNOWS".to_string())
        );
        assert_eq!(extract_rel_type("MATCH (n:Person) RETURN n"), None);
    }

    /// End-to-end over the wire: a `DatabaseNotFound` on `BEGIN` triggers the
    /// `SHOW DATABASES` probe, and the probe rows survive the real-server
    /// framing where RUN gets its own SUCCESS ahead of any RECORD.
    #[tokio::test]
    async fn test_db_not_found_error_lists_server_databases() {
        use crate::bolt::{BoltConfig, BoltUri};
        use crate::packstream::{decode_message, encode_message, BoltValue as BV};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_msg(stream: &mut tokio::net::TcpStream) -> (u8, Vec<BV>) {
            let mut payload = Vec::new();
            loop {
                let mut len = [0u8; 2];
                stream.read_exact(&mut len).await.unwrap();
                let n = u16::from_be_bytes(len) as usize;
                if n == 0 {
                    break;
                }
                let mut chunk = vec![0u8; n];
                stream.read_exact(&mut chunk).await.unwrap();
                payload.extend_from_slice(&chunk);
            }
            decode_message(&payload).unwrap()
        }

        async fn send_msg(stream: &mut tokio::net::TcpStream, tag: u8, fields: &[BV]) {
            let raw = encode_message(tag, fields).unwrap();
            stream
                .write_all(&crate::bolt::chunk_message(&raw))
                .await
                .unwrap();
        }

        fn map(pairs: &[(&str, BV)]) -> BV {
            BV::Map(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut hs = [0u8; 20];
            stream.read_exact(&mut hs).await.unwrap();
            stream
                .write_all(&0x00000404u32.to_be_bytes())
                .await
                .unwrap(); // Bolt 4.4

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x01); // HELLO
            send_msg(
                &mut stream,
                0x70,
                &[map(&[(
                    "server",
                    BV::String("Neo4j/5.27-fake".to_string()),
                )])],
            )
            .await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x11); // BEGIN {db}
            send_msg(
                &mut stream,
                0x7F,
                &[map(&[
                    (
                        "code",
                        BV::String("Neo.ClientError.Database.DatabaseNotFound".to_string()),
                    ),
                    (
                        "message",
                        BV::String("Database wrongdb not found".to_string()),
                    ),
                ])],
            )
            .await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x0F); // RESET after failure
            send_msg(&mut stream, 0x70, &[map(&[])]).await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x10); // RUN SHOW DATABASES
            send_msg(
                &mut stream,
                0x70,
                &[map(&[(
                    "fields",
                    BV::List(vec![BV::String("name".to_string())]),
                )])],
            )
            .await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x3F); // PULL
            send_msg(
                &mut stream,
                0x71,
                &[BV::List(vec![BV::String("neo4j".to_string())])],
            )
            .await;
            send_msg(
                &mut stream,
                0x71,
                &[BV::List(vec![BV::String("system".to_string())])],
            )
            .await;
            send_msg(&mut stream, 0x70, &[map(&[])]).await;
        });

        let uri = BoltUri::parse(&format!("bolt://127.0.0.1:{}", port)).unwrap();
        let mut config = BoltConfig::new(uri, "neo4j".to_string(), "test".to_string());
        config.database = Some("wrongdb".to_string());
        let backend = BoltBackend::new(config);
        let err = backend.test_connection().await.expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("neo4j, system"), "{}", msg);
        assert!(msg.contains("AURA_DATABASE"), "{}", msg);
        server.abort();
    }

    /// `run_query` (the Cypher proxy path) must open an explicit
    /// transaction against the configured database, exactly like `run_read`
    /// and `run_write`, or proxy reads silently hit the server default.
    #[tokio::test]
    async fn test_run_query_selects_configured_database() {
        use crate::bolt::{BoltConfig, BoltUri};
        use crate::packstream::{decode_message, encode_message, BoltValue as BV};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_msg(stream: &mut tokio::net::TcpStream) -> (u8, Vec<BV>) {
            let mut payload = Vec::new();
            loop {
                let mut len = [0u8; 2];
                stream.read_exact(&mut len).await.unwrap();
                let n = u16::from_be_bytes(len) as usize;
                if n == 0 {
                    break;
                }
                let mut chunk = vec![0u8; n];
                stream.read_exact(&mut chunk).await.unwrap();
                payload.extend_from_slice(&chunk);
            }
            decode_message(&payload).unwrap()
        }

        async fn send_msg(stream: &mut tokio::net::TcpStream, tag: u8, fields: &[BV]) {
            let raw = encode_message(tag, fields).unwrap();
            stream
                .write_all(&crate::bolt::chunk_message(&raw))
                .await
                .unwrap();
        }

        fn map(pairs: &[(&str, BV)]) -> BV {
            BV::Map(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut hs = [0u8; 20];
            stream.read_exact(&mut hs).await.unwrap();
            stream
                .write_all(&0x00000404u32.to_be_bytes())
                .await
                .unwrap(); // Bolt 4.4

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x01); // HELLO
            send_msg(
                &mut stream,
                0x70,
                &[map(&[(
                    "server",
                    BV::String("Neo4j/5.27-fake".to_string()),
                )])],
            )
            .await;

            // BEGIN must carry the configured database.
            let (tag, fields) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x11);
            match fields.as_slice() {
                [BV::Map(extra)] => assert_eq!(
                    extra.get("db").and_then(|v| v.as_str()),
                    Some("analytics"),
                    "BEGIN must select the configured database"
                ),
                other => panic!("BEGIN must carry an extras map, got {:?}", other),
            }
            send_msg(&mut stream, 0x70, &[map(&[])]).await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x10); // RUN
            send_msg(
                &mut stream,
                0x70,
                &[map(&[(
                    "fields",
                    BV::List(vec![BV::String("one".to_string())]),
                )])],
            )
            .await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x3F); // PULL
            send_msg(&mut stream, 0x71, &[BV::List(vec![BV::Int(1)])]).await;
            send_msg(&mut stream, 0x70, &[map(&[])]).await;

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x12); // COMMIT
            send_msg(&mut stream, 0x70, &[map(&[])]).await;
        });

        let uri = BoltUri::parse(&format!("bolt://127.0.0.1:{}", port)).unwrap();
        let mut config = BoltConfig::new(uri, "neo4j".to_string(), "test".to_string());
        config.database = Some("analytics".to_string());
        let backend = BoltBackend::new(config);
        let records = backend
            .run_query("RETURN 1 AS one", BTreeMap::new())
            .await
            .expect("run_query must succeed");
        assert_eq!(records.columns, vec!["one".to_string()]);
        assert_eq!(records.rows, vec![vec![BV::Int(1)]]);
        server.abort();
    }

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
