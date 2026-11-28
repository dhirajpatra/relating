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

#[tokio::test]
async fn test_health_endpoint() {
    let (app, _dir) = setup_test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["product"], "Relating");
}

#[tokio::test]
async fn test_full_project_and_cypher_lifecycle() {
    let (app, _dir) = setup_test_app().await;

    // 1. Create a project
    let create_payload = json!({
        "name": "Integration Test Graph",
        "description": "Testing project creation and Cypher queries"
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/projects")
                .header("Content-Type", "application/json")
                .body(Body::from(create_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let project: Value = serde_json::from_slice(&body).unwrap();
    let project_id = project["id"].as_str().unwrap();
    assert_eq!(project["name"], "Integration Test Graph");

    // 2. Test AI provider health check on project
    let ai_test_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/projects/{}/ai/test", project_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(ai_test_res.status(), StatusCode::OK);
    let ai_body = ai_test_res.into_body().collect().await.unwrap().to_bytes();
    let ai_report: Value = serde_json::from_slice(&ai_body).unwrap();
    assert_eq!(ai_report["healthy"], true);
    assert_eq!(ai_report["provider"], "mock");

    // 3. Write Cypher into the project graph
    let cypher_write_payload = json!({
        "query": "CREATE (c:Company {name: $company_name})-[:OWNS]->(p:Product {name: $product_name})",
        "params": {
            "company_name": "Relating Corp",
            "product_name": "Graph Workbench"
        }
    });

    let write_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/projects/{}/cypher/write", project_id))
                .header("Content-Type", "application/json")
                .body(Body::from(cypher_write_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(write_res.status(), StatusCode::OK);
    let write_body = write_res.into_body().collect().await.unwrap().to_bytes();
    let write_json: Value = serde_json::from_slice(&write_body).unwrap();
    assert_eq!(write_json["nodes_created"], 2);
    assert_eq!(write_json["relationships_created"], 1);

    // 4. Query read Cypher with parameters
    let cypher_read_payload = json!({
        "query": "MATCH (c:Company)-[:OWNS]->(p:Product) RETURN c.name AS company, p.name AS product",
        "params": {}
    });

    let read_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/projects/{}/cypher/read", project_id))
                .header("Content-Type", "application/json")
                .body(Body::from(cypher_read_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(read_res.status(), StatusCode::OK);
    let read_body = read_res.into_body().collect().await.unwrap().to_bytes();
    let read_json: Value = serde_json::from_slice(&read_body).unwrap();
    assert_eq!(read_json["row_count"], 1);
    assert_eq!(read_json["rows"][0]["company"], "Relating Corp");
    assert_eq!(read_json["rows"][0]["product"], "Graph Workbench");

    // 5. Inspect project detail and schema counts
    let detail_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/projects/{}", project_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(detail_res.status(), StatusCode::OK);
    let detail_body = detail_res.into_body().collect().await.unwrap().to_bytes();
    let detail_json: Value = serde_json::from_slice(&detail_body).unwrap();
    assert_eq!(detail_json["schema"]["node_count"], 2);
    assert_eq!(detail_json["schema"]["relationship_count"], 1);

    // 6. Delete project
    let delete_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/projects/{}", project_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(delete_res.status(), StatusCode::NO_CONTENT);

    // Verify it is gone
    let after_delete_res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/projects/{}", project_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(after_delete_res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_api_token_gates_v1_routes_but_not_health() {
    let dir = tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: Some("test-token-123".to_string()),
        auth: AuthState::default(),
    };
    let app = create_router(state);

    // Health stays open with no credentials.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // API routes without a token are refused.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // A wrong token is refused too.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .header("Authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // The right token passes through to the handler.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .header("Authorization", "Bearer test-token-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
