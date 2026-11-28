use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_server::{create_router, AppState, AuthState};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let pool = init_in_memory_db().await.unwrap();
    let state = AppState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir: dir.path().to_path_buf(),
        api_token: None,
        auth: AuthState::default(),
    };
    let app = create_router(state);
    (app, dir)
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
    let json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, json)
}

fn good_schema() -> Value {
    json!({
        "entities": [
            {"label": "Employee", "display_name": "Employee",
             "properties": [{"name": "name", "data_type": "string", "is_primary_key": true}]},
            {"label": "Company", "display_name": "Company",
             "properties": [{"name": "name", "data_type": "string", "is_primary_key": true}]},
        ],
        "relationships": [
            {"rel_type": "WORKS_AT", "source_label": "Employee", "target_label": "Company"}
        ]
    })
}

#[tokio::test]
async fn test_ontology_import_validates_and_stores_proposal() {
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Import"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();

    // A good schema imports as a proposal and approves like an LLM one.
    let (status, onto) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/import", pid),
        json!({"schema": good_schema()}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(onto["version"], 1);
    let oid = onto["id"].as_str().unwrap();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/approve", pid),
        json!({"ontology_id": oid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Structural problems are refused, never stored.
    for bad in [
        json!({"entities": [], "relationships": []}),
        json!({"entities": [{"label": "", "display_name": "x"}], "relationships": []}),
        json!({"entities": [
            {"label": "A", "display_name": "A"},
            {"label": "A", "display_name": "A2"}
        ], "relationships": []}),
        json!({"entities": [{"label": "A", "display_name": "A"}],
               "relationships": [{"rel_type": "R", "source_label": "A", "target_label": "Ghost"}]}),
    ] {
        let (status, _) = post_json(
            &app,
            format!("/api/v1/projects/{}/ontology/import", pid),
            json!({"schema": bad}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "schema: {}", bad);
    }
}

#[tokio::test]
async fn test_aura_fill_fixture_seeds_graph_deterministically() {
    // The shipped fixture must always deserialize into the current schema
    // struct, or the deterministic seed silently rots.
    let workspace_fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/aura-fill-ontology.json");
    let raw = std::fs::read_to_string(&workspace_fixture)
        .expect("aura-fill fixture must exist next to the workspace tests dir");
    let mut fixture: serde_json::Value = serde_json::from_str(&raw).unwrap();
    fixture.as_object_mut().unwrap().remove("_comment");
    let schema: relating_core::OntologySchema = serde_json::from_value(fixture).unwrap();
    assert_eq!(schema.entities.len(), 4);
    assert_eq!(schema.relationships.len(), 3);

    // Small inline CSVs in the same shape as the script's seed: 2 employees
    // across 2 companies, 2 cities, 2 roles -> 8 nodes, 6 relationships.
    // No LLM call anywhere on this path: same bytes in, same graph out.
    let (app, _dir) = setup_test_app().await;
    let (status, proj) = post_json(
        &app,
        "/api/v1/projects".to_string(),
        json!({"name": "Seed"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap().to_string();

    let sources = [
        ("employees.csv", "name,role,company,city,company_id,role_id,city_id\nMara Ellison,Data Engineer,Northwind,Lisbon,Northwind,Data Engineer,Lisbon\nTomas Reyes,ML Engineer,Initech,Porto,Initech,ML Engineer,Porto"),
        ("company.csv", "name\nNorthwind\nInitech"),
        ("city.csv", "name\nLisbon\nPorto"),
        ("roles.csv", "title\nData Engineer\nML Engineer"),
    ];
    for (filename, content) in sources {
        let (status, _) = post_json(
            &app,
            format!("/api/v1/projects/{}/sources", pid),
            json!({"filename": filename, "content": content, "content_type": "text/csv"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "source {}", filename);
    }

    let (status, onto) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/import", pid),
        json!({"schema": schema}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let oid = onto["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/ontology/approve", pid),
        json!({"ontology_id": oid}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, ing) =
        post_json(&app, format!("/api/v1/projects/{}/ingest", pid), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ing["nodes_created"], 8);
    assert_eq!(ing["relationships_created"], 6);
}
