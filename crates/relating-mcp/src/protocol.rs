use crate::error::McpError;
use relating_core::ProjectId;
use relating_graph::RelateBackend;
use relating_metadata::DbPool;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MCP_SERVER_NAME: &str = "relating-mcp";
pub const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct McpState {
    pub pool: DbPool,
    pub graph: Arc<RelateBackend>,
    pub data_dir: PathBuf,
    /// HTTP bearer token snapshotted at boot (`RELATING_MCP_TOKEN`), like the
    /// API server's `AppState.api_token`. `None` disables the HTTP transport.
    /// The stdio transport is local-only and ignores this field.
    pub mcp_token: Option<String>,
}

impl McpState {
    pub fn project_graph_path(&self, id: &ProjectId) -> PathBuf {
        self.data_dir
            .join("projects")
            .join(id.to_string())
            .join("graph.rl")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn call(
        &self,
        state: &McpState,
        project: &ProjectId,
        args: &Value,
    ) -> Result<String, McpError>;
}

pub struct ToolRegistry {
    handlers: HashMap<String, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, handler: Arc<dyn ToolHandler>) {
        self.handlers
            .insert(handler.definition().name.clone(), handler);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> =
            self.handlers.values().map(|h| h.definition()).collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.handlers.get(name).cloned()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// MCP tool calls arrive as `{name, arguments}`; notifications carry no id.
pub fn parse_tool_call(params: &Value) -> Result<(String, Value), McpError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidArguments("Missing tool 'name'".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    Ok((name.to_string(), arguments))
}

pub fn text_content(text: String) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
    })
}

pub fn error_content(message: String) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": message}],
        "isError": true,
    })
}

pub const TOOL_TIMEOUT_SECS: u64 = 30;

/// Dispatch one JSON-RPC message. Returns None for notifications.
pub async fn dispatch_request(
    state: &McpState,
    project: &ProjectId,
    registry: &ToolRegistry,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    let id = match req.id.clone() {
        Some(id) => id,
        None => {
            // Notification: no response, but validate project scope exists.
            if req.method == "notifications/initialized" {
                let _ = relating_metadata::get_project(&state.pool, project).await;
            }
            return None;
        }
    };

    if req.jsonrpc != "2.0" {
        return Some(JsonRpcResponse::err(
            id,
            -32600,
            "Invalid JSON-RPC version; expected '2.0'".to_string(),
        ));
    }

    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::ok(
            id,
            serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": MCP_SERVER_NAME, "version": MCP_SERVER_VERSION},
            }),
        )),
        "ping" => Some(JsonRpcResponse::ok(id, serde_json::json!({}))),
        "tools/list" => {
            let tools: Vec<ToolDefinition> = registry.definitions();
            let tools_json = serde_json::to_value(&tools).unwrap_or(Value::Null);
            Some(JsonRpcResponse::ok(
                id,
                serde_json::json!({"tools": tools_json}),
            ))
        }
        "tools/call" => {
            let (name, args) = match parse_tool_call(&req.params) {
                Ok(parsed) => parsed,
                Err(e) => {
                    return Some(JsonRpcResponse::err(id, -32602, e.to_string()));
                }
            };
            let handler = match registry.get(&name) {
                Some(handler) => handler,
                None => {
                    return Some(JsonRpcResponse::err(
                        id,
                        -32602,
                        format!("Unknown tool '{}'", name),
                    ));
                }
            };
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
                handler.call(state, project, &args),
            )
            .await;
            match outcome {
                Ok(Ok(text)) => Some(JsonRpcResponse::ok(id, text_content(text))),
                Ok(Err(e)) => Some(JsonRpcResponse::ok(id, error_content(e.to_string()))),
                Err(_) => Some(JsonRpcResponse::ok(
                    id,
                    error_content(format!(
                        "Tool '{}' timed out after {}s",
                        name, TOOL_TIMEOUT_SECS
                    )),
                )),
            }
        }
        _ => Some(JsonRpcResponse::err(
            id,
            -32601,
            format!(
                "Method '{}' not supported (read-only tool server)",
                req.method
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::build_registry;
    use relating_core::{AiProviderConfig, Project, ProjectId};
    use relating_graph::GraphBackend;
    use relating_metadata::{init_in_memory_db, insert_project};

    async fn test_setup() -> (McpState, ProjectId) {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_in_memory_db().await.unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "MCP Test".to_string(),
            description: String::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ai_config: AiProviderConfig::mock(),
        };
        insert_project(&pool, &project).await.unwrap();
        // Keep the tempdir alive by leaking: state only needs the path.
        let data_dir = dir.path().to_path_buf();
        std::mem::forget(dir);
        let state = McpState {
            pool,
            graph: Arc::new(RelateBackend::new()),
            data_dir,
            mcp_token: None,
        };
        (state, project.id)
    }

    fn req(id: i64, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(id)),
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn test_initialize_and_list() {
        let (state, project) = test_setup().await;
        let registry = build_registry();

        let resp = dispatch_request(
            &state,
            &project,
            &registry,
            req(1, "initialize", Value::Null),
        )
        .await
        .unwrap();
        assert_eq!(
            resp.result.as_ref().unwrap()["protocolVersion"],
            MCP_PROTOCOL_VERSION
        );

        let resp = dispatch_request(
            &state,
            &project,
            &registry,
            req(2, "tools/list", Value::Null),
        )
        .await
        .unwrap();
        let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 12);
        assert!(tools.iter().any(|t| t["name"] == "read_cypher"));
    }

    #[tokio::test]
    async fn test_tool_call_and_unknown_tool() {
        let (state, project) = test_setup().await;
        let registry = build_registry();

        let resp = dispatch_request(
            &state,
            &project,
            &registry,
            req(
                1,
                "tools/call",
                serde_json::json!({"name": "list_projects"}),
            ),
        )
        .await
        .unwrap();
        let text = resp.result.as_ref().unwrap()["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("MCP Test"));

        let resp = dispatch_request(
            &state,
            &project,
            &registry,
            req(
                2,
                "tools/call",
                serde_json::json!({"name": "drop_database"}),
            ),
        )
        .await
        .unwrap();
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn test_notifications_get_no_response() {
        let (state, project) = test_setup().await;
        let registry = build_registry();
        let notif = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "notifications/initialized".to_string(),
            params: Value::Null,
        };
        assert!(dispatch_request(&state, &project, &registry, notif)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_scope_is_enforced() {
        let (state, _project) = test_setup().await;
        let registry = build_registry();
        let outsider = ProjectId::new();
        let resp = dispatch_request(
            &state,
            &outsider,
            &registry,
            req(
                1,
                "tools/call",
                serde_json::json!({"name": "get_project_summary"}),
            ),
        )
        .await
        .unwrap();
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not in scope"));
    }

    #[tokio::test]
    async fn test_read_cypher_rejects_writes() {
        let (state, project) = test_setup().await;
        let registry = build_registry();
        // Ensure the store exists so the failure is the write itself.
        state
            .graph
            .ensure_store(&state.project_graph_path(&project))
            .unwrap();
        let resp = dispatch_request(
            &state,
            &project,
            &registry,
            req(
                1,
                "tools/call",
                serde_json::json!({"name": "read_cypher", "arguments": {"query": "CREATE (n:Evil)"}}),
            ),
        )
        .await
        .unwrap();
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["isError"], true);
    }
}
