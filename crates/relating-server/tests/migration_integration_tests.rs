use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_migrate::{
    bolt_to_json, chunk_message, decode_message, encode_message, json_to_bolt, BoltValue,
};
use relating_server::{create_router, AppState};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tower::ServiceExt;

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_AURA_PASSWORD";

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
    };
    let app = create_router(state);
    (app, dir)
}

async fn post_json(app: &axum::Router, uri: String, payload: Value) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, json)
}

async fn get_json(app: &axum::Router, uri: String) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

// ---------------------------------------------------------------------------
// Fake Bolt v4 server: speaks handshake + HELLO + RUN/PULL with MERGE
// semantics so execute/verify run end to end over real TCP.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeState {
    nodes: HashMap<String, HashMap<String, serde_json::Value>>,
    rel_counts: HashMap<String, u64>,
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
            0x02 => break, // GOODBYE
            _ => break,
        }
    }
}

async fn start_fake_bolt() -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<FakeState>>) {
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

#[tokio::test]
async fn test_connection_crud_and_validation() {
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Aura CRUD"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    // Valid connection (never stores a password).
    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "prod",
            "uri": "neo4j+s://abc.databases.neo4j.io",
            "username": "neo4j",
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(conn["name"], "prod");
    assert!(conn.get("password").is_none());
    let cid = conn["id"].as_str().unwrap().to_string();

    // Duplicate names are rejected.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({"name": "prod", "uri": "bolt://localhost"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Bad URIs and embedded credentials are rejected.
    for bad_uri in [
        "http://localhost:7474",
        "bolt://neo4j:secret@localhost",
        "not a uri",
    ] {
        let (status, err) = post_json(
            &app,
            format!("/api/v1/projects/{}/connections/aura", pid),
            json!({"name": format!("c-{}", bad_uri.len()), "uri": bad_uri}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "uri {}", bad_uri);
        assert_eq!(err["error"]["code"], "BAD_REQUEST");
    }

    let (status, list) = get_json(&app, format!("/api/v1/projects/{}/connections/aura", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let res = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/projects/{}/connections/aura/{}", pid, cid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_unreachable_target_reports_honestly() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Unreachable"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "down",
            "uri": "bolt://127.0.0.1:9",
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    // Connection test fails fast with a gateway error, not a hang.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura/{}/test", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(err["error"]["code"], "AURA_UNREACHABLE");

    // Analysis still succeeds: source findings plus an unreachable-target note.
    let (status, analysis) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/analyze", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(analysis["target_reachable"], false);
    assert!(!analysis["source_findings"].as_array().unwrap().is_empty());

    // Plan records the blocker; approval and execution are refused.
    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(!plan["dry_run"]["blockers"].as_array().unwrap().is_empty());
    let mid = plan["migration"]["id"].as_str().unwrap();

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_full_migration_against_fake_bolt_server() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _fake_state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Migrate E2E"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap().to_string();

    // Seed the source graph with uid-carrying nodes and one relationship.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({
            "query": "CREATE (a:Person {uid: 'Person:1', name: 'Ada'})-[:KNOWS]->(b:Person {uid: 'Person:2', name: 'Bob'})"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "fake",
            "uri": uri,
            "username": "neo4j",
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap().to_string();

    // 1. Connection test reports the fake server agent.
    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura/{}/test", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["reachable"], true);
    assert_eq!(report["server_agent"], "Neo4j/5.18.0");

    // 2. Analysis covers source and target.
    let (status, analysis) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/analyze", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(analysis["target_reachable"], true);
    let subjects: Vec<String> = analysis["source_findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["subject"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(subjects.contains(&"Person".to_string()));
    assert!(subjects.contains(&"KNOWS".to_string()));

    // 3. Plan: 2 nodes + 1 relationship, uid DDL, zero blockers.
    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(plan["migration"]["plan"]["total_nodes"], 2);
    assert_eq!(plan["migration"]["plan"]["total_relationships"], 1);
    assert!(!plan["migration"]["plan"]["ddl_statements"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(plan["dry_run"]["blockers"].as_array().unwrap().is_empty());
    assert_eq!(plan["dry_run"]["remote_writes"], 0);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();

    // 4. Explicit approval, then execution over real Bolt TCP.
    let (status, approved) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");

    let (status, executed) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(executed["status"], "completed");
    assert_eq!(executed["progress"]["nodes_migrated"], 2);
    assert_eq!(executed["progress"]["relationships_migrated"], 1);
    assert_eq!(
        executed["progress"]["completed_batches"]
            .as_array()
            .unwrap()
            .len(),
        executed["progress"]["batches_total"].as_u64().unwrap() as usize
    );

    // 5. Verification passes against the migrated data.
    let (status, verification) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/verify", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verification["passed"], true);
    assert_eq!(verification["samples_checked"], 2);
    assert_eq!(verification["samples_matched"], 2);

    // 6. Full manifest report.
    let (status, manifest) = get_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/report", pid, mid),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(manifest["status"], "verified");

    server.abort();
}
