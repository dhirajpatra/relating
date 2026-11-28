use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_core::{base64url_encode, OidcConfig};
use relating_graph::RelateBackend;
use relating_metadata::{init_in_memory_db, DbPool};
use relating_server::{create_router, AppState, AuthState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

mod common;
use common::start_fake_bolt;

const TEST_PASSWORD_ENV: &str = "RELATING_TEST_AURA_PASSWORD";
const OIDC_ISSUER: &str = "https://sso.test";
const OIDC_AUDIENCE: &str = "relating-test";
const OIDC_SECRET: &str = "test-hs256-secret-123";

fn test_oidc() -> OidcConfig {
    OidcConfig {
        issuer: OIDC_ISSUER.to_string(),
        audience: OIDC_AUDIENCE.to_string(),
        client_secret: Some(OIDC_SECRET.to_string()),
        auth_url: format!("{}/authorize", OIDC_ISSUER),
    }
}

fn test_hmac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= block[i];
        opad[i] ^= block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_sum = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_sum);
    outer.finalize().to_vec()
}

/// Independently minted HS256 id token (separate HMAC code from the server).
fn mint_id_token(sub: &str, email: &str, exp_offset_secs: i64) -> String {
    let header = base64url_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let now = chrono::Utc::now().timestamp();
    let payload = base64url_encode(
        json!({
            "iss": OIDC_ISSUER,
            "sub": sub,
            "aud": OIDC_AUDIENCE,
            "exp": now + exp_offset_secs,
            "email": email,
            "name": email,
        })
        .to_string()
        .as_bytes(),
    );
    let mac = test_hmac(
        OIDC_SECRET.as_bytes(),
        format!("{}.{}", header, payload).as_bytes(),
    );
    format!("{}.{}.{}", header, payload, base64url_encode(&mac))
}

async fn setup_oidc_app(api_token: Option<String>) -> (axum::Router, tempfile::TempDir, DbPool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool: pool.clone(),
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token,
        auth: AuthState::with_oidc(test_oidc()),
    };
    (create_router(state), dir, pool)
}

async fn authed(
    app: &axum::Router,
    method: &str,
    uri: String,
    token: Option<&str>,
    payload: Option<Value>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }
    let req = match payload {
        Some(body) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json, headers)
}

