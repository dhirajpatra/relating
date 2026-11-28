use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_server::{create_router, AppState, AuthState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: None,
        auth: AuthState::default(),
    };
    (create_router(state), dir)
}

async fn post_json(app: &axum::Router, uri: String, payload: Value) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn get_json(app: &axum::Router, uri: String) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

fn person_schema() -> Value {
    json!({
        "entities": [{
            "label": "Person", "display_name": "Person",
            "properties": [
                {"name": "uid", "data_type": "string", "is_primary_key": true},
                {"name": "name", "data_type": "string"}
            ]
        }],
        "relationships": []
    })
}

async fn approve_person_ontology(app: &axum::Router, pid: &str) {
    let (status, onto) = post_json(
        app,
        format!("/api/v1/projects/{}/ontology/import", pid),
        json!({"schema": person_schema()}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let oid = onto["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        app,
        format!("/api/v1/projects/{}/ontology/approve", pid),
        json!({"ontology_id": oid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn create_project(app: &axum::Router, name: &str) -> String {
    let (status, proj) =
        post_json(app, "/api/v1/projects".to_string(), json!({"name": name})).await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

/// Seed a file-backed SQLite source database for connector tests.
async fn seed_sqlite_source(path: &std::path::Path) {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE people (uid TEXT, name TEXT, updated TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    for (uid, name, updated) in [
        ("p-1", "Ada", "2026-01-01T00:00:00Z"),
        ("p-2", "Bob", "2026-02-01T00:00:00Z"),
    ] {
        sqlx::query("INSERT INTO people (uid, name, updated) VALUES (?, ?, ?)")
            .bind(uid)
            .bind(name)
            .bind(updated)
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_sql_connector_crud_and_validation() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "SQL CRUD").await;
    let other = create_project(&app, "Other").await;

    for (kind, config, message_part) in [
        ("nosql", json!({"driver": "sqlite"}), "kind"),
        (
            "sql",
            json!({"driver": "oracle", "query": "SELECT 1"}),
            "driver",
        ),
        (
            "sql",
            json!({"driver": "sqlite", "path": "/tmp/x.db", "query": "DELETE FROM t"}),
            "read-only",
        ),
        (
            "sql",
            json!({"driver": "sqlite", "path": "/tmp/x.db", "query": "SELECT 1; SELECT 2"}),
            "single statement",
        ),
        (
            "sql",
            json!({"driver": "sqlite", "query": "SELECT 1"}),
            "path",
        ),
    ] {
        let (status, err) = post_json(
            &app,
            format!("/api/v1/projects/{}/connectors", pid),
            json!({"name": "bad", "kind": kind, "config": config}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", err);
        assert!(err["error"]["message"]
            .as_str()
            .unwrap()
            .contains(message_part));
    }

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "warehouse",
            "kind": "sql",
            "config": {"driver": "sqlite", "path": "/tmp/warehouse.db",
                        "query": "SELECT uid, name FROM people",
                        "watermark_column": "updated"},
            "mapping": {"uid_field": "uid"},
            "poll_interval_secs": 3600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", conn);
    assert_eq!(conn["kind"], "sql");
    let cid = conn["id"].as_str().unwrap().to_string();

    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "warehouse", "kind": "sql",
            "config": {"driver": "sqlite", "path": "/tmp/x.db", "query": "SELECT 1"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, list) = get_json(&app, format!("/api/v1/projects/{}/connectors", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Cross-project ids 404 (no scope leak).
    let (status, _) = get_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}", other, cid),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/projects/{}/connectors/{}", pid, cid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_rest_connector_rejects_raw_secret_headers() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "REST secrets").await;

    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "crm", "kind": "rest",
            "config": {"base_url": "https://crm.example/items",
                        "headers": {"X-Api-Key": "live-secret"}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{}", err);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("bearer_env"));

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "crm", "kind": "rest",
            "config": {"base_url": "https://crm.example/items",
                        "headers": {"X-Tenant": "acme"},
                        "bearer_env": "MY_CRM_TOKEN",
                        "record_path": "data.items",
                        "pagination": {"mode": "page", "page_size": 50}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", conn);
}

#[tokio::test]
async fn test_sql_sync_loads_graph_and_skips_unchanged() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "SQL sync").await;
    let source_path = _dir.path().join("warehouse.db");
    seed_sqlite_source(&source_path).await;

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "warehouse", "kind": "sql",
            "config": {"driver": "sqlite", "path": source_path.to_string_lossy(),
                        "query": "SELECT uid, name, updated FROM people",
                        "watermark_column": "updated"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conn["id"].as_str().unwrap().to_string();

    // Syncing without an approved ontology is refused, never half-loaded.
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}/sync", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    approve_person_ontology(&app, &pid).await;

    // The probe reads one row and registers nothing.
    let (status, probe) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}/test", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", probe);
    assert_eq!(probe["rows_seen"], 1);
    assert!(probe["columns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "uid"));
    let (status, sources) = get_json(&app, format!("/api/v1/projects/{}/sources", pid)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(sources.as_array().unwrap().is_empty());

    let (status, first) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}/sync", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", first);
    assert_eq!(first["rows_fetched"], 2);
    assert_eq!(first["records_mapped"], 2);
    assert_eq!(first["unchanged"], false);
    assert_eq!(first["nodes_created"], 2);
    assert_eq!(first["watermark"], "2026-02-01T00:00:00Z");

    // Unchanged snapshots skip the graph entirely.
    let (status, second) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}/sync", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["unchanged"], true);
    assert_eq!(second["nodes_created"], 0);

    // Managed source exists under its stable filename.
    let (status, sources) = get_json(&app, format!("/api/v1/projects/{}/sources", pid)).await;
    assert_eq!(status, StatusCode::OK);
    let filenames: Vec<&str> = sources
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["filename"].as_str())
        .collect();
    assert!(filenames.contains(&"connector://warehouse"));
}

#[tokio::test]
async fn test_rest_sync_paginates_and_retries() {
    use relating_connectors::test_stub::{start_json_stub, StubRoute};
    let (stub_base, _script, server) = start_json_stub(vec![(
        "/users".to_string(),
        vec![
            StubRoute::status(503, "Busy", "{}"),
            StubRoute::json(r#"[{"uid":"u-1","name":"Ada"}]"#),
            StubRoute::json(r#"[]"#),
        ],
    )])
    .await;

    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "REST sync").await;
    approve_person_ontology(&app, &pid).await;

    let (status, conn) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors", pid),
        json!({
            "name": "directory", "kind": "rest",
            "config": {"base_url": format!("{}/users", stub_base),
                        "pagination": {"mode": "offset", "page_size": 1},
                        "rate_limit_per_sec": 50.0}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", conn);
    let cid = conn["id"].as_str().unwrap().to_string();

    let (status, report) = post_json(
        &app,
        format!("/api/v1/projects/{}/connectors/{}/sync", pid, cid),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", report);
    assert_eq!(report["rows_fetched"], 1);
    assert_eq!(report["nodes_created"], 1);
    server.abort();
}

#[tokio::test]
async fn test_due_lists_poll_connectors() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Scheduler").await;

    for (name, interval) in [("hourly", Some(3600)), ("manual", None)] {
        let (status, _) = post_json(
            &app,
            format!("/api/v1/projects/{}/connectors", pid),
            json!({
                "name": name, "kind": "rest",
                "config": {"base_url": "https://api.example/x"},
                "poll_interval_secs": interval
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, due) = get_json(&app, format!("/api/v1/projects/{}/connectors/due", pid)).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = due
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(names, vec!["hourly"]);
}

#[tokio::test]
async fn test_ingest_skips_unchanged_and_full_reprocesses() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app, "Incremental").await;
    approve_person_ontology(&app, &pid).await;
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({"filename": "team.csv", "content": "uid,name\nu-1,Ada\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, first) =
        post_json(&app, format!("/api/v1/projects/{}/ingest", pid), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{}", first);
    assert_eq!(first["sources_processed"], 1);
    assert_eq!(first["sources_skipped"], 0);
    assert!(first["nodes_created"].as_i64().unwrap() >= 1);

    let (status, second) =
        post_json(&app, format!("/api/v1/projects/{}/ingest", pid), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["sources_processed"], 0);
    assert_eq!(second["sources_skipped"], 1);
    assert_eq!(second["nodes_created"], 0);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/projects/{}/ingest?full=true", pid))
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let forced: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(forced["sources_processed"], 1);
    assert_eq!(forced["sources_skipped"], 0);
}
