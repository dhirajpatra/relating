pub mod ai;
pub mod cypher;
pub mod health;
pub mod projects;

use crate::state::AppState;
use axum::routing::{get, post};
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
        );

    let router = Router::new()
        .nest("/api/v1", api_routes);

    let static_dir = std::path::Path::new("web/dist");
    let router = if static_dir.exists() {
        let serve_dir = tower_http::services::ServeDir::new(static_dir)
            .not_found_service(tower_http::services::ServeFile::new(static_dir.join("index.html")));
        router.fallback_service(serve_dir)
    } else {
        router
    };

    router
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
