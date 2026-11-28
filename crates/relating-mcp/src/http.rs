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

/// Read the MCP HTTP token from the environment: trimmed, empty as unset.
/// Called once at router construction so tests stay hermetic.
pub fn mcp_token_from_env() -> Option<String> {
    std::env::var(MCP_TOKEN_ENV_VAR)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn create_mcp_router(state: McpState) -> axum::Router<()> {
    let registry = std::sync::Arc::new(build_registry());
    axum::Router::new()
        .route("/mcp", post(mcp_post_handler))
        .route("/mcp/health", get(mcp_health_handler))
        .with_state((state, registry))
}

type McpRouterState = (McpState, std::sync::Arc<crate::protocol::ToolRegistry>);

/// True when an opaque login session token is live and its user holds any
/// grant (global or project) for this project. Fixes the old
/// same-token-unlocks-all scoping for SSO callers.
async fn session_authorizes_project(
    state: &McpState,
    presented: &str,
    project: &ProjectId,
) -> bool {
    let Ok(Some(session)) =
        relating_metadata::find_session_by_hash(&state.pool, &relating_core::sha256_hex(presented))
            .await
    else {
        return false;
    };
    if session.expires_at <= chrono::Utc::now() {
        return false;
    }
    let project_id = project.to_string();
    let global = relating_metadata::user_global_role(&state.pool, &session.user_id).await;
    let scoped =
        relating_metadata::user_project_role(&state.pool, &session.user_id, &project_id).await;
    matches!(global, Ok(Some(_))) || matches!(scoped, Ok(Some(_)))
}

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

    // Static token (constant-time) is a global admin; login session tokens
    // are scoped to the caller's grants for this project (MCP tools are
    // read-only, so any grant suffices).
    let presented = bearer_token(&headers).unwrap_or_default();
    let mut authorized = matches!(
        state.mcp_token.as_deref(),
        Some(expected) if relating_core::constant_time_eq(&presented, expected)
    );
    if !authorized && !presented.is_empty() {
        authorized = session_authorizes_project(&state, &presented, &project).await;
    }
    if !authorized {
        let (status, message) = match &state.mcp_token {
            None if presented.is_empty() => (
                StatusCode::FORBIDDEN,
                "MCP HTTP transport is disabled: set RELATING_MCP_TOKEN to enable it",
            ),
            _ => (StatusCode::UNAUTHORIZED, "Invalid or missing bearer token"),
        };
        return (status, Json(serde_json::json!({"error": message}))).into_response();
    }

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
            // Hermetic: no static token here regardless of process env.
            mcp_token: None,
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
