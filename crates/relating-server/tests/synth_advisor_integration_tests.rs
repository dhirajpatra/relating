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

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_R6_PASSWORD";

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
    let json: Value = serde_json::from_slice(&body).unwrap();
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

async fn create_project(app: &axum::Router) -> String {
    let (status, proj) =
        post_json(app, "/api/v1/projects".to_string(), json!({"name": "R6"})).await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

async fn seed_graph(app: &axum::Router, pid: &str) {
    let (status, _) = post_json(
        app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (a:Person {uid: 'Person:1', name: 'Ada', age: 36})-[:KNOWS]->(b:Person {uid: 'Person:2', name: 'Bob', age: 41})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_synth_profile_generate_materialize() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;
    seed_graph(&app, &pid).await;

    let (status, profile) = post_json(
        &app,
        format!("/api/v1/projects/{}/synth/profile", pid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["total_nodes"], 2);
    assert_eq!(profile["labels"][0]["label"], "Person");

    // Deterministic generation with every edge case.
    let (status, gen) = post_json(
        &app,
        format!("/api/v1/projects/{}/synth/generate", pid),
        json!({
            "scale": 1.0, "seed": 7,
            "edge_cases": ["duplicates", "orphans", "cycle", "invalid_values"],
            "include_dataset": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(gen["stats"]["per_label"]["Person"], 2 + 1 + 3 + 3 + 1);
    assert_eq!(
        gen["stats"]["edge_cases_applied"].as_array().unwrap().len(),
        4
    );
    assert!(!gen["dataset"]["nodes"].as_array().unwrap().is_empty());

    // Same seed regenerates identically.
    let (status, gen2) = post_json(
        &app,
        format!("/api/v1/projects/{}/synth/generate", pid),
        json!({"scale": 1.0, "seed": 7, "include_dataset": true}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(gen2["dataset"]["nodes"][0], gen["dataset"]["nodes"][0]);

    // Materialize into an isolated sibling project.
    let (status, gen) = post_json(
        &app,
        format!("/api/v1/projects/{}/synth/generate", pid),
        json!({"scale": 0.5, "seed": 1, "materialize": true, "target_name": "fixture"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let target_id = gen["target_project_id"].as_str().unwrap();
    assert!(gen["materialized"]["nodes_created"].as_i64().unwrap() > 0);

    let (status, detail) = get_json(&app, format!("/api/v1/projects/{}", target_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["schema"]["node_count"].as_u64().unwrap() > 0);

    // Source project is untouched by materialization.
    let (status, detail) = get_json(&app, format!("/api/v1/projects/{}", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["schema"]["node_count"], 2);

    // Invalid scale is rejected.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/synth/generate", pid),
        json!({"scale": 2.0}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["code"], "SYNTH_ERROR");
}

#[tokio::test]
async fn test_advisor_static_and_measured() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;
    seed_graph(&app, &pid).await;

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({"name": "fake", "uri": uri, "auth_env_var": TEST_PASSWORD_ENV}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({
            "queries": [
                {"name": "open scan", "cypher": "MATCH (a), (b) RETURN a, b"},
                {"name": "lookup", "cypher": "MATCH (p:Person {uid: $uid}) RETURN p LIMIT 1"}
            ],
            "connection_id": cid,
            "measure": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let open = &report["queries"][0];
    let rules: Vec<String> = open["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["rule_id"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(rules.contains(&"ADV-UNBOUNDED".to_string()));
    assert!(rules.contains(&"ADV-CARTESIAN".to_string()));

    let lookup_rules: Vec<String> = report["queries"][1]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["rule_id"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(lookup_rules.contains(&"ADV-INDEX".to_string()));

    assert!(!report["structure"].as_array().unwrap().is_empty());
    assert!(report["sizing"]["tier_hint"].is_string());
    assert!(!report["sizing"]["caveats"].as_array().unwrap().is_empty());
    assert!(report["measurements"]["plans"].as_array().unwrap().len() == 2);
    assert_eq!(report["measurements"]["plans"][0]["planned"], true);

    // measure=true without a connection is a client error.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({"queries": [], "measure": true}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    server.abort();
}

#[tokio::test]
async fn test_advisor_vector_sizing_checks_tier_fit() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    // Without vectors the sizing shape is unchanged (no vectors section).
    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({"queries": []}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(report["sizing"].get("vectors").is_none());

    // With vectors the advisor folds bytes into storage and checks fit.
    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({
            "queries": [],
            "vectors": [{"profile_name": "docs", "dimensions": 768, "vector_count": 10000}],
            "aura_tier": "free"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["sizing"]["vectors"]["tier_id"], "free");
    assert_eq!(
        report["sizing"]["vectors"]["total_vector_bytes"],
        10_000 * 768 * 4
    );
    assert_eq!(
        report["sizing"]["vectors"]["fits"][0]["verdict"],
        "SUPPORTED"
    );

    // Oversized dimensions are flagged without needing a tier.
    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({
            "queries": [],
            "vectors": [{"dimensions": 8192, "vector_count": 10}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report["sizing"]["vectors"]["fits"][0]["verdict"],
        "UNSUPPORTED"
    );

    // Unknown tiers are rejected with the valid choices.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/advisor/analyze", pid),
        json!({
            "queries": [],
            "vectors": [{"dimensions": 128, "vector_count": 10}],
            "aura_tier": "mystery-tier"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("mystery-tier"));
}
