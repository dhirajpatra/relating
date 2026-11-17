use crate::protocol::{dispatch_request, JsonRpcRequest, JsonRpcResponse, McpState, ToolRegistry};
use crate::tools::build_registry;
use relating_core::ProjectId;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Newline-delimited JSON-RPC over stdio (the MCP local-agent transport).
/// Logs go to stderr; stdout carries only protocol messages.
pub async fn run_stdio(state: McpState, project: ProjectId) -> Result<(), String> {
    let registry = build_registry();
    run_stdio_with_registry(state, project, &registry).await
}

pub async fn run_stdio_with_registry(
    state: McpState,
    project: ProjectId,
    registry: &ToolRegistry,
) -> Result<(), String> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|e| format!("Failed to read stdin: {}", e))?;
        let line = match line {
            Some(line) => line,
            None => break, // EOF: agent disconnected.
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(req) => dispatch_request(&state, &project, registry, req).await,
            Err(e) => Some(JsonRpcResponse::err(
                serde_json::Value::Null,
                -32700,
                format!("Parse error: {}", e),
            )),
        };
        if let Some(response) = response {
            let mut out = serde_json::to_string(&response)
                .map_err(|e| format!("Failed to serialize response: {}", e))?;
            out.push('\n');
            stdout
                .write_all(out.as_bytes())
                .await
                .map_err(|e| format!("Failed to write stdout: {}", e))?;
            stdout
                .flush()
                .await
                .map_err(|e| format!("Failed to flush stdout: {}", e))?;
        }
    }
    Ok(())
}
