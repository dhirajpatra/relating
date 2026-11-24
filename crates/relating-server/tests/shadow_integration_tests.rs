use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_server::{create_router, AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

mod common;
use common::start_fake_bolt;

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_SHADOW_PASSWORD";

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: None,
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

async fn create_project(app: &axum::Router) -> String {
    let (status, proj) = post_json(
        app,
        "/api/v1/projects".to_string(),
        json!({"name": "Shadow R5"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_shadow_replay_compares_backends() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    // Seed the source graph with two cities.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (a:City {uid: 'City:1', name: 'Lisbon'}) CREATE (b:City {uid: 'City:2', name: 'Porto'})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({"name": "fake", "uri": uri, "auth_env_var": TEST_PASSWORD_ENV}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    // Empty-vs-empty matches; divergent counts diff; injected failure diffs.
    let (status, resp) = post_json(
        &app,
        format!("/api/v1/projects/{}/shadow/runs", pid),
        json!({
            "connection_id": cid,
            "queries": [
                {"name": "empty both", "cypher": "MATCH (n:Nowhere) RETURN n"},
                {"name": "city count", "cypher": "MATCH (n:City) RETURN count(n)"},
                {"name": "target blows up", "cypher": "MATCH (n) RETURN n // __fail__"}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let comparisons = resp["report"]["comparisons"].as_array().unwrap();
    assert_eq!(comparisons.len(), 3);
    assert_eq!(comparisons[0]["verdict"], "MATCH");
    assert_eq!(comparisons[1]["verdict"], "COUNT_DIFF");
    assert_eq!(comparisons[2]["verdict"], "ERROR_DIFF");
    assert_eq!(resp["report"]["summary"]["total"], 3);
    assert!(
        resp["report"]["summary"]["confidence"] == "LOW"
            || resp["report"]["summary"]["confidence"] == "MEDIUM"
    );

    // Params are redacted in the echo by default.
    let (status, resp) = post_json(
        &app,
        format!("/api/v1/projects/{}/shadow/runs", pid),
        json!({
            "connection_id": cid,
            "queries": [
                {"name": "param", "cypher": "MATCH (n:City {name: $name}) RETURN n", "params": {"name": "Lisbon"}}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["queries"][0]["params"]["name"], "<string redacted>");

    server.abort();
}

#[tokio::test]
async fn test_shadow_refuses_writes() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;
    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({"name": "fake", "uri": uri, "auth_env_var": TEST_PASSWORD_ENV}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    for bad in [
        "CREATE (n:Evil)",
        "MATCH (n) SET n.x = 1",
        "MATCH (n) DELETE n",
    ] {
        let (status, err) = post_json(
            &app,
            format!("/api/v1/projects/{}/shadow/runs", pid),
            json!({
                "connection_id": cid,
                "queries": [{"name": "evil", "cypher": bad}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "query: {}", bad);
        assert_eq!(err["error"]["code"], "SHADOW_ERROR");
    }

    // Empty suites are rejected too.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/shadow/runs", pid),
        json!({"connection_id": cid, "queries": []}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    server.abort();
}
