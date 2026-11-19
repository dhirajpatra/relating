use crate::error::McpError;
use crate::protocol::{McpState, ToolDefinition, ToolHandler, ToolRegistry};
use relating_ai::create_ai_provider;
use relating_core::{AuraConnectionId, Project, ProjectId, RetrievalStrategy};
use relating_graph::GraphBackend;
use relating_metadata::{
    count_chunks_for_profile, get_aura_connection, get_evaluation, get_latest_ontology,
    get_project, list_embedding_profiles, list_evaluations, list_sources,
};
use relating_migrate::{
    analyze_project, preview_migration, proxied_read, BoltBackend, BoltConfig, BoltUri,
    DEFAULT_BATCH_SIZE,
};
use relating_retrieval::run_retrieval;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

macro_rules! define_tool {
    ($name:ident, $tool_name:expr, $desc:expr, $schema:expr, $func:expr) => {
        pub struct $name;
        #[async_trait::async_trait]
        impl ToolHandler for $name {
            fn definition(&self) -> ToolDefinition {
                ToolDefinition {
                    name: $tool_name.to_string(),
                    description: $desc.to_string(),
                    input_schema: $schema,
                }
            }
            async fn call(
                &self,
                state: &McpState,
                project: &ProjectId,
                args: &Value,
            ) -> Result<String, McpError> {
                $func(state, project, args).await
            }
        }
    };
}

async fn require_project(state: &McpState, project: &ProjectId) -> Result<Project, McpError> {
    get_project(&state.pool, project)
        .await?
        .ok_or_else(|| McpError::Scope(format!("Project '{}' is not in scope", project)))
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_usize(args: &Value, key: &str, default: usize) -> usize {
    args.get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(default as u64) as usize
}

fn escape_label(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn to_pretty<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    serde_json::to_string_pretty(value)
        .map_err(|e| McpError::Internal(format!("Failed to serialize result: {}", e)))
}

async fn list_projects_fn(
    state: &McpState,
    project: &ProjectId,
    _args: &Value,
) -> Result<String, McpError> {
    // Connections are project-scoped: only the granted project is visible.
    let proj = require_project(state, project).await?;
    let graph_path = state.project_graph_path(project);
    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();
    to_pretty(&serde_json::json!([{
        "id": proj.id.to_string(),
        "name": proj.name,
        "description": proj.description,
        "node_count": schema.node_count,
        "relationship_count": schema.relationship_count,
    }]))
}

async fn get_project_summary_fn(
    state: &McpState,
    project: &ProjectId,
    _args: &Value,
) -> Result<String, McpError> {
    let proj = require_project(state, project).await?;
    let graph_path = state.project_graph_path(project);
    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();
    let sources = list_sources(&state.pool, project).await?;
    let ontology = get_latest_ontology(&state.pool, project).await?;
    let profiles = list_embedding_profiles(&state.pool, project).await?;
    let mut total_chunks = 0u64;
    for profile in &profiles {
        total_chunks += count_chunks_for_profile(&state.pool, &profile.id).await?;
    }
    to_pretty(&serde_json::json!({
        "id": proj.id.to_string(),
        "name": proj.name,
        "description": proj.description,
        "provider": proj.ai_config.provider_type,
        "chat_model": proj.ai_config.chat_model,
        "embedding_model": proj.ai_config.embedding_model,
        "node_count": schema.node_count,
        "relationship_count": schema.relationship_count,
        "labels": schema.labels,
        "relationship_types": schema.relationship_types,
        "source_count": sources.len(),
        "latest_ontology": ontology,
        "embedding_profiles": profiles.len(),
        "chunk_count": total_chunks,
    }))
}

async fn get_graph_schema_fn(
    state: &McpState,
    project: &ProjectId,
    _args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let graph_path = state.project_graph_path(project);
    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();
    to_pretty(&schema)
}

async fn search_sources_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let query = arg_str(args, "query").unwrap_or_default().to_lowercase();
    let limit = arg_usize(args, "limit", 20).clamp(1, 100);
    let sources = list_sources(&state.pool, project).await?;
    let matches: Vec<_> = sources
        .into_iter()
        .filter(|s| {
            query.is_empty()
                || s.filename.to_lowercase().contains(&query)
                || s.content_type.to_lowercase().contains(&query)
        })
        .take(limit)
        .collect();
    to_pretty(&matches)
}

