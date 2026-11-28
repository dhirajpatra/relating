//! SSO and RBAC: OIDC login handshake, session callbacks, identity lookup,
//! and grant management.
//!
//! Flow: `POST /auth/login {redirect_uri}` returns the provider authorization
//! URL plus a single-use state. The frontend completes the provider flow and
//! posts `{state, id_token}` to `/auth/callback`, which verifies the HS256
//! JWT, links the `(issuer, subject)` identity to a user, and returns an
//! opaque session token. Sessions and raw JWTs both work as
//! `Authorization: Bearer` credentials afterwards.

use crate::error::AppError;
use crate::routes::audit::audit_event;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use relating_core::{ActorContext, ProjectId, Role};
use serde::{Deserialize, Serialize};

/// Login sessions live half a day; JWT bearers follow their own `exp`.
const SESSION_TTL_HOURS: i64 = 12;
/// Single-use login states expire after ten minutes.
const LOGIN_STATE_TTL_MINUTES: i64 = 10;

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub redirect_uri: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub auth_url: String,
    pub state: String,
    pub expires_in_secs: i64,
}

pub async fn login_handler(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let Some(oidc) = state.auth.oidc.clone() else {
        return Err(AppError::BadRequest(
            "Single sign-on is not configured (set RELATING_OIDC_ISSUER and RELATING_OIDC_AUDIENCE)"
                .to_string(),
        ));
    };
    let redirect_uri = payload.redirect_uri.trim();
    if !(redirect_uri.starts_with("https://") || redirect_uri.starts_with("http://")) {
        return Err(AppError::BadRequest(
            "redirect_uri must be an absolute http(s) URL".to_string(),
        ));
    }
    let login_state = random_token();
    let nonce = random_token();
    let now = Utc::now();
    relating_metadata::create_pending_login(
        &state.pool,
        &login_state,
        &nonce,
        redirect_uri,
        &(now + Duration::minutes(LOGIN_STATE_TTL_MINUTES)),
    )
    .await?;
    audit_event(
        &state.pool,
        "unknown",
        "auth.login.started",
        "/api/v1/auth/login",
        "ok",
        "",
    )
    .await;
    Ok(Json(LoginResponse {
        auth_url: oidc.login_url(redirect_uri, &login_state, &nonce),
        state: login_state,
        expires_in_secs: LOGIN_STATE_TTL_MINUTES * 60,
    }))
}

#[derive(Deserialize)]
pub struct CallbackRequest {
    pub state: String,
    pub id_token: String,
}

pub async fn callback_handler(
    State(state): State<AppState>,
    Json(payload): Json<CallbackRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(oidc) = state.auth.oidc.clone() else {
        return Err(AppError::BadRequest(
            "Single sign-on is not configured (set RELATING_OIDC_ISSUER and RELATING_OIDC_AUDIENCE)"
                .to_string(),
        ));
    };
    let now = Utc::now();
    let pending = relating_metadata::take_pending_login(&state.pool, &payload.state, &now)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest("Unknown or expired login state; start over".to_string())
        })?;
    let _ = pending;
    let Some(secret) = oidc.client_secret.as_deref() else {
        return Err(AppError::Internal(
            "OIDC client secret is missing (set RELATING_OIDC_CLIENT_SECRET for HS256 id tokens)"
                .to_string(),
        ));
    };
    let claims = relating_core::verify_jwt_hs256(
        &payload.id_token,
        secret,
        &oidc.issuer,
        &oidc.audience,
        now.timestamp(),
    )
    .map_err(AppError::Unauthorized)?;
    let email = claims.email.clone().ok_or_else(|| {
        AppError::BadRequest("The id token must carry an email claim".to_string())
    })?;

    // Link (issuer, subject) to a user: existing identity wins, otherwise an
    // admin-provisioned same-email shell, otherwise a fresh user row.
    let user_id = if let Some(identity) =
        relating_metadata::find_identity(&state.pool, &claims.iss, &claims.sub).await?
    {
        identity.user_id
    } else {
        let user =
            relating_metadata::find_or_create_user(&state.pool, &email, claims.name.as_deref())
                .await?;
        relating_metadata::create_identity(&state.pool, &user.id, &claims.iss, &claims.sub).await?;
        user.id
    };
    let user = relating_metadata::find_user_by_id(&state.pool, &user_id)
        .await?
        .ok_or_else(|| AppError::Internal("User vanished mid-login".to_string()))?;

    let token = random_token();
    let expires_at = now + Duration::hours(SESSION_TTL_HOURS);
    relating_metadata::create_session(
        &state.pool,
        &user.id,
        &relating_core::sha256_hex(&token),
        &expires_at,
    )
    .await?;
    relating_metadata::delete_expired_sessions(&state.pool, &now).await;
    audit_event(
        &state.pool,
        &format!("user:{}", user.email),
        "auth.login.ok",
        "/api/v1/auth/callback",
        "ok",
        "",
    )
    .await;
    Ok(Json(serde_json::json!({
        "token": token,
        "token_type": "Bearer",
        "expires_at": expires_at.to_rfc3339(),
        "user": user,
    })))
}

