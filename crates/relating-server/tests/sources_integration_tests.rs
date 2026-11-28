use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use relating_graph::RelateBackend;
use relating_metadata::init_in_memory_db;
use relating_server::{create_router, AppState, AuthState};
use serde_json::{json, Value};
use std::io::{Cursor, Write};
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

async fn create_project(app: &axum::Router) -> String {
    let (status, proj) = post_json(
        app,
        "/api/v1/projects".to_string(),
        json!({"name": "Sources"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    proj["id"].as_str().unwrap().to_string()
}

fn docx_bytes() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Roadmap</w:t></w:r></w:p>
    <w:p><w:r><w:t>Ship the graph first.</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

#[tokio::test]
async fn test_docx_upload_via_base64_end_to_end() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    let encoded = base64::engine::general_purpose::STANDARD.encode(docx_bytes());
    let (status, source) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({
            "filename": "roadmap.docx",
            "content": "",
            "content_base64": encoded,
            "content_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {}", source);
    assert_eq!(source["metadata"]["total_records"], 2);
    let fields = source["metadata"]["field_names"].as_array().unwrap();
    assert!(fields.contains(&json!("section")));
    let samples = source["metadata"]["sample_records"].as_array().unwrap();
    assert!(samples.iter().any(|r| r["text"] == "Roadmap"));
    assert!(samples.iter().any(|r| r["section"] == "Roadmap"));
}

#[tokio::test]
async fn test_text_upload_still_uses_plain_content() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    let (status, source) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({"filename": "notes.txt", "content": "hello\n\nworld"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(source["metadata"]["total_records"], 2);
}

#[tokio::test]
async fn test_binary_upload_rejects_garbage_clearly() {
    let (app, _dir) = setup_test_app().await;
    let pid = create_project(&app).await;

    // Invalid base64 is a client error, not a 500.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({"filename": "x.pdf", "content_base64": "!!! not base64 !!!"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"].as_str().unwrap().contains("base64"));

    // Bytes that are not a PDF are rejected with guidance, not parsed as text.
    let encoded = base64::engine::general_purpose::STANDARD.encode(b"definitely not a pdf");
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({"filename": "fake.pdf", "content_base64": encoded}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"].as_str().unwrap().contains("%PDF"));

    // Legacy .doc names a conversion path instead of producing garbage records.
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/sources", pid),
        json!({"filename": "legacy.doc", "content": "binary-ish"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"]["message"].as_str().unwrap().contains(".docx"));
}
