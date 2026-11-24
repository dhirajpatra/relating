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

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_PORT_PASSWORD";

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
        json!({"name": "Port R3"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

fn fixture_repo() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        "from neo4j import GraphDatabase\n\
         URI = \"bolt://localhost:7687\"\n\
         def friends(tx, name):\n    \
         return tx.run(\"MATCH (p:Person {name: $name})-[:KNOWS]->(f) RETURN f.name\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("bulk.cypher"),
        "USING PERIODIC COMMIT 500\nLOAD CSV FROM 'file:///data.csv' AS row\nCREATE (n:Import) SET n += row;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("legacy.cypher"),
        "CREATE CONSTRAINT ON (n:Person) ASSERT n.uid IS UNIQUE;\n",
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn test_scan_reports_findings_and_remediation() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;
    let repo = fixture_repo();

    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/portability/scan", pid),
        json!({"repo_path": repo.path().to_string_lossy()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["summary"]["files_scanned"], 3);
    // 1 python query + 1 bulk statement + 1 legacy statement.
    assert_eq!(report["summary"]["queries_total"], 3);

    let findings = report["findings"].as_array().unwrap();
    let periodic = findings
        .iter()
        .find(|f| f["rule_id"] == "R-PERIODIC")
        .unwrap();
    assert_eq!(periodic["verdict"], "NEEDS_REWRITE");
    assert!(periodic["remediation"].as_str().unwrap().contains("UNWIND"));

    let driver = findings
        .iter()
        .find(|f| f["rule_id"] == "R-DRIVER")
        .unwrap();
    assert_eq!(driver["verdict"], "PORTABLE");

    let ddl = findings
        .iter()
        .find(|f| f["rule_id"] == "R-DDL-OLD")
        .unwrap();
    assert_eq!(ddl["verdict"], "TRANSFORM");
    assert!(ddl["remediation"].as_str().unwrap().contains("REQUIRE"));

    assert_eq!(report["breaking"].as_array().unwrap().len(), 1);
    assert_eq!(report["tests"].as_array().unwrap().len(), 3);
    // The bulk CREATE is write-shaped: its generated test must be fenced.
    let bulk_test = report["tests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["file"] == "bulk.cypher")
        .unwrap();
    assert_eq!(bulk_test["read_only"], false);
}

#[tokio::test]
async fn test_scan_validation() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/portability/scan", pid),
        json!({"repo_path": "/nonexistent-xyz-123"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["code"], "PORTABILITY_ERROR");

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/portability/scan", pid),
        json!({"repo_path": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_run_tests_against_fake_bolt() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _state) = start_fake_bolt().await;

    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;
    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connections/aura", pid),
        json!({
            "name": "fake",
            "uri": uri,
            "auth_env_var": TEST_PASSWORD_ENV
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap();

    let (status, results) = post_json(
        &app,
        format!("/api/v1/projects/{}/portability/run-tests", pid),
        json!({
            "connection_id": cid,
            "tests": [
                {"name": "simple read", "cypher": "RETURN 1 AS one", "read_only": true,
                 "assertion": {"type": "row_count_eq", "expected": 0}},
                {"name": "write fenced by explain", "cypher": "CREATE (n:Tmp)", "read_only": false,
                 "assertion": {"type": "no_error"}},
                {"name": "deliberate mismatch", "cypher": "RETURN 1", "read_only": true,
                 "assertion": {"type": "row_count_eq", "expected": 5}}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(results["passed"], 2);
    assert_eq!(results["failed"], 1);
    assert_eq!(results["results"][2]["passed"], false);

    // Empty and oversized suites are rejected.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/portability/run-tests", pid),
        json!({"connection_id": cid, "tests": []}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    server.abort();
}
