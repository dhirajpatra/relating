pub mod advisor;
pub mod ai;
pub mod audit;
pub mod aura;
pub mod auth;
pub mod cypher;
pub mod debug;
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
pub mod shadow;
pub mod sources;
pub mod synth;
pub mod targets;

use crate::state::{AppState, AUTH_LOCKOUT_SECS, MAX_AUTH_FAILURES};
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use relating_core::{authorize, constant_time_eq, is_jwt_shape, sha256_hex, verify_jwt_hs256};
use relating_core::{ActorContext, Role};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Paths that never need credentials: liveness plus the SSO handshake.
/// Everything else under `/api/v1/` is gated when the server is locked
/// (static token and/or OIDC configured).
fn is_public_api_path(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/health" | "/api/v1/auth/login" | "/api/v1/auth/callback"
    )
}

/// Project scope of a request path: the `{id}` in `/projects/{id}/...`.
/// List/create (`/projects` exactly) carry no scope.
fn project_scope_from_path(path: &str) -> Option<String> {
    let mut segments = path.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segment == "projects" {
            return segments
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }
    None
}

fn client_key(req: &Request<Body>) -> String {
    req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": { "code": "UNAUTHORIZED", "message": message }
        })),
    )
        .into_response()
}

fn forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": { "code": "FORBIDDEN", "message": message }
        })),
    )
        .into_response()
}

/// Bearer-token gate with OIDC sessions, RBAC, 401 lockout, and audit.
///
/// Locked mode (static `RELATING_API_TOKEN` and/or OIDC configured) accepts,
/// in order: the static token (break-glass admin, constant-time compared),
/// a login session token, or a verified OIDC JWT for a known identity.
/// Resolved actors get RBAC (`viewer` reads, `editor` writes, global admin
/// manages grants) and every gated request is audit-logged. Five consecutive
/// 401s lock the client key out for a minute. Open mode (neither configured)
/// passes everything as anonymous and stays loopback-only.
async fn require_auth(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if is_public_api_path(&path) || !path.starts_with("/api/v1/") {
        return next.run(req).await;
    }
    let locked = state.api_token.is_some() || state.auth.oidc.is_some();
    if !locked {
        req.extensions_mut().insert(ActorContext::anonymous());
        let response = next.run(req).await;
        return response;
    }

    let key = client_key(&req);
    let now = chrono::Utc::now();
    // Never hold the std mutex guard across .await (the future must stay Send).
    let lockout_retry: Option<String> = state
        .auth
        .attempts
        .lock()
        .ok()
        .and_then(|attempts| attempts.get(&key).and_then(|entry| entry.locked_until))
        .filter(|locked_until| *locked_until > now)
        .map(|locked_until| (locked_until - now).num_seconds().max(1).to_string());
    if let Some(retry) = lockout_retry {
        crate::routes::audit::audit_event(
            &state.pool,
            "unknown",
            "auth.locked",
            &path,
            "locked",
            "",
        )
        .await;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, retry)],
            Json(serde_json::json!({
                "error": {
                    "code": "RATE_LIMITED",
                    "message": "Too many failed authentication attempts; retry after the lockout"
                }
            })),
        )
            .into_response();
    }

    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .strip_prefix("Bearer ")
        .unwrap_or("")
        .to_string();

    let ctx = match resolve_actor(&state, &presented, &path).await {
        Ok(ctx) => ctx,
        Err(message) => {
            record_auth_failure(&state, &key);
            crate::routes::audit::audit_event(
                &state.pool,
                "unknown",
                "auth.denied",
                &path,
                "unauthorized",
                "",
            )
            .await;
            return unauthorized(&message);
        }
    };

    let method = req.method().to_string();
    if let Err(reason) = authorize(&ctx, &method, &path) {
        let actor = ctx.actor.display();
        crate::routes::audit::audit_event(
            &state.pool,
            &actor,
            "auth.forbidden",
            &path,
            "forbidden",
            "",
        )
        .await;
        return forbidden(&reason);
    }

    if let Ok(mut attempts) = state.auth.attempts.lock() {
        attempts.remove(&key);
    }
    let actor_label = ctx.actor.display();
    req.extensions_mut().insert(ctx);
    let response = next.run(req).await;
    let outcome = if response.status().is_success() {
        "ok"
    } else {
        "error"
    };
    let detail = response.status().as_u16().to_string();
    crate::routes::audit::audit_event(
        &state.pool,
        &actor_label,
        &format!("api.{}", method.to_lowercase()),
        &path,
        outcome,
        &detail,
    )
    .await;
    response
}

fn record_auth_failure(state: &AppState, key: &str) {
    let Ok(mut attempts) = state.auth.attempts.lock() else {
        return;
    };
    let entry = attempts.entry(key.to_string()).or_default();
    entry.fails += 1;
    if entry.fails >= MAX_AUTH_FAILURES {
        entry.locked_until =
            Some(chrono::Utc::now() + chrono::Duration::seconds(AUTH_LOCKOUT_SECS));
    }
}

