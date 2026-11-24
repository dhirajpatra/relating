use relating_migrate::{
    bolt_to_json, chunk_message, decode_message, encode_message, json_to_bolt, BoltValue,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Fake Bolt v4 server: speaks handshake + HELLO + RUN/PULL with MERGE
// semantics so execute/verify run end to end over real TCP.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeState {
    pub nodes: HashMap<String, HashMap<String, serde_json::Value>>,
    pub rel_counts: HashMap<String, u64>,
    pub rel_uids: HashMap<String, HashSet<String>>,
    /// Migration tags for rollback tests: (label, uid) / (type, uid) -> mid.
    pub node_tags: HashMap<(String, String), String>,
    pub rel_tags: HashMap<(String, String), String>,
    /// Fault injection: fail the Nth UNWIND write (0-based) once, then
    /// behave. Set to None to disable.
    pub fail_write_at: Option<u64>,
    pub write_seq: u64,
    pub port: u16,
    pub route_calls: u64,
}

fn fake_label(query: &str) -> Option<String> {
    let bytes = query.as_bytes();
    // Preferred: backtick-quoted identifier (generated queries always quote
    // labels and relationship types, never property keys like {uid: ...}).
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

async fn read_message(stream: &mut TcpStream) -> Option<(u8, Vec<BoltValue>)> {
    let mut payload = Vec::new();
    loop {
        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf).await.ok()?;
        let len = u16::from_be_bytes(len_buf) as usize;
        if len == 0 {
            break;
        }
        let mut chunk = vec![0u8; len];
        stream.read_exact(&mut chunk).await.ok()?;
        payload.extend_from_slice(&chunk);
    }
    decode_message(&payload).ok()
}

async fn send_message(stream: &mut TcpStream, tag: u8, fields: &[BoltValue]) -> bool {
    let raw = encode_message(tag, fields).unwrap();
    let framed = chunk_message(&raw);
    stream.write_all(&framed).await.is_ok()
}

async fn send_success(stream: &mut TcpStream, stats: Vec<(&str, i64)>) -> bool {
    let mut map = BTreeMap::new();
    let stats_map: BTreeMap<String, BoltValue> = stats
        .into_iter()
        .map(|(k, v)| (k.to_string(), BoltValue::Int(v)))
        .collect();
    map.insert("stats".to_string(), BoltValue::Map(stats_map));
    send_message(stream, 0x70, &[BoltValue::Map(map)]).await
}