pub async fn me_handler(
    Extension(ctx): Extension<ActorContext>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (kind, email) = match &ctx.actor {
        relating_core::Actor::Anonymous => ("anonymous", None),
        relating_core::Actor::ApiToken => ("api-token", None),
        relating_core::Actor::User { email, .. } => ("user", Some(email.clone())),
    };
    Ok(Json(serde_json::json!({
        "actor": ctx.actor.display(),
        "kind": kind,
        "email": email,
        "global_role": ctx.global_role.map(|r| r.as_str()),
        "project_role": ctx.project_role.map(|r| r.as_str()),
        "global_admin": ctx.global_admin,
    })))
}

fn parse_project(id_str: &str) -> Result<ProjectId, AppError> {
    ProjectId::parse(id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))
}

async fn require_project(state: &AppState, id: &ProjectId) -> Result<(), AppError> {
    relating_metadata::get_project(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", id)))?;
    Ok(())
}

pub async fn list_grants_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<relating_metadata::GrantView>>, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    let grants =
        relating_metadata::list_grants_for_project(&state.pool, &project_id.to_string()).await?;
    Ok(Json(grants))
}

pub async fn list_global_grants_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<relating_metadata::GrantView>>, AppError> {
    Ok(Json(
        relating_metadata::list_global_grants(&state.pool).await?,
    ))
}

#[derive(Deserialize)]
pub struct CreateGrantRequest {
    pub email: String,
    pub role: String,
}

async fn grant_user(
    state: &AppState,
    ctx: &ActorContext,
    project_id: Option<&str>,
    payload: CreateGrantRequest,
) -> Result<relating_metadata::Grant, AppError> {
    let email = payload.email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() < 3 {
        return Err(AppError::BadRequest(
            "Grant email must be a valid address".to_string(),
        ));
    }
    let role = Role::parse(&payload.role).ok_or_else(|| {
        AppError::BadRequest("Unknown role: use viewer, editor, or admin".to_string())
    })?;
    let user = relating_metadata::find_or_create_user(&state.pool, &email, None).await?;
    relating_metadata::create_grant(
        &state.pool,
        &user.id,
        project_id,
        role,
        &ctx.actor.display(),
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            AppError::BadRequest(format!("{} already holds {} access", email, role.as_str()))
        } else {
            AppError::Core(e)
        }
    })
}

pub async fn create_grant_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<ActorContext>,
    Path(id_str): Path<String>,
    Json(payload): Json<CreateGrantRequest>,
) -> Result<(StatusCode, Json<relating_metadata::Grant>), AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    let grant = grant_user(&state, &ctx, Some(&project_id.to_string()), payload).await?;
    audit_event(
        &state.pool,
        &ctx.actor.display(),
        "auth.grant.created",
        &format!("/api/v1/projects/{}/grants", project_id),
        "ok",
        &grant.id,
    )
    .await;
    Ok((StatusCode::CREATED, Json(grant)))
}

pub async fn create_global_grant_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<ActorContext>,
    Json(payload): Json<CreateGrantRequest>,
) -> Result<(StatusCode, Json<relating_metadata::Grant>), AppError> {
    let grant = grant_user(&state, &ctx, None, payload).await?;
    audit_event(
        &state.pool,
        &ctx.actor.display(),
        "auth.grant.created",
        "/api/v1/grants",
        "ok",
        &grant.id,
    )
    .await;
    Ok((StatusCode::CREATED, Json(grant)))
}

pub async fn delete_grant_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<ActorContext>,
    Path((id_str, grant_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let project_id = parse_project(&id_str)?;
    require_project(&state, &project_id).await?;
    if relating_metadata::delete_grant(&state.pool, &grant_id).await? {
        audit_event(
            &state.pool,
            &ctx.actor.display(),
            "auth.grant.revoked",
            &format!("/api/v1/projects/{}/grants/{}", project_id, grant_id),
            "ok",
            "",
        )
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!(
            "Grant '{}' not found",
            grant_id
        )))
    }
}