/// Resolve a bearer token to an authorization context: static token, login
/// session, or verified OIDC JWT for a linked identity — in that order.
async fn resolve_actor(
    state: &AppState,
    presented: &str,
    path: &str,
) -> Result<ActorContext, String> {
    if presented.is_empty() {
        return Err(
            "This API requires an Authorization: Bearer <token> header (API token, session token, or OIDC JWT)"
                .to_string(),
        );
    }
    if let Some(expected) = state.api_token.as_deref() {
        if constant_time_eq(presented, expected) {
            return Ok(ActorContext::api_token());
        }
    }
    let Some(oidc) = state.auth.oidc.as_ref() else {
        return Err(
            "Invalid bearer token (set RELATING_API_TOKEN on the server and send the same value)"
                .to_string(),
        );
    };

    // Login sessions first: opaque tokens, hashed at rest, expirable.
    if let Ok(Some(session)) =
        relating_metadata::find_session_by_hash(&state.pool, &sha256_hex(presented)).await
    {
        if session.expires_at > chrono::Utc::now() {
            if let Ok(Some(user)) =
                relating_metadata::find_user_by_id(&state.pool, &session.user_id).await
            {
                return actor_context_for_user(state, &user.id, &user.email, path).await;
            }
        }
        return Err("Session token is expired or revoked; sign in again".to_string());
    }

    // Raw OIDC JWTs for identities linked through a prior callback.
    if !is_jwt_shape(presented) {
        return Err("Invalid bearer token".to_string());
    }
    let Some(secret) = oidc.client_secret.as_deref() else {
        return Err(
            "JWT bearer tokens need RELATING_OIDC_CLIENT_SECRET for HS256 verification".to_string(),
        );
    };
    let claims = verify_jwt_hs256(
        presented,
        secret,
        &oidc.issuer,
        &oidc.audience,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|e| format!("Invalid OIDC token: {}", e))?;
    let identity = relating_metadata::find_identity(&state.pool, &claims.iss, &claims.sub)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Unknown OIDC identity; complete POST /auth/callback first".to_string())?;
    let user = relating_metadata::find_user_by_id(&state.pool, &identity.user_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "User for this identity no longer exists".to_string())?;
    actor_context_for_user(state, &user.id, &user.email, path).await
}

async fn actor_context_for_user(
    state: &AppState,
    user_id: &str,
    email: &str,
    path: &str,
) -> Result<ActorContext, String> {
    let global_role = relating_metadata::user_global_role(&state.pool, user_id)
        .await
        .map_err(|e| e.to_string())?;
    let project_role = match project_scope_from_path(path) {
        Some(project_id) => relating_metadata::user_project_role(&state.pool, user_id, &project_id)
            .await
            .map_err(|e| e.to_string())?,
        None => None,
    };
    let global_admin = global_role == Some(Role::Admin);
    Ok(ActorContext {
        actor: relating_core::Actor::User {
            id: user_id.to_string(),
            email: email.to_string(),
        },
        global_role,
        project_role,
        global_admin,
    })
}

pub fn create_router(state: AppState) -> Router {
    // Tight method/header allowlists; origins stay open because the server
    // binds loopback and serves the workbench same-origin. Auth, not CORS,
    // is the access control here.
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);

    let api_routes = Router::new()
        .route("/health", get(health::health_handler))
        .route("/auth/login", post(auth::login_handler))
        .route("/auth/callback", post(auth::callback_handler))
        .route("/auth/me", get(auth::me_handler))
        .route(
            "/grants",
            get(auth::list_global_grants_handler).post(auth::create_global_grant_handler),
        )
        .route(
            "/projects/:id/grants",
            get(auth::list_grants_handler).post(auth::create_grant_handler),
        )
        .route(
            "/projects/:id/grants/:grant_id",
            delete(auth::delete_grant_handler),
        )
        .route(
            "/projects/:id/audit/export",
            get(audit::export_audit_handler),
        )
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
        .route(
            "/projects/:id/query-targets",
            get(targets::list_query_targets_handler),
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
            "/projects/:id/ontology/import",
            post(ontology::import_ontology_handler),
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
            "/projects/:id/migrations/:mid/rollback",
            post(migrations::rollback_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/report",
            get(migrations::get_migration_report_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/sync",
            post(migrations::sync_migration_handler),
        )
        .route(
            "/projects/:id/migrations/:mid/drift",
            post(migrations::drift_migration_handler),
        )
        .route("/projects/:id/changes", get(migrations::changes_handler))
        // R2: schema-as-code and retrieval configuration
        .route(
            "/projects/:id/schema/export",
            get(schema::export_schema_handler),
        )
        .route(
            "/projects/:id/schema/revisions",
            get(schema::list_revisions_handler).post(schema::create_revision_handler),
        )
        .route(
            "/projects/:id/schema/revisions/:version",
            get(schema::get_revision_handler),
        )
        .route(
            "/projects/:id/schema/diff",
            post(schema::diff_schema_handler),
        )
        .route(
            "/projects/:id/schema/apply",
            post(schema::apply_schema_handler),
        )
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
        )
        // R5: shadow testing and graph debugger
        .route(
            "/projects/:id/shadow/runs",
            post(shadow::run_shadow_handler),
        )
        .route(
            "/projects/:id/debug/trace",
            post(debug::debug_trace_handler),
        )
        // R6: synthetic data and performance advisor
        .route("/projects/:id/synth/profile", post(synth::profile_handler))
        .route(
            "/projects/:id/synth/generate",
            post(synth::generate_handler),
        )
        .route(
            "/projects/:id/advisor/analyze",
            post(advisor::analyze_handler),
        );

    let mcp_state = relating_mcp::protocol::McpState {
        pool: state.pool.clone(),
        graph: state.graph.clone(),
        data_dir: state.data_dir.clone(),
        mcp_token: relating_mcp::mcp_token_from_env(),
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

    // Auth runs inside CORS so preflights short-circuit before the gate.
    let auth = axum::middleware::from_fn_with_state(state.clone(), require_auth);
    router
        .with_state(state)
        .merge(relating_mcp::create_mcp_router(mcp_state))
        .layer(auth)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
