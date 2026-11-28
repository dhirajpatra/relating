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

async fn put_json(app: &axum::Router, uri: String, payload: Value) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
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
    let (status, proj) = post_json(
        app,
        "/api/v1/projects".to_string(),
        json!({"name": "Schema R2"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_schema_export_revision_diff_apply_lifecycle() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/cypher/write", pid),
        json!({"query": "CREATE (n:Person {uid: 'Person:1', name: 'Ada'})"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 1. Export produces the as-code file set.
    let (status, export) = get_json(&app, format!("/api/v1/projects/{}/schema/export", pid)).await;
    assert_eq!(status, StatusCode::OK);
    let paths: Vec<String> = export["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str().map(|s| s.to_string()))
        .collect();
    for expected in [
        "graph/schema.json",
        "graph/ontology.json",
        "graph/constraints.cypher",
        "graph/indexes.cypher",
        "tests/graph_contracts.json",
    ] {
        assert!(
            paths.contains(&expected.to_string()),
            "missing {}",
            expected
        );
    }
    assert_eq!(export["document"]["format_version"], "relating-schema-v1");
    let constraints = export["document"]["constraints"].as_array().unwrap();
    assert!(constraints.iter().any(|c| c["label"] == "Person"));

    // 2. Snapshot a revision; identical snapshots are idempotent.
    let (status, snap1) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/revisions", pid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(snap1["revision"]["version"], 1);
    assert_eq!(snap1["created"], true);

    let (status, snap2) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/revisions", pid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snap2["created"], false);
    assert_eq!(snap2["revision"]["version"], 1);

    let (status, revisions) =
        get_json(&app, format!("/api/v1/projects/{}/schema/revisions", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revisions.as_array().unwrap().len(), 1);

    let (status, rev1) =
        get_json(&app, format!("/api/v1/projects/{}/schema/revisions/1", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rev1["version"], 1);

    // 3. Retrieval config round-trips.
    let (status, config) = put_json(
        &app,
        format!("/api/v1/projects/{}/retrieval-config", pid),
        json!({"default_strategy": "keyword", "default_top_k": 3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(config["configured"], true);
    assert_eq!(config["default_strategy"], "keyword");

    // 4. Additive diff is safe and proposes a versioned filename.
    let mut additive = export["document"].clone();
    additive["ontology"]["entities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "label": "Order",
            "display_name": "Order",
            "properties": [{"name": "uid", "data_type": "string", "is_primary_key": true}],
            "description": ""
        }));
    let (status, diff) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/diff", pid),
        json!({"to_document": additive}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(diff["diff"]["breaking_count"], 0);
    assert!(diff["proposal"]["filename"]
        .as_str()
        .unwrap()
        .starts_with("V002__"));

    // 5. Removing the only entity is breaking with manual steps.
    let mut breaking = export["document"].clone();
    breaking["ontology"]["entities"] = json!([]);
    // Keep the uid constraint removal too by clearing constraints.
    breaking["constraints"] = json!([]);
    let (status, diff) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/diff", pid),
        json!({"to_document": breaking}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(diff["diff"]["breaking_count"].as_u64().unwrap() >= 1);
    assert!(!diff["proposal"]["steps"].as_array().unwrap().is_empty());

    // 6. Apply records v2 with a passing conformance report.
    let (status, applied) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/apply", pid),
        json!({"document": additive}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(applied["revision"]["version"], 2);
    assert_eq!(applied["conformance"]["passed"], true);

    // 7. Apply records non-conformant intent too, but reports it honestly.
    // The live Person node has no email, so requiring it must fail conformance.
    let mut strict = additive.clone();
    strict["ontology"]["entities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "label": "Person",
            "display_name": "Person",
            "description": "",
            "properties": [
                {"name": "uid", "data_type": "string", "is_primary_key": true},
                {"name": "email", "data_type": "string", "is_primary_key": false}
            ]
        }));
    let (status, applied) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/apply", pid),
        json!({"document": strict}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(applied["revision"]["version"], 3);
    assert_eq!(applied["conformance"]["passed"], false);

    // 8. Revision-to-revision diff works.
    let (status, diff) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/diff", pid),
        json!({"from_version": 1, "to_version": 3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!diff["diff"]["changes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_schema_validation_rejects_bad_documents() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    // Diff with no revisions demands a snapshot first.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/diff", pid),
        json!({"to_version": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("snapshot"));

    // Unknown revision versions 404.
    let (status, _) = get_json(
        &app,
        format!("/api/v1/projects/{}/schema/revisions/99", pid),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Bad format version is rejected.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/apply", pid),
        json!({"document": {"format_version": "v0"}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["code"], "SCHEMA_ERROR");

    // Bad property types are rejected.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/schema/apply", pid),
        json!({"document": {
            "format_version": "relating-schema-v1",
            "ontology": {"entities": [{
                "label": "Person", "display_name": "P", "description": "",
                "properties": [{"name": "born", "data_type": "datetime"}]
            }], "relationships": []}
        }}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Retrieval config ranges are enforced.
    for bad_top_k in [0, 99] {
        let (status, _) = put_json(
            &app,
            format!("/api/v1/projects/{}/retrieval-config", pid),
            json!({"default_top_k": bad_top_k}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn test_retrieval_uses_stored_defaults() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

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

    // Defaults apply when the request omits strategy and top_k.
    let (status, _) = put_json(
        &app,
        format!("/api/v1/projects/{}/retrieval-config", pid),
        json!({"default_strategy": "keyword", "default_top_k": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, trace) = post_json(
        &app,
        format!("/api/v1/projects/{}/retrieval", pid),
        json!({"query": "Ada"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trace["strategy"], "keyword");
    assert_eq!(trace["top_k"], 1);

    // Explicit request values still win over stored defaults.
    let (status, trace) = post_json(
        &app,
        format!("/api/v1/projects/{}/retrieval", pid),
        json!({"query": "Ada", "strategy": "vector", "top_k": 2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trace["strategy"], "vector");
    assert_eq!(trace["top_k"], 2);
}