async fn search_graph_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let label = arg_str(args, "label");
    let term = arg_str(args, "term").unwrap_or_default().to_lowercase();
    let limit = arg_usize(args, "limit", 25).clamp(1, 100) as u32;

    let query = match &label {
        Some(l) => format!("MATCH (n:{}) RETURN n LIMIT {}", escape_label(l), limit * 4),
        None => format!("MATCH (n) RETURN n LIMIT {}", limit * 4),
    };
    let graph_path = state.project_graph_path(project);
    let result = state
        .graph
        .execute_read(&graph_path, &query, HashMap::new(), None)?;

    let mut nodes: Vec<Value> = Vec::new();
    for row in result.rows {
        if let Some(node) = row.get("n") {
            if term.is_empty() || node.to_string().to_lowercase().contains(&term) {
                nodes.push(node.clone());
            }
            if nodes.len() >= limit as usize {
                break;
            }
        }
    }
    to_pretty(&nodes)
}

async fn read_cypher_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let query = arg_str(args, "query")
        .filter(|q| !q.trim().is_empty())
        .ok_or_else(|| McpError::InvalidArguments("Missing required 'query'".to_string()))?;
    let mut params = HashMap::new();
    if let Some(Value::Object(map)) = args.get("params") {
        for (k, v) in map {
            params.insert(k.clone(), v.clone());
        }
    }
    let limit = Some(arg_usize(args, "limit", 100).clamp(1, 1000) as u32);
    // Optional backend flip: the same query text runs on Aura instead of the
    // local store, returned in the identical shape. Absent by default.
    if let Some(connection_id) = arg_str(args, "connection_id") {
        let conn_id = AuraConnectionId::parse(&connection_id).map_err(|e| {
            McpError::InvalidArguments(format!("Invalid connection id '{}': {}", connection_id, e))
        })?;
        let conn = get_aura_connection(&state.pool, &conn_id)
            .await?
            .ok_or_else(|| McpError::Scope(format!("Connection '{}' is not in scope", conn_id)))?;
        if conn.project_id != *project {
            return Err(McpError::Scope(format!(
                "Connection '{}' is not in scope",
                conn_id
            )));
        }
        let uri = BoltUri::parse(&conn.uri)?;
        let password = relating_core::resolve_secret(&conn.auth_env_var)
            .map_err(McpError::InvalidArguments)?;
        let mut config = BoltConfig::new(uri, conn.username.clone(), password);
        config.database = conn.database.clone();
        let backend = BoltBackend::new(config);
        let result = proxied_read(&backend, &query, params, limit).await?;
        return to_pretty(&result);
    }
    let graph_path = state.project_graph_path(project);
    // Execute_read opens the store read-only: writes are rejected by the engine.
    let result = state
        .graph
        .execute_read(&graph_path, &query, params, limit)?;
    to_pretty(&result)
}

async fn retrieve_context_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    let proj = require_project(state, project).await?;
    let query = arg_str(args, "query")
        .filter(|q| !q.trim().is_empty())
        .ok_or_else(|| McpError::InvalidArguments("Missing required 'query'".to_string()))?;
    let strategy = match arg_str(args, "strategy").as_deref() {
        Some("vector") => RetrievalStrategy::Vector,
        Some("keyword") => RetrievalStrategy::Keyword,
        _ => RetrievalStrategy::Hybrid,
    };
    let top_k = arg_usize(args, "top_k", 5);
    let provider = create_ai_provider(&proj.ai_config)?;
    let trace = run_retrieval(&state.pool, &provider, &proj, &query, strategy, top_k, None).await?;
    to_pretty(&trace)
}

async fn explain_retrieval_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    let proj = require_project(state, project).await?;
    let query = arg_str(args, "query")
        .filter(|q| !q.trim().is_empty())
        .ok_or_else(|| McpError::InvalidArguments("Missing required 'query'".to_string()))?;
    let strategy = match arg_str(args, "strategy").as_deref() {
        Some("vector") => RetrievalStrategy::Vector,
        Some("keyword") => RetrievalStrategy::Keyword,
        _ => RetrievalStrategy::Hybrid,
    };
    let top_k = arg_usize(args, "top_k", 5);
    let provider = create_ai_provider(&proj.ai_config)?;
    let trace = run_retrieval(&state.pool, &provider, &proj, &query, strategy, top_k, None).await?;

    let mut out = format!(
        "Retrieval for query '{}' used {} strategy over embedding profile '{}' (model {}, {} dims) and returned {} passage(s) in {:.1}ms.\n",
        trace.query,
        match trace.strategy {
            RetrievalStrategy::Vector => "vector",
            RetrievalStrategy::Keyword => "keyword",
            RetrievalStrategy::Hybrid => "hybrid (vector + keyword with reciprocal rank fusion)",
        },
        trace.profile_name,
        trace.embedding_model,
        trace.dimensions,
        trace.passages.len(),
        trace.latency_ms
    );
    for passage in &trace.passages {
        out.push_str(&format!(
            "\n[{}] {} (score {:.3}): {}",
            passage.rank,
            passage.filename,
            passage.score,
            passage.content.chars().take(200).collect::<String>()
        ));
    }
    Ok(out)
}

