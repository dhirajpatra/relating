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

const MCP_TOKEN: &str = "test-mcp-token-12345";

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

async fn mcp_call(
    app: &axum::Router,
    project_id: &str,
    payload: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Content-Type", "application/json")
        .header("x-relating-project", project_id);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(payload.to_string())).unwrap())
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

async fn create_project_with_node(app: &axum::Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/projects")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({"name": "MCP HTTP"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let proj: Value = serde_json::from_slice(&body).unwrap();
    let pid = proj["id"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/projects/{}/cypher/write", pid))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({"query": "CREATE (n:City {name: 'Lisbon'})"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    pid
}

#[tokio::test]
async fn test_mcp_http_initialize_list_and_call() {
    std::env::set_var("RELATING_MCP_TOKEN", MCP_TOKEN);
    let (app, _dir) = setup_test_app().await;
    let pid = create_project_with_node(&app).await;

    let (status, resp) = mcp_call(
        &app,
        &pid,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(resp["result"]["serverInfo"]["name"], "relating-mcp");

    let (status, resp) = mcp_call(
        &app,
        &pid,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["result"]["tools"].as_array().unwrap().len(), 12);

    let (status, resp) = mcp_call(
        &app,
        &pid,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "read_cypher", "arguments": {"query": "MATCH (c:City) RETURN c.name AS city"}}
        }),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Lisbon"), "unexpected tool text: {}", text);

    let (status, resp) = mcp_call(
        &app,
        &pid,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "get_graph_schema", "arguments": {}}
        }),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("City"), "unexpected schema text: {}", text);
}

#[tokio::test]
async fn test_mcp_http_auth_and_scope() {
    std::env::set_var("RELATING_MCP_TOKEN", MCP_TOKEN);
    let (app, _dir) = setup_test_app().await;
    let pid = create_project_with_node(&app).await;

    // Missing token is rejected.
    let (status, _) = mcp_call(
        &app,
        &pid,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Wrong token is rejected.
    let (status, _) = mcp_call(
        &app,
        &pid,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
        Some("wrong-token"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Missing project header is rejected.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", MCP_TOKEN))
                .body(Body::from(
                    json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Unknown project yields a tool-level scope error, not data.
    let (status, resp) = mcp_call(
        &app,
        "00000000-0000-0000-0000-000000000000",
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "list_projects", "arguments": {}}
        }),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["result"]["isError"], true);

    // Unknown tools are a JSON-RPC error.
    let (status, resp) = mcp_call(
        &app,
        &pid,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "write_cypher", "arguments": {}}
        }),
        Some(MCP_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.get("error").is_some());

    // Health is public and reports read-only mode.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let health: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(health["mode"], "read-only");
    assert_eq!(health["tools"], 12);
}
