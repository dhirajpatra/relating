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

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_PROXY_PASSWORD";

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

async fn create_project(app: &axum::Router, name: &str) -> String {
    let (status, proj) =
        post_json(app, "/api/v1/projects".to_string(), json!({"name": name})).await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

async fn create_connection(app: &axum::Router, pid: &str, uri: &str) -> String {
    let (status, conn) = post_json(
        app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({"name": "fake", "uri": uri, "auth_env_var": TEST_PASSWORD_ENV}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    conn["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_query_targets_lists_local_and_aura_strings() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Targets").await;
    let (bolt_uri, handle, _state) = start_fake_bolt().await;
    let cid = create_connection(&app, &pid, &bolt_uri).await;

    let (status, targets) = get_json(&app, format!("/api/v1/projects/{}/query-targets", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(targets["local"]["connection_string"], "relate://local");
    assert_eq!(targets["local"]["backend"], "relate");
    assert_eq!(targets["aura"].as_array().unwrap().len(), 1);
    assert_eq!(
        targets["aura"][0]["connection_string"],
        format!("aura://{}", cid)
    );
    handle.abort();
}

#[tokio::test]
async fn test_proxied_read_returns_local_shaped_result() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Proxy Read").await;
    let (bolt_uri, handle, _state) = start_fake_bolt().await;
    let cid = create_connection(&app, &pid, &bolt_uri).await;

    // Same statement shape as a local read; routed to the fake Aura target.
    let (status, result) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/read", pid),
        json!({
            "query": "MATCH (n:Person) RETURN count(n) AS c",
            "params": {},
            "connection_id": cid,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {}", result);
    assert_eq!(result["columns"], json!(["c"]));
    assert_eq!(result["row_count"], 1);
    assert_eq!(result["rows"][0]["c"], 0);
    handle.abort();
}

#[tokio::test]
async fn test_proxied_read_rejects_unknown_and_foreign_connections() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Proxy Scope").await;
    let other = create_project(&app, "Proxy Other").await;
    let (bolt_uri, handle, _state) = start_fake_bolt().await;
    let foreign_cid = create_connection(&app, &other, &bolt_uri).await;

    for conn_id in [
        "00000000-0000-0000-0000-000000000000".to_string(),
        foreign_cid,
    ] {
        let (status, _) = post_json(
            &app,
            format!("/api/v1/projects/{}/cypher/read", pid),
            json!({"query": "RETURN 1", "connection_id": conn_id}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    handle.abort();
}

#[tokio::test]
async fn test_proxied_write_requires_explicit_confirmation() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Proxy Write").await;
    let (bolt_uri, handle, _state) = start_fake_bolt().await;
    let cid = create_connection(&app, &pid, &bolt_uri).await;

    // Without confirmation the remote write is refused and nothing is sent.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({
            "query": "UNWIND $batch AS row MERGE (n:`Person` {uid: row.uid})",
            "params": {"batch": [{"uid": "Person:1", "props": {}}]},
            "connection_id": cid,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("confirm_remote_write"));

    // With confirmation the write runs and returns the local result shape.
    let (status, result) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({
            "query": "UNWIND $batch AS row MERGE (n:`Person` {uid: row.uid})",
            "params": {"batch": [{"uid": "Person:1", "props": {}}]},
            "connection_id": cid,
            "confirm_remote_write": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {}", result);
    assert_eq!(result["nodes_created"], 1);
    assert!(result.get("elapsed_ms").is_some());
    handle.abort();
}