async fn get_ingestion_status_fn(
    state: &McpState,
    project: &ProjectId,
    _args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let sources = list_sources(&state.pool, project).await?;
    let ontology = get_latest_ontology(&state.pool, project).await?;
    let graph_path = state.project_graph_path(project);
    let schema = state.graph.get_schema(&graph_path).unwrap_or_default();
    to_pretty(&serde_json::json!({
        "sources": sources.iter().map(|s| serde_json::json!({
            "id": s.id.to_string(),
            "filename": s.filename,
            "status": s.status,
            "byte_size": s.byte_size,
        })).collect::<Vec<_>>(),
        "latest_ontology": ontology,
        "graph_node_count": schema.node_count,
        "graph_relationship_count": schema.relationship_count,
    }))
}

async fn get_evaluation_report_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    if let Some(eval_id_str) = arg_str(args, "evaluation_id") {
        let eval_id = relating_core::EvaluationId::parse(&eval_id_str)
            .map_err(|e| McpError::InvalidArguments(format!("Invalid evaluation id: {}", e)))?;
        let run = get_evaluation(&state.pool, &eval_id)
            .await?
            .ok_or_else(|| McpError::InvalidArguments("Evaluation not found".to_string()))?;
        return to_pretty(&run);
    }
    let runs = list_evaluations(&state.pool, project).await?;
    match runs.first() {
        Some(run) => to_pretty(run),
        None => Ok("No evaluation runs exist for this project yet.".to_string()),
    }
}

async fn analyze_aura_compatibility_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let connection = match arg_str(args, "connection_id") {
        Some(conn_id_str) => {
            let conn_id = relating_core::AuraConnectionId::parse(&conn_id_str)
                .map_err(|e| McpError::InvalidArguments(format!("Invalid connection id: {}", e)))?;
            Some(
                get_aura_connection(&state.pool, &conn_id)
                    .await?
                    .ok_or_else(|| {
                        McpError::InvalidArguments("Aura connection not found".to_string())
                    })?,
            )
        }
        None => None,
    };
    let graph_path = state.project_graph_path(project);
    let report = analyze_project(
        &state.pool,
        state.graph.as_ref(),
        project,
        &graph_path,
        connection.as_ref(),
    )
    .await?;
    to_pretty(&report)
}