fn fake_rel_type(query: &str) -> Option<String> {
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

async fn send_failure(stream: &mut TcpStream, message: &str) -> bool {
    let mut meta = BTreeMap::new();
    meta.insert(
        "code".to_string(),
        BoltValue::String("Neo.TransientError.General.DatabaseUnavailable".to_string()),
    );
    meta.insert(
        "message".to_string(),
        BoltValue::String(message.to_string()),
    );
    send_message(stream, 0x7F, &[BoltValue::Map(meta)]).await
}

async fn send_success_with_fields(
    stream: &mut TcpStream,
    stats: Vec<(&str, i64)>,
    fields: Vec<&str>,
) -> bool {
    let mut map = BTreeMap::new();
    let stats_map: BTreeMap<String, BoltValue> = stats
        .into_iter()
        .map(|(k, v)| (k.to_string(), BoltValue::Int(v)))
        .collect();
    map.insert("stats".to_string(), BoltValue::Map(stats_map));
    map.insert(
        "fields".to_string(),
        BoltValue::List(
            fields
                .into_iter()
                .map(|f| BoltValue::String(f.to_string()))
                .collect(),
        ),
    );
    send_message(stream, 0x70, &[BoltValue::Map(map)]).await
}

async fn handle_connection(mut stream: TcpStream, state: Arc<Mutex<FakeState>>) {
    // Handshake: read magic + versions, offer Bolt 4.4.
    let mut hello = [0u8; 20];
    if stream.read_exact(&mut hello).await.is_err() {
        return;
    }
    if stream.write_all(&[0, 0, 4, 4]).await.is_err() {
        return;
    }

    let mut pending_run: Option<(String, BTreeMap<String, BoltValue>)> = None;
    loop {
        let (tag, fields) = match read_message(&mut stream).await {
            Some(msg) => msg,
            None => break,
        };
        match tag {
            0x01 => {
                // HELLO
                let mut meta = BTreeMap::new();
                meta.insert(
                    "server".to_string(),
                    BoltValue::String("Neo4j/5.18.0".to_string()),
                );
                if !send_message(&mut stream, 0x70, &[BoltValue::Map(meta)]).await {
                    break;
                }
            }
            0x10 => {
                // RUN: stash query + params until PULL.
                let query = fields
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = match fields.get(1) {
                    Some(BoltValue::Map(m)) => m.clone(),
                    _ => BTreeMap::new(),
                };
                pending_run = Some((query, params));
            }
            0x3F => {
                // PULL: answer the pending RUN.
                let (query, params) = pending_run.take().unwrap_or_default();
                if query.contains("__fail__") {
                    let mut meta = BTreeMap::new();
                    meta.insert(
                        "code".to_string(),
                        BoltValue::String("Neo.ClientError.Statement.SyntaxError".to_string()),
                    );
                    meta.insert(
                        "message".to_string(),
                        BoltValue::String("injected failure for tests".to_string()),
                    );
                    if !send_message(&mut stream, 0x7F, &[BoltValue::Map(meta)]).await {
                        break;
                    }
                    continue;
                }
                if query.contains("CREATE CONSTRAINT") || query.contains("CREATE INDEX") {
                    if !send_success(&mut stream, vec![]).await {
                        break;
                    }
                } else if query.contains("UNWIND")
                    && (query.contains("MERGE (n:") || query.contains("MERGE (s)-["))
                {
                    // Fault injection: fail one numbered write, then behave.
                    let injected = {
                        let mut guard = state.lock().await;
                        let seq = guard.write_seq;
                        guard.write_seq += 1;
                        guard.fail_write_at == Some(seq)
                    };
                    if injected {
                        if !send_failure(&mut stream, "injected mid-batch failure for tests").await
                        {
                            break;
                        }
                        continue;
                    }
                    let mid = params
                        .get("mid")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    if query.contains("MERGE (n:") {
                        let label = fake_label(&query).unwrap_or_default();
                        let mut created = 0i64;
                        if let Some(BoltValue::List(rows)) = params.get("batch") {
                            let mut guard = state.lock().await;
                            let mut fresh = Vec::new();
                            {
                                let table = guard.nodes.entry(label.clone()).or_default();
                                for row in rows {
                                    if let BoltValue::Map(map) = row {
                                        if let Some(BoltValue::String(uid)) = map.get("uid") {
                                            if !table.contains_key(uid) {
                                                created += 1;
                                                fresh.push(uid.clone());
                                            }
                                            let props = map
                                                .get("props")
                                                .map(bolt_to_json)
                                                .unwrap_or(Value::Null);
                                            table.insert(uid.clone(), props);
                                        }
                                    }
                                }
                            }
                            // ON CREATE tagging: only fresh nodes are stamped.
                            if let Some(mid) = mid {
                                for uid in fresh {
                                    guard.node_tags.insert((label.clone(), uid), mid.clone());
                                }
                            }
                        }
                        if !send_success(&mut stream, vec![("nodes-created", created)]).await {
                            break;
                        }
                    } else {
                        let rel_type = fake_rel_type(&query)
                            .or_else(|| fake_label(&query))
                            .unwrap_or_default();
                        let mut created = 0i64;
                        if let Some(BoltValue::List(rows)) = params.get("batch") {
                            let mut guard = state.lock().await;
                            let mut fresh = Vec::new();
                            {
                                let set = guard.rel_uids.entry(rel_type.clone()).or_default();
                                for row in rows {
                                    if let BoltValue::Map(map) = row {
                                        match map.get("uid").and_then(|v| v.as_str()) {
                                            Some(uid) => {
                                                if set.insert(uid.to_string()) {
                                                    created += 1;
                                                    fresh.push(uid.to_string());
                                                }
                                            }
                                            None => created += 1,
                                        }
                                    }
                                }
                            }
                            let rel_len = guard
                                .rel_uids
                                .get(&rel_type)
                                .map(|s| s.len() as u64)
                                .unwrap_or(0);
                            guard.rel_counts.insert(rel_type.clone(), rel_len);
                            if let Some(mid) = mid {
                                for uid in fresh {
                                    guard.rel_tags.insert((rel_type.clone(), uid), mid.clone());
                                }
                            }
                        }
                        if !send_success(&mut stream, vec![("relationships-created", created)])
                            .await
                        {
                            break;
                        }
                    }
                } else if query.contains("DELETE") {
                    // Rollback deletes: remove records carrying the tag.
                    let mid = params
                        .get("mid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if query.contains("DETACH DELETE") {
                        let mut guard = state.lock().await;
                        let tagged: Vec<(String, String)> = guard
                            .node_tags
                            .iter()
                            .filter(|(_, tag)| **tag == mid)
                            .map(|(key, _)| key.clone())
                            .collect();
                        let deleted = tagged.len() as i64;
                        for (label, uid) in tagged {
                            guard.node_tags.remove(&(label.clone(), uid.clone()));
                            if let Some(table) = guard.nodes.get_mut(&label) {
                                table.remove(&uid);
                            }
                        }
                        drop(guard);
                        if !send_success(&mut stream, vec![("nodes-deleted", deleted)]).await {
                            break;
                        }
                    } else {
                        let mut guard = state.lock().await;
                        let tagged: Vec<(String, String)> = guard
                            .rel_tags
                            .iter()
                            .filter(|(_, tag)| **tag == mid)
                            .map(|(key, _)| key.clone())
                            .collect();
                        let deleted = tagged.len() as i64;
                        for (rel_type, uid) in &tagged {
                            guard.rel_tags.remove(&(rel_type.clone(), uid.clone()));
                            if let Some(set) = guard.rel_uids.get_mut(rel_type) {
                                set.remove(uid);
                            }
                        }
                        let affected: HashSet<String> =
                            tagged.into_iter().map(|(t, _)| t).collect();
                        for rel_type in affected {
                            let len = guard
                                .rel_uids
                                .get(&rel_type)
                                .map(|s| s.len() as u64)
                                .unwrap_or(0);
                            guard.rel_counts.insert(rel_type, len);
                        }
                        drop(guard);
                        if !send_success(&mut stream, vec![("relationships-deleted", deleted)])
                            .await
                        {
                            break;
                        }
                    }
                } else if query.contains("RETURN n.uid") {
                    let label = fake_label(&query).unwrap_or_default();
                    let guard = state.lock().await;
                    let mut uids: Vec<String> = guard
                        .nodes
                        .get(&label)
                        .map(|t| t.keys().cloned().collect())
                        .unwrap_or_default();
                    drop(guard);
                    uids.sort();
                    for uid in uids {
                        if !send_message(
                            &mut stream,
                            0x71,
                            &[BoltValue::List(vec![BoltValue::String(uid)])],
                        )
                        .await
                        {
                            break;
                        }
                    }
                    if !send_success(&mut stream, vec![]).await {
                        break;
                    }
                } else if query.contains("count(") {
                    let guard = state.lock().await;
                    let count = if query.contains("-[") {
                        let rel_type = fake_rel_type(&query)
                            .or_else(|| fake_label(&query))
                            .unwrap_or_default();
                        guard
                            .rel_uids
                            .get(&rel_type)
                            .map(|s| s.len() as u64)
                            .unwrap_or(0)
                    } else {
                        let label = fake_label(&query).unwrap_or_default();
                        guard.nodes.get(&label).map(|t| t.len() as u64).unwrap_or(0)
                    };
                    drop(guard);
                    if !send_message(
                        &mut stream,
                        0x71,
                        &[BoltValue::List(vec![BoltValue::Int(count as i64)])],
                    )
                    .await
                    {
                        break;
                    }
                    // Name the column so proxy-mode tests can assert the shaped result.
                    if !send_success_with_fields(&mut stream, vec![], vec!["c"]).await {
                        break;
                    }
                } else if query.contains("RETURN n AS node") {
                    let uid = params
                        .get("uid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let label = fake_label(&query).unwrap_or_default();
                    let guard = state.lock().await;
                    let props = guard.nodes.get(&label).and_then(|t| t.get(&uid)).cloned();
                    drop(guard);
                    if let Some(props) = props {
                        let props_map = match json_to_bolt(&props) {
                            BoltValue::Map(m) => m,
                            _ => BTreeMap::new(),
                        };
                        let node = BoltValue::Node(relating_migrate::BoltNode {
                            id: 1,
                            labels: vec![label],
                            props: props_map,
                        });
                        if !send_message(&mut stream, 0x71, &[BoltValue::List(vec![node])]).await {
                            break;
                        }
                    }
                    if !send_success(&mut stream, vec![]).await {
                        break;
                    }
                } else {
                    if !send_success(&mut stream, vec![]).await {
                        break;
                    }
                }
            }
            0x11..=0x13 => {
                // BEGIN / COMMIT / ROLLBACK
                if !send_success(&mut stream, vec![]).await {
                    break;
                }
            }
            0x66 => {
                // ROUTE: answer with a table pointing back at this fake so
                // routing-scheme clients prove the follow-the-table hop.
                let db = match fields.get(2) {
                    Some(BoltValue::String(db)) => db.clone(),
                    _ => "neo4j".to_string(),
                };
                let port = {
                    let mut guard = state.lock().await;
                    guard.route_calls += 1;
                    guard.port
                };
                let here = format!("127.0.0.1:{}", port);
                let mut table = BTreeMap::new();
                table.insert("ttl".to_string(), BoltValue::Int(300));
                table.insert("db".to_string(), BoltValue::String(db));
                let mut servers = Vec::new();
                for role in ["WRITE", "READ", "ROUTE"] {
                    let mut entry = BTreeMap::new();
                    entry.insert("role".to_string(), BoltValue::String(role.to_string()));
                    entry.insert(
                        "addresses".to_string(),
                        BoltValue::List(vec![BoltValue::String(here.clone())]),
                    );
                    servers.push(BoltValue::Map(entry));
                }
                table.insert("servers".to_string(), BoltValue::List(servers));
                if !send_message(&mut stream, 0x70, &[BoltValue::Map(table)]).await {
                    break;
                }
            }
            0x0F => {
                // RESET after a failure.
                if !send_success(&mut stream, vec![]).await {
                    break;
                }
            }
            0x02 => break, // GOODBYE
            _ => break,
        }
    }
}

pub async fn start_fake_bolt() -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<FakeState>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(Mutex::new(FakeState {
        port,
        ..FakeState::default()
    }));
    let task_state = state.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let state = task_state.clone();
            tokio::spawn(handle_connection(stream, state));
        }
    });
    (format!("bolt://127.0.0.1:{}", port), handle, state)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