/// Full SSO handshake for one user; returns the session token.
async fn login_user(app: &axum::Router, sub: &str, email: &str) -> String {
    let (status, login, _) = authed(
        app,
        "POST",
        "/api/v1/auth/login".to_string(),
        None,
        Some(json!({"redirect_uri": "https://app.test/callback"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let state = login["state"].as_str().unwrap().to_string();
    let (status, callback, _) = authed(
        app,
        "POST",
        "/api/v1/auth/callback".to_string(),
        None,
        Some(json!({"state": state, "id_token": mint_id_token(sub, email, 300)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "callback: {}", callback);
    callback["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_oidc_login_callback_and_me() {
    let (app, _dir, _pool) = setup_oidc_app(None).await;

    // Login advertises the provider URL with the round-tripped state.
    let (status, login, _) = authed(
        &app,
        "POST",
        "/api/v1/auth/login".to_string(),
        None,
        Some(json!({"redirect_uri": "https://app.test/cb"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(login["auth_url"]
        .as_str()
        .unwrap()
        .starts_with("https://sso.test/authorize?"));
    assert!(login["auth_url"]
        .as_str()
        .unwrap()
        .contains("client_id=relating-test"));
    let state = login["state"].as_str().unwrap().to_string();

    // Bad redirect targets and unknown states are rejected.
    let (status, _, _) = authed(
        &app,
        "POST",
        "/api/v1/auth/login".to_string(),
        None,
        Some(json!({"redirect_uri": "javascript:alert(1)"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = authed(
        &app,
        "POST",
        "/api/v1/auth/callback".to_string(),
        None,
        Some(json!({"state": "nope", "id_token": mint_id_token("u", "u@t", 300)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Expired tokens fail closed at the callback.
    let (status, expired, _) = authed(
        &app,
        "POST",
        "/api/v1/auth/callback".to_string(),
        None,
        Some(json!({"state": state.clone(), "id_token": mint_id_token("u-1", "ada@test", -600)})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{}", expired);

    // States are single-use: the consumed state above cannot replay.
    let (status, _, _) = authed(
        &app,
        "POST",
        "/api/v1/auth/callback".to_string(),
        None,
        Some(json!({"state": state, "id_token": mint_id_token("u-1", "ada@test", 300)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Fresh handshake succeeds and the session identifies the user.
    let token = login_user(&app, "u-1", "ada@test").await;
    let (status, me, _) = authed(
        &app,
        "GET",
        "/api/v1/auth/me".to_string(),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["actor"], "user:ada@test");
    assert_eq!(me["kind"], "user");
}

#[tokio::test]
async fn test_rbac_grants_and_read_write_split() {
    let (app, _dir, _pool) = setup_oidc_app(Some("break-glass".to_string())).await;
    let admin_token = "break-glass";

    // Bootstrap: break-glass token grants a global admin, who is also an SSO user.
    let boss_session = login_user(&app, "boss-1", "boss@test").await;
    let (status, _, _) = authed(
        &app,
        "POST",
        "/api/v1/grants".to_string(),
        Some(admin_token),
        Some(json!({"email": "boss@test", "role": "admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Duplicate grants are a client error, not a 500.
    let (status, _, _) = authed(
        &app,
        "POST",
        "/api/v1/grants".to_string(),
        Some(admin_token),
        Some(json!({"email": "boss@test", "role": "admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The global admin creates a project and grants roles on it.
    let (status, proj, _) = authed(
        &app,
        "POST",
        "/api/v1/projects".to_string(),
        Some(&boss_session),
        Some(json!({"name": "RBAC demo"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", proj);
    let pid = proj["id"].as_str().unwrap().to_string();

    let viewer_session = login_user(&app, "v-1", "viewer@test").await;
    let (status, _, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/grants", pid),
        Some(&boss_session),
        Some(json!({"email": "viewer@test", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Viewer reads, viewer does not write.
    let (status, _, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/sources", pid),
        Some(&viewer_session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, denied, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/sources", pid),
        Some(&viewer_session),
        Some(json!({"filename": "x.txt", "content": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", denied);

    // Strangers with no grant cannot even read.
    let stranger = login_user(&app, "s-1", "stranger@test").await;
    let (status, _, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/sources", pid),
        Some(&stranger),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Non-admins cannot manage grants.
    let (status, _, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/grants", pid),
        Some(&viewer_session),
        Some(json!({"email": "stranger@test", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Revoking the grant locks the viewer out.
    let (status, grants, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/grants", pid),
        Some(&boss_session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let grant_id = grants
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["email"] == "viewer@test")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/projects/{}/grants/{}", pid, grant_id))
                .header("Authorization", format!("Bearer {}", boss_session))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let (status, _, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/sources", pid),
        Some(&viewer_session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_second_actor_approve_rule() {
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (uri, server, _fake) = start_fake_bolt().await;
    let (app, _dir, _pool) = setup_oidc_app(Some("break-glass".to_string())).await;

    let planner = login_user(&app, "plan-1", "planner@test").await;
    let reviewer = login_user(&app, "rev-1", "reviewer@test").await;
    for (email, role) in [("planner@test", "editor"), ("reviewer@test", "editor")] {
        let (status, _, _) = authed(
            &app,
            "POST",
            "/api/v1/grants".to_string(),
            Some("break-glass"),
            Some(json!({"email": email, "role": role})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, proj, _) = authed(
        &app,
        "POST",
        "/api/v1/projects".to_string(),
        Some(&planner),
        Some(json!({"name": "Second actor"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap().to_string();
    let (status, _, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/cypher/write", pid),
        Some(&planner),
        Some(json!({"query": "CREATE (:Person {uid: 'Person:1'})"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, conn, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/connections/aura", pid),
        Some(&planner),
        Some(json!({
            "name": "fake", "uri": uri, "username": "neo4j",
            "auth_env_var": TEST_PASSWORD_ENV
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap().to_string();

    let (status, plan, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/migrations/plan", pid),
        Some(&planner),
        Some(json!({"connection_id": cid})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        plan["migration"]["progress"]["created_by"],
        "user:planner@test"
    );
    let mid = plan["migration"]["id"].as_str().unwrap().to_string();

    // The planner cannot approve their own migration.
    let (status, err, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        Some(&planner),
        Some(json!({"approved": true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{}", err);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("second actor"));

    // A different editor can, and the trail records both actors.
    let (status, approved, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/migrations/{}/approve", pid, mid),
        Some(&reviewer),
        Some(json!({"approved": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", approved);
    assert_eq!(approved["progress"]["approved_by"], "user:reviewer@test");

    server.abort();
}

#[tokio::test]
async fn test_rate_limit_locks_out_after_five_401s() {
    let (app, _dir, _pool) = setup_oidc_app(Some("correct-token".to_string())).await;

    for _ in 0..5 {
        let (status, _, _) = authed(
            &app,
            "GET",
            "/api/v1/projects".to_string(),
            Some("wrong"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, limited, headers) = authed(
        &app,
        "GET",
        "/api/v1/projects".to_string(),
        Some("wrong"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{}", limited);
    assert_eq!(limited["error"]["code"], "RATE_LIMITED");
    assert!(headers.contains_key("retry-after"));

    // Even the right token waits out the lockout, but health never locks.
    let (status, _, _) = authed(
        &app,
        "GET",
        "/api/v1/projects".to_string(),
        Some("correct-token"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, _, _) = authed(&app, "GET", "/api/v1/health".to_string(), None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_audit_export_json_csv_and_chain() {
    let (app, _dir, pool) = setup_oidc_app(Some("audit-token".to_string())).await;

    let (status, proj, _) = authed(
        &app,
        "POST",
        "/api/v1/projects".to_string(),
        Some("audit-token"),
        Some(json!({"name": "Audited"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap().to_string();
    let (status, _, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/sources", pid),
        Some("audit-token"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, export, _) = authed(
        &app,
        "GET",
        format!("/api/v1/projects/{}/audit/export", pid),
        Some("audit-token"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", export);
    assert_eq!(export["chain"]["valid"], true);
    assert!(export["event_count"].as_u64().unwrap() >= 1);
    assert!(export["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["resource"].as_str().unwrap().contains(&pid)));

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/projects/{}/audit/export?format=csv", pid))
                .header("Authorization", "Bearer audit-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("text/csv"));
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.starts_with("seq,actor,action,resource,outcome,detail,prev_hash,hash,created_at"));

    // The chain verifies over the live log and detects tampering.
    assert!(relating_metadata::verify_audit_chain(&pool).await.is_ok());
    sqlx::query("UPDATE audit_log SET action = 'tampered' WHERE seq = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(relating_metadata::verify_audit_chain(&pool).await.is_err());
}

async fn mcp_call(
    app: &axum::Router,
    pid: &str,
    token: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("x-relating-project", pid)
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn test_mcp_session_tokens_are_project_scoped() {
    std::env::set_var("RELATING_MCP_TOKEN", "mcp-static-token");
    std::env::set_var(TEST_PASSWORD_ENV, "test-password");
    let (app, _dir, _pool) = setup_oidc_app(Some("break-glass".to_string())).await;

    // Two projects; the user is granted on one only.
    let user = login_user(&app, "mcp-1", "mcp-user@test").await;
    let mut pids = Vec::new();
    for name in ["MCP scoped A", "MCP scoped B"] {
        let (status, proj, _) = authed(
            &app,
            "POST",
            "/api/v1/projects".to_string(),
            Some("break-glass"),
            Some(json!({"name": name})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        pids.push(proj["id"].as_str().unwrap().to_string());
    }
    let (status, _, _) = authed(
        &app,
        "POST",
        format!("/api/v1/projects/{}/grants", pids[0]),
        Some("break-glass"),
        Some(json!({"email": "mcp-user@test", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    // Session token: granted project answers, ungranted project refuses.
    let (status, _) = mcp_call(&app, &pids[0], &user, ping.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = mcp_call(&app, &pids[1], &user, ping.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Static MCP token stays global (break-glass for agents).
    let (status, _) = mcp_call(&app, &pids[1], "mcp-static-token", ping).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_provider_keys_are_write_only_and_env_vars_allowlisted() {
    // Fully open app: neither a static token nor OIDC, so requests pass as
    // anonymous and the redaction/allowlist behavior is isolated.
    let dir = tempfile::tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let open = create_router(AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: None,
        auth: AuthState::default(),
    });

    let (status, proj, _) = authed(
        &open,
        "POST",
        "/api/v1/projects".to_string(),
        None,
        Some(json!({
            "name": "Keys",
            "ai_config": {"provider_type": "openai", "api_key": "sk-live-secret"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(proj["ai_config"]["api_key"], Value::Null);
    let pid = proj["id"].as_str().unwrap().to_string();
    let (status, detail, _) = authed(
        &open,
        "GET",
        format!("/api/v1/projects/{}", pid),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["project"]["ai_config"]["api_key"], Value::Null);

    for bad in ["AWS_REGION", "lowercase_name", "PATH", "AURA PASSWORD", ""] {
        let (status, err, _) = authed(
            &open,
            "POST",
            format!("/api/v1/projects/{}/connections/aura", pid),
            None,
            Some(json!({"name": format!("c-{}", bad.len()), "uri": "bolt://localhost", "auth_env_var": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "var {}", bad);
        assert!(err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("auth_env_var"));
    }
    let (status, _, _) = authed(
        &open,
        "POST",
        format!("/api/v1/projects/{}/connections/aura", pid),
        None,
        Some(
            json!({"name": "ok", "uri": "bolt://localhost", "auth_env_var": "MY_BACKUP_PASSWORD"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}
