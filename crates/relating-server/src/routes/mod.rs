pub mod ai;
pub mod aura;
pub mod cypher;
pub mod embeddings;
pub mod evaluations;
pub mod graph;
pub mod health;
pub mod ingest;
pub mod migrations;
pub mod ontology;
pub mod portability;
pub mod projects;
pub mod questions;
pub mod retrieval;
pub mod schema;
pub mod sources;

use crate::state::AppState;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/health", get(health::health_handler))
        .route(
            "/projects",
            get(projects::list_projects_handler).post(projects::create_project_handler),
        )
        .route(
            "/projects/:id",
            get(projects::get_project_handler).delete(projects::delete_project_handler),
        )
        .route("/projects/:id/ai/test", post(ai::test_project_ai_handler))
        .route(
            "/projects/:id/cypher/read",
            post(cypher::cypher_read_handler),
        )
        .route(
            "/projects/:id/cypher/write",
            post(cypher::cypher_write_handler),
        )
        // Sources endpoints
        .route(
            "/projects/:id/sources",
            get(sources::list_sources_handler).post(sources::register_source_handler),
        )
        .route(
            "/projects/:id/sources/:source_id",
            delete(sources::delete_source_handler),
        )
        // Ontology endpoints
        .route(
            "/projects/:id/ontology/propose",
            post(ontology::propose_ontology_handler),
        )
        .route(
            "/projects/:id/ontology",
            get(ontology::get_ontology_handler),
        )
        .route(
            "/projects/:id/ontology/approve",
            post(ontology::approve_ontology_handler),
        )
        // Ingestion endpoint
        .route("/projects/:id/ingest", post(ingest::run_ingestion_handler))
        // Graph visualization elements endpoint
        .route(
            "/projects/:id/graph/elements",
            get(graph::get_graph_elements_handler),
        )
        // Slice 3: embeddings, retrieval, grounded Q&A, evaluation
        .route(
            "/projects/:id/embeddings",
            get(embeddings::get_embeddings_status_handler)
                .post(embeddings::generate_embeddings_handler),
        )
        .route(
            "/projects/:id/retrieval",
            post(retrieval::run_retrieval_handler),
        )
        .route(
            "/projects/:id/questions",
            post(questions::ask_question_handler),
        )
        .route(
            "/projects/:id/evaluations",
            get(evaluations::list_evaluations_handler).post(evaluations::create_evaluation_handler),
        )
        .route(
            "/projects/:id/evaluations/:eval_id",
            get(evaluations::get_evaluation_handler),
        )
        // R1: Aura connections and migration workflow
        .route(
            "/projects/:id/connections/aura",
            get(aura::list_connections_handler).post(aura::create_connection_handler),
        )
        .route(
            "/projects/:id/connections/aura/:conn_id",
            delete(aura::delete_connection_handler),
        )
        .route(
            "/projects/:id/connections/aura/:conn_id/test",
            post(aura::test_connection_handler),
        )
        .route(
            "/projects/:id/migrations/analyze",
            post(migrations::analyze_migration_handler),
        )
        .route(
            "/projects/:id/migrations/plan",
            post(migrations::plan_migration_handler),
        )
        .route(
            "/projects/:id/migrations",
            get(migrations::list_migrations_handler),
        )
        .route(
            "/projects/:id/migrations/:mid",
            get(migrations::get_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/dry-run",
            post(migrations::dry_run_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/approve",
            post(migrations::approve_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/execute",
            post(migrations::execute_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/verify",
            post(migrations::verify_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/report",
            get(migrations::get_migration_report_handler),
        )
        // R2: schema-as-code and retrieval configuration
        .route("/projects/:id/schema/export", get(schema::export_schema_handler))
        .route(
            "/projects/:id/schema/revisions",
            get(schema::list_revisions_handler).post(schema::create_revision_handler),
        )
        .route(
            "/projects/:id/schema/revisions/:version",
            get(schema::get_revision_handler),
        )
        .route("/projects/:id/schema/diff", post(schema::diff_schema_handler))
        .route("/projects/:id/schema/apply", post(schema::apply_schema_handler))
        .route(
            "/projects/:id/retrieval-config",
            get(schema::get_retrieval_config_handler).put(schema::update_retrieval_config_handler),
        )
        // R3: portability compiler
        .route(
            "/projects/:id/portability/scan",
            post(portability::scan_repo_handler),
        )
        .route(
            "/projects/:id/portability/run-tests",
            post(portability::run_tests_handler),
        );

    let mcp_state = relating_mcp::protocol::McpState {
        pool: state.pool.clone(),
        graph: state.graph.clone(),
        data_dir: state.data_dir.clone(),
    };

    let router = Router::new().nest("/api/v1", api_routes);

    let static_dir = std::path::Path::new("web/dist");
    let router = if static_dir.exists() {
        let serve_dir = tower_http::services::ServeDir::new(static_dir).not_found_service(
            tower_http::services::ServeFile::new(static_dir.join("index.html")),
        );
        router.fallback_service(serve_dir)
    } else {
        router
    };

    router
        .with_state(state)
        .merge(relating_mcp::create_mcp_router(mcp_state))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
