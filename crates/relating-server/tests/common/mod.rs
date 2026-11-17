use relating_migrate::{
    bolt_to_json, chunk_message, decode_message, encode_message, json_to_bolt, BoltValue,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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
                } else if query.contains("UNWIND") && query.contains("MERGE (n:") {
                    let label = fake_label(&query).unwrap_or_default();
                    let mut created = 0i64;
                    if let Some(BoltValue::List(rows)) = params.get("batch") {
                        let mut guard = state.lock().await;
                        let table = guard.nodes.entry(label).or_default();
                        for row in rows {
                            if let BoltValue::Map(map) = row {
                                if let Some(BoltValue::String(uid)) = map.get("uid") {
                                    if !table.contains_key(uid) {
                                        created += 1;
                                    }
                                    let props =
                                        map.get("props").map(bolt_to_json).unwrap_or(Value::Null);
                                    table.insert(uid.clone(), props);
                                }
                            }
                        }
                    }
                    if !send_success(&mut stream, vec![("nodes-created", created)]).await {
                        break;
                    }
                } else if query.contains("UNWIND") && query.contains("MERGE (s)-[") {
                    let rel_type = fake_label(&query).unwrap_or_default();
                    let count = match params.get("batch") {
                        Some(BoltValue::List(rows)) => rows.len() as i64,
                        _ => 0,
                    };
                    let mut guard = state.lock().await;
                    *guard.rel_counts.entry(rel_type).or_insert(0) += count as u64;
                    if !send_success(&mut stream, vec![("relationships-created", count)]).await {
                        break;
                    }
                } else if query.contains("count(") {
                    let label = fake_label(&query).unwrap_or_default();
                    let guard = state.lock().await;
                    let count = if query.contains("-[") {
                        guard.rel_counts.get(&label).copied().unwrap_or(0)
                    } else {
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
                    if !send_success(&mut stream, vec![]).await {
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
    let state = Arc::new(Mutex::new(FakeState::default()));
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
