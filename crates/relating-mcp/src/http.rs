use crate::protocol::{dispatch_request, JsonRpcRequest, McpState};
use crate::tools::build_registry;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use relating_core::ProjectId;

pub const MCP_TOKEN_ENV_VAR: &str = "RELATING_MCP_TOKEN";
pub const MCP_PROJECT_HEADER: &str = "x-relating-project";

pub fn create_mcp_router(state: McpState) -> axum::Router<()> {
    let registry = std::sync::Arc::new(build_registry());
    axum::Router::new()
        .route("/mcp", post(mcp_post_handler))
        .route("/mcp/health", get(mcp_health_handler))
        .with_state((state, registry))
}

type McpRouterState = (McpState, std::sync::Arc<crate::protocol::ToolRegistry>);

pub async fn mcp_health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "server": crate::protocol::MCP_SERVER_NAME,
        "version": crate::protocol::MCP_SERVER_VERSION,
        "protocol_version": crate::protocol::MCP_PROTOCOL_VERSION,
        "tools": 12,
        "mode": "read-only",
    }))
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(|s| s.to_string())
}

pub async fn mcp_post_handler(
    State((state, registry)): State<McpRouterState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // HTTP transport is disabled until an operator configures a token.
    let expected = match std::env::var(MCP_TOKEN_ENV_VAR) {
        Ok(token) if !token.is_empty() => token,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "MCP HTTP transport is disabled: set RELATING_MCP_TOKEN to enable it"
                })),
            )
                .into_response();
        }
    };
    match bearer_token(&headers) {
        Some(token) if token == expected => {}
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid or missing bearer token"})),
            )
                .into_response();
        }
    }

    let project = match headers
        .get(MCP_PROJECT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| ProjectId::parse(s).ok())
    {
        Some(project) => project,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or invalid x-relating-project header"})),
            )
                .into_response();
        }
    };

    let req: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(req) => req,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid JSON-RPC body: {}", e)})),
            )
                .into_response();
        }
    };

    match dispatch_request(&state, &project, &registry, req).await {
        Some(response) => Json(response).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use relating_graph::RelateBackend;
    use relating_metadata::init_in_memory_db;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn test_router() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_in_memory_db().await.unwrap();
        let state = McpState {
            pool,
            graph: Arc::new(RelateBackend::new()),
            data_dir: dir.path().to_path_buf(),
        };
        (create_mcp_router(state), dir)
    }

    #[tokio::test]
    async fn test_mcp_health_is_public() {
        let (app, _dir) = test_router().await;
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
    }

    #[tokio::test]
    async fn test_mcp_post_rejects_missing_token() {
        let (app, _dir) = test_router().await;
        // With no RELATING_MCP_TOKEN configured the transport stays disabled.
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer anything")
                    .header("x-relating-project", "00000000-0000-0000-0000-000000000000")
                    .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            res.status() == StatusCode::FORBIDDEN || res.status() == StatusCode::UNAUTHORIZED,
            "status was {}",
            res.status()
        );
    }
}
