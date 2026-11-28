pub mod error;
pub mod http;
pub mod protocol;
pub mod stdio;
pub mod tools;

pub use error::McpError;
pub use http::{create_mcp_router, mcp_token_from_env, MCP_PROJECT_HEADER, MCP_TOKEN_ENV_VAR};
pub use protocol::{dispatch_request, McpState, ToolDefinition, ToolHandler, ToolRegistry};
pub use tools::build_registry;
