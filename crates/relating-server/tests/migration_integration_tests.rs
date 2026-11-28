use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_server::{create_router, AppState, AuthState};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

mod common;
use common::start_fake_bolt;

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_AURA_PASSWORD";

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: None,
        auth: AuthState::default(),
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

#[tokio::test]
async fn test_analyze_accepts_and_validates_aura_tier() {
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Tier check"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    // Source-only analysis with a tier succeeds (no profiles => no vector findings).
    let (status, analysis) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/analyze", pid),
        json!({"aura_tier": "professional-4gb"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!analysis["source_findings"].as_array().unwrap().is_empty());

    // Unknown tiers are rejected before any analysis runs.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/analyze", pid),
        json!({"aura_tier": "mystery-tier"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("mystery-tier"));
}

async fn seed_project_with_cypher(app: &axum::Router, name: &str, statements: &[&str]) -> String {
    let (status, proj) =
        post_json(app, "/api/v1/projects".to_string(), json!({"name": name})).await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap().to_string();
    for stmt in statements {
        let (status, _) = post_json(
            app,
            format!("/api/v1/projects/{}/cypher/write", pid),
            json!({"query": stmt}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "statement: {}", stmt);
    }
    pid
}

async fn connect_fake_bolt(app: &axum::Router, pid: &str, uri: &str) -> String {
    let (status, conn) = post_json(
        app,
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
    conn["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_execute_resumes_after_mid_batch_failure() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    // Three nodes, batch size 1: three node batches, no relationships.
    let pid = seed_project_with_cypher(
        &app,
        "Resume E2E",
        &[
            "CREATE (:Person {uid: 'Person:1', name: 'Ada'})",
            "CREATE (:Person {uid: 'Person:2', name: 'Bob'})",
            "CREATE (:Person {uid: 'Person:3', name: 'Cyd'})",
        ],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;

    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid, "batch_size": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(plan["dry_run"]["blockers"].as_array().unwrap().is_empty());
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Kill the second write: batch 0 lands, batch 1 fails, batch 2 never runs.
    fake_state.lock().await.fail_write_at = Some(1);
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "execute must fail, got {}",
        status
    );
    let (status, failed) =
        get_json(&app, format!("/api/v1/projects/{}/migrations/{}", pid, mid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(failed["status"], "failed");
    assert_eq!(
        failed["progress"]["completed_batches"],
        json!(["nodes:Person:0"])
    );

    // Resume: the completed batch is skipped, the rest land exactly once.
    fake_state.lock().await.fail_write_at = None;
    let (status, executed) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(executed["status"], "completed");
    assert_eq!(executed["progress"]["nodes_migrated"], 3);
    assert_eq!(
        executed["progress"]["completed_batches"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let guard = fake_state.lock().await;
    assert_eq!(guard.nodes.get("Person").map(|t| t.len()), Some(3));

    server.abort();
}

#[tokio::test]
async fn test_parallel_edges_between_same_nodes_stay_distinct() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    let pid = seed_project_with_cypher(
        &app,
        "Parallel edges",
        &[
            "CREATE (:Person {uid: 'Person:1', name: 'Ada'})",
            "CREATE (:Person {uid: 'Person:2', name: 'Bob'})",
            "MATCH (a:Person {uid: 'Person:1'}), (b:Person {uid: 'Person:2'}) CREATE (a)-[:KNOWS {since: 2020}]->(b)",
            "MATCH (a:Person {uid: 'Person:1'}), (b:Person {uid: 'Person:2'}) CREATE (a)-[:KNOWS {since: 2021}]->(b)",
        ],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;

    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(plan["migration"]["plan"]["total_relationships"], 2);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, executed) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(executed["progress"]["relationships_migrated"], 2);

    // Two distinct target edges, and verification (counts + hashes + degrees)
    // agrees with the source.
    assert_eq!(
        fake_state
            .lock()
            .await
            .rel_uids
            .get("KNOWS")
            .map(|s| s.len()),
        Some(2)
    );
    let (status, verification) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/verify", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verification["passed"], true);
    assert_eq!(verification["hash_checks"].as_array().unwrap().len(), 1);
    assert_eq!(verification["degree_checks"].as_array().unwrap().len(), 1);

    server.abort();
}

#[tokio::test]
async fn test_rollback_removes_only_what_the_migration_created() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    let pid = seed_project_with_cypher(
        &app,
        "Rollback E2E",
        &[
            "CREATE (a:Person {uid: 'Person:1', name: 'Ada'})-[:KNOWS]->(b:Person {uid: 'Person:2', name: 'Bob'})",
        ],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;

    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();

    // A migration that never executed has nothing to roll back.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/rollback", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fake_state.lock().await.nodes.get("Person").map(|t| t.len()),
        Some(2)
    );

    let (status, outcome) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/rollback", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(outcome["nodes_deleted"], 2);
    assert_eq!(outcome["relationships_deleted"], 1);

    let (status, rolled_back) =
        get_json(&app, format!("/api/v1/projects/{}/migrations/{}", pid, mid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rolled_back["status"], "rolled_back");

    let guard = fake_state.lock().await;
    assert_eq!(guard.nodes.get("Person").map(|t| t.len()), Some(0));
    assert_eq!(guard.rel_uids.get("KNOWS").map(|s| s.len()), Some(0));

    server.abort();
}

#[tokio::test]
async fn test_delta_sync_transfers_new_records_and_propagates_deletes() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    let pid = seed_project_with_cypher(
        &app,
        "Delta sync",
        &[
            "CREATE (:Person {uid: 'Person:1', name: 'Ada'})",
            "CREATE (:Person {uid: 'Person:2', name: 'Bob'})",
        ],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;
    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, executed) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(executed["progress"]["sync_mode"], "full");
    assert!(executed["progress"]["last_sync"].is_string());
    assert!(executed["progress"]["high_water_mark"].is_string());

    // Add one node, then delta re-execute in place (no new plan needed).
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (:Person {uid: 'Person:3', name: 'Cyd'})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, synced) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/sync", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(synced["status"], "completed");
    assert_eq!(synced["progress"]["sync_mode"], "delta");
    assert_eq!(synced["progress"]["nodes_migrated"], 3);
    assert_eq!(
        fake_state.lock().await.nodes.get("Person").map(|t| t.len()),
        Some(3)
    );

    // Delete one source node: plain sync leaves the target copy (merge-upsert
    // never deletes); sync with propagation removes it.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "MATCH (n:Person {uid: 'Person:2'}) DETACH DELETE n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/projects/{}/migrations/{}/sync?propagate_deletes=true",
                    pid, mid
                ))
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let propagated: Value = serde_json::from_slice(&body).unwrap();
    assert!(propagated["progress"]["message"]
        .as_str()
        .unwrap()
        .contains("delete propagation"));
    assert_eq!(
        fake_state.lock().await.nodes.get("Person").map(|t| t.len()),
        Some(2)
    );

    server.abort();
}

#[tokio::test]
async fn test_drift_detector_reports_without_changing_status() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    let pid = seed_project_with_cypher(
        &app,
        "Drift check",
        &["CREATE (:Person {uid: 'Person:1', name: 'Ada'})"],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;
    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/verify", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, clean) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/drift", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(clean["drifted"], false);

    // New source record drifts from the target snapshot.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (:Person {uid: 'Person:2', name: 'Bob'})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, drifted) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/drift", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(drifted["drifted"], true);

    // Drift checks never flip migration status.
    let (status, migration) =
        get_json(&app, format!("/api/v1/projects/{}/migrations/{}", pid, mid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(migration["status"], "verified");

    server.abort();
}

#[tokio::test]
async fn test_changes_feed_returns_watermarked_page() {
    let (app, _dir) = setup_test_app().await;
    let pid = seed_project_with_cypher(
        &app,
        "Changes feed",
        &[
            "CREATE (:Person {uid: 'Person:1', _updated_at: '2026-01-01T00:00:00Z'})",
            "CREATE (:Person {uid: 'Person:2', _updated_at: '2026-06-01T00:00:00Z'})",
            "CREATE (:Person {uid: 'Person:3'})",
        ],
    )
    .await;

    let (status, all) = get_json(&app, format!("/api/v1/projects/{}/changes", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["row_count"], 3);

    // Watermark filters timestamped rows; legacy rows without _updated_at
    // always count as changed.
    let (status, page) = get_json(
        &app,
        format!(
            "/api/v1/projects/{}/changes?since=2026-03-01T00:00:00Z",
            pid
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["row_count"], 2);
}

#[tokio::test]
async fn test_stable_rescan_has_no_skip_or_dupe_after_concurrent_write() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, fake_state) = start_fake_bolt().await;
    let (app, _dir) = setup_test_app().await;

    // Small batches force multi-page SKIP/LIMIT scans.
    let pid = seed_project_with_cypher(
        &app,
        "Stable scan",
        &[
            "CREATE (:Person {uid: 'Person:1'})",
            "CREATE (:Person {uid: 'Person:2'})",
            "CREATE (:Person {uid: 'Person:3'})",
        ],
    )
    .await;
    let cid = connect_fake_bolt(&app, &pid, &uri).await;
    let (status, plan) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/plan", pid),
        json!({"connection_id": cid, "batch_size": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Concurrent-style write between full load and delta rescan.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (:Person {uid: 'Person:0'})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, synced) = post_json(
        &app,
        format!("/api/v1/projects/{}/migrations/{}/execute", pid, mid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(synced["progress"]["sync_mode"], "delta");

    // All four uids land exactly once: ORDER BY scans neither skip the
    // early-sorting new row nor duplicate existing ones.
    let guard = fake_state.lock().await;
    let table = guard.nodes.get("Person").unwrap();
    assert_eq!(table.len(), 4);
    for uid in ["Person:0", "Person:1", "Person:2", "Person:3"] {
        assert!(table.contains_key(uid), "missing {}", uid);
    }

    server.abort();
}