async fn preview_migration_fn(
    state: &McpState,
    project: &ProjectId,
    args: &Value,
) -> Result<String, McpError> {
    require_project(state, project).await?;
    let conn_id_str = arg_str(args, "connection_id").ok_or_else(|| {
        McpError::InvalidArguments("Missing required 'connection_id'".to_string())
    })?;
    let conn_id = relating_core::AuraConnectionId::parse(&conn_id_str)
        .map_err(|e| McpError::InvalidArguments(format!("Invalid connection id: {}", e)))?;
    let conn = get_aura_connection(&state.pool, &conn_id)
        .await?
        .ok_or_else(|| McpError::InvalidArguments("Aura connection not found".to_string()))?;
    if conn.project_id != *project {
        return Err(McpError::Scope(
            "Connection is not in this project".to_string(),
        ));
    }
    let batch_size = arg_usize(args, "batch_size", DEFAULT_BATCH_SIZE).clamp(1, 5000);
    let graph_path = state.project_graph_path(project);
    // Stateless preview: plans and dry-runs without persisting or writing.
    let (_plan, dry_run) = preview_migration(
        &state.pool,
        state.graph.as_ref(),
        project,
        &graph_path,
        &conn,
        batch_size,
    )
    .await?;
    to_pretty(&dry_run)
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

pub fn build_registry() -> ToolRegistry {
    define_tool!(
        ListProjects,
        "list_projects",
        "List projects visible to this connection (project-scoped: the granted project).",
        object_schema(serde_json::json!({}), &[]),
        list_projects_fn
    );
    define_tool!(
        GetProjectSummary,
        "get_project_summary",
        "Summarize the granted project: models, graph counts, sources, ontology, embeddings.",
        object_schema(serde_json::json!({}), &[]),
        get_project_summary_fn
    );
    define_tool!(
        GetGraphSchema,
        "get_graph_schema",
        "Return the project graph schema: labels, relationship types, counts.",
        object_schema(serde_json::json!({}), &[]),
        get_graph_schema_fn
    );
    define_tool!(
        SearchSources,
        "search_sources",
        "Search project sources by filename or content type.",
        object_schema(
            serde_json::json!({
                "query": {"type": "string", "description": "Substring to match"},
                "limit": {"type": "integer", "default": 20},
            }),
            &[]
        ),
        search_sources_fn
    );
    define_tool!(
        SearchGraph,
        "search_graph",
        "Search graph nodes, optionally filtered by label and text term.",
        object_schema(
            serde_json::json!({
                "label": {"type": "string", "description": "Node label filter"},
                "term": {"type": "string", "description": "Case-insensitive substring filter"},
                "limit": {"type": "integer", "default": 25},
            }),
            &[]
        ),
        search_graph_fn
    );
    define_tool!(
        ReadCypher,
        "read_cypher",
        "Run a read-only Cypher query with bound parameters. Writes are rejected. Pass connection_id to run the identical statement on Aura instead of the local store.",
        object_schema(
            serde_json::json!({
                "query": {"type": "string", "description": "Read-only Cypher statement"},
                "params": {"type": "object", "description": "Bound parameters"},
                "limit": {"type": "integer", "default": 100},
                "connection_id": {"type": "string", "description": "Optional Aura connection id; when set, the same statement is proxied to Aura and returned in the identical shape"},
            }),
            &["query"]
        ),
        read_cypher_fn
    );
    define_tool!(
        RetrieveContext,
        "retrieve_context",
        "Retrieve evidence passages for a question using vector, keyword, or hybrid search.",
        object_schema(
            serde_json::json!({
                "query": {"type": "string"},
                "strategy": {"type": "string", "enum": ["vector", "keyword", "hybrid"], "default": "hybrid"},
                "top_k": {"type": "integer", "default": 5},
            }),
            &["query"]
        ),
        retrieve_context_fn
    );
    define_tool!(
        ExplainRetrieval,
        "explain_retrieval",
        "Explain in plain language how retrieval answered a query: strategy, profile, scores.",
        object_schema(
            serde_json::json!({
                "query": {"type": "string"},
                "strategy": {"type": "string", "enum": ["vector", "keyword", "hybrid"], "default": "hybrid"},
                "top_k": {"type": "integer", "default": 5},
            }),
            &["query"]
        ),
        explain_retrieval_fn
    );
    define_tool!(
        GetIngestionStatus,
        "get_ingestion_status",
        "Report source ingestion states, ontology status, and graph counts.",
        object_schema(serde_json::json!({}), &[]),
        get_ingestion_status_fn
    );
    define_tool!(
        GetEvaluationReport,
        "get_evaluation_report",
        "Fetch the latest (or a specific) retrieval evaluation report.",
        object_schema(
            serde_json::json!({
                "evaluation_id": {"type": "string", "description": "Defaults to latest run"},
            }),
            &[]
        ),
        get_evaluation_report_fn
    );
    define_tool!(
        AnalyzeAuraCompatibility,
        "analyze_aura_compatibility",
        "Analyse source schema and Aura target compatibility (read-only).",
        object_schema(
            serde_json::json!({
                "connection_id": {"type": "string", "description": "Aura connection to probe"},
            }),
            &[]
        ),
        analyze_aura_compatibility_fn
    );
    define_tool!(
        PreviewMigration,
        "preview_migration",
        "Preview a migration plan and dry-run report without persisting or writing anything.",
        object_schema(
            serde_json::json!({
                "connection_id": {"type": "string"},
                "batch_size": {"type": "integer", "default": 500},
            }),
            &["connection_id"]
        ),
        preview_migration_fn
    );

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ListProjects));
    registry.register(Arc::new(GetProjectSummary));
    registry.register(Arc::new(GetGraphSchema));
    registry.register(Arc::new(SearchSources));
    registry.register(Arc::new(SearchGraph));
    registry.register(Arc::new(ReadCypher));
    registry.register(Arc::new(RetrieveContext));
    registry.register(Arc::new(ExplainRetrieval));
    registry.register(Arc::new(GetIngestionStatus));
    registry.register(Arc::new(GetEvaluationReport));
    registry.register(Arc::new(AnalyzeAuraCompatibility));
    registry.register(Arc::new(PreviewMigration));
    registry
}
