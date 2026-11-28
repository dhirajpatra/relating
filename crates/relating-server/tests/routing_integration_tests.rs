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

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_ROUTING_PASSWORD";

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

#[tokio::test]
async fn test_routing_scheme_follows_routing_table() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (fake_uri, _handle, state) = start_fake_bolt().await;
    // Same fake, routing scheme: the client must ROUTE, then reconnect.
    let routing_uri = fake_uri.replacen("bolt://", "neo4j://", 1);

    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Route"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "routed",
            "uri": routing_uri,
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura/{}/test", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["reachable"], true);

    // The fake answered ROUTE (pointing at itself) and the client followed
    // it with a second connection instead of using the first directly.
    assert_eq!(state.lock().await.route_calls, 1);
}

#[tokio::test]
async fn test_direct_scheme_skips_routing() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (fake_uri, _handle, state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Direct"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "direct",
            "uri": fake_uri,
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura/{}/test", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["reachable"], true);
    assert_eq!(state.lock().await.route_calls, 0);
}
