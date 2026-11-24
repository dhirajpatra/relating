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

#[tokio::test]
async fn test_debug_trace_full_pipeline() {
    let (app, _dir) = setup_test_app().await;

    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Debug R5"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    // Source -> embeddings -> ontology -> ingest: builds provenance end to end.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({
            "filename": "staff.csv",
            "content": "name,role\nAda,Engineer\nBob,Manager",
            "content_type": "text/csv"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/embeddings", pid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, ontology) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/propose", pid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let oid = ontology["id"].as_str().unwrap();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/approve", pid),
        json!({"ontology_id": oid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, ingest) =
        post_json(&app, format!("/api/v1/projects/{}/ingest", pid), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ingest["nodes_created"], 2);

    // Debug trace exposes every pipeline stage.
    let (status, debug) = post_json(
        &app,
        format!("/api/v1/projects/{}/debug/trace", pid),
        json!({"question": "Who is Ada?", "ai_hypothesis": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(debug["trace"]["grounded"], true);
    assert!(!debug["trace"]["passages"].as_array().unwrap().is_empty());
    assert!(!debug["trace"]["ranking"].as_array().unwrap().is_empty());
    assert!(debug["trace"]["ranking"][0]["vector_rank"].is_number());
    assert!(debug["trace"]["ranking"][0]["keyword_rank"].is_number());
    // At least one passage links to graph entities via provenance.
    let linked = debug["trace"]["graph_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| !l["nodes"].as_array().unwrap().is_empty());
    assert!(linked, "expected provenance-linked nodes");
    assert_eq!(debug["trace"]["ontology_approved"], true);
    assert!(!debug["trace"]["answer"].as_str().unwrap().is_empty());
    assert!(!debug["trace"]["evidence_preview"]
        .as_str()
        .unwrap()
        .is_empty());
    // Diagnosis always reports rules and likely causes.
    assert!(!debug["diagnosis"]["rules"].as_array().unwrap().is_empty());
    assert!(!debug["diagnosis"]["likely_causes"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(debug["diagnosis"].get("ai_hypothesis").is_none());

    // AI hypothesis included on request (mock provider).
    let (status, debug) = post_json(
        &app,
        format!("/api/v1/projects/{}/debug/trace", pid),
        json!({"question": "Who is Ada?", "ai_hypothesis": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(debug["diagnosis"]["ai_hypothesis"]["text"].is_string());
}

#[tokio::test]
async fn test_debug_trace_without_evidence() {
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Debug Empty"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    let (status, debug) = post_json(
        &app,
        format!("/api/v1/projects/{}/debug/trace", pid),
        json!({"question": "Anything?", "ai_hypothesis": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(debug["trace"]["grounded"], false);
    assert!(debug["trace"]["passages"].as_array().unwrap().is_empty());
    let codes: Vec<String> = debug["diagnosis"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["code"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(codes.contains(&"DBG-E001".to_string()));

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/debug/trace", pid),
        json!({"question": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
