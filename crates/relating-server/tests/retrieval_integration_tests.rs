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
    let json: Value = serde_json::from_slice(&body).unwrap();
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
    let json: Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

async fn create_project(app: &axum::Router, name: &str) -> String {
    let (status, project) =
        post_json(app, "/api/v1/projects".to_string(), json!({ "name": name })).await;
    assert_eq!(status, StatusCode::CREATED);
    project["id"].as_str().unwrap().to_string()
}

const EMPLOYEES_CSV: &str =
    "name,role,department\nAlice,Engineer,Platform\nBob,Manager,Platform\nCarol,Designer,Product";

async fn register_csv_source(app: &axum::Router, project_id: &str) -> String {
    let (status, source) = post_json(
        app,
        format!("/api/v1/projects/{}/sources", project_id),
        json!({
            "filename": "employees.csv",
            "content": EMPLOYEES_CSV,
            "content_type": "text/csv"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    source["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_embeddings_retrieval_qa_evaluation_lifecycle() {
    let (app, _dir) = setup_test_app().await;
    let project_id = create_project(&app, "Slice 3 Workbench").await;
    let _source_id = register_csv_source(&app, &project_id).await;

    // 1. Generate embeddings: one chunk per CSV row
    let (status, gen) = post_json(
        &app,
        format!("/api/v1/projects/{}/embeddings", project_id),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(gen["sources_processed"], 1);
    assert_eq!(gen["chunks_created"], 3);
    assert_eq!(gen["chunks_skipped"], 0);
    assert_eq!(gen["dimensions"], 1536);
    assert_eq!(gen["profile"]["name"], "default");

    // 2. Regenerating is idempotent: everything is skipped
    let (status, regen) = post_json(
        &app,
        format!("/api/v1/projects/{}/embeddings", project_id),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(regen["chunks_created"], 0);
    assert_eq!(regen["chunks_skipped"], 3);

    // 3. Embedding status reports the profile and chunk count
    let (status, status_json) =
        get_json(&app, format!("/api/v1/projects/{}/embeddings", project_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(status_json["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(status_json["profiles"][0]["chunk_count"], 3);

    // 4. Retrieval works for every strategy and carries a trace
    for strategy in ["vector", "keyword", "hybrid"] {
        let (status, trace) = post_json(
            &app,
            format!("/api/v1/projects/{}/retrieval", project_id),
            json!({
                "query": "What is Alice's role?",
                "strategy": strategy,
                "top_k": 2
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "strategy {}", strategy);
        assert_eq!(trace["strategy"], strategy);
        assert_eq!(trace["dimensions"], 1536);
        assert_eq!(trace["passages"].as_array().unwrap().len(), 2);
        assert!(trace["passages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("name:"));
    }

    // Keyword retrieval ranks the Alice row first
    let (_, trace) = post_json(
        &app,
        format!("/api/v1/projects/{}/retrieval", project_id),
        json!({
            "query": "Alice Engineer",
            "strategy": "keyword",
            "top_k": 1
        }),
    )
    .await;
    assert!(trace["passages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Alice"));

    // 5. Grounded Q&A returns evidence-backed answers with a trace
    let (status, answer) = post_json(
        &app,
        format!("/api/v1/projects/{}/questions", project_id),
        json!({ "question": "Who works in Platform?" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["grounded"], true);
    assert_eq!(answer["prompt_version"], "rag-qa-v1");
    assert!(!answer["trace"]["passages"].as_array().unwrap().is_empty());

    // 6. Evaluation persists a scored report over multiple questions
    let (status, run) = post_json(
        &app,
        format!("/api/v1/projects/{}/evaluations", project_id),
        json!({ "questions": ["Who is Alice?", "Who is Bob?"] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(run["metrics"]["question_count"], 2);
    assert_eq!(run["metrics"]["grounded_count"], 2);
    assert_eq!(run["prompt_version"], "rag-qa-v1");
    assert_eq!(run["metrics_version"], "rag-eval-v1");
    assert_eq!(run["questions"].as_array().unwrap().len(), 2);
    let eval_id = run["id"].as_str().unwrap().to_string();

    let (status, fetched) = get_json(
        &app,
        format!("/api/v1/projects/{}/evaluations/{}", project_id, eval_id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"], eval_id);

    let (status, listed) =
        get_json(&app, format!("/api/v1/projects/{}/evaluations", project_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_questions_without_index_are_honestly_ungrounded() {
    let (app, _dir) = setup_test_app().await;
    let project_id = create_project(&app, "Empty Index").await;

    // No sources at all: no fabrication, explicit ungrounded flag
    let (status, answer) = post_json(
        &app,
        format!("/api/v1/projects/{}/questions", project_id),
        json!({ "question": "Anything?" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["grounded"], false);
    assert!(answer["citations"].as_array().unwrap().is_empty());
    assert!(answer["answer"]
        .as_str()
        .unwrap()
        .contains("could not find"));

    // Sources exist but embeddings were never generated: same honest answer
    let _source_id = register_csv_source(&app, &project_id).await;
    let (status, answer) = post_json(
        &app,
        format!("/api/v1/projects/{}/questions", project_id),
        json!({ "question": "Who is Alice?" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["grounded"], false);

    // Retrieval without an index is a client error with a clear message
    let (status, err) = post_json(
        &app,
        format!("/api/v1/projects/{}/retrieval", project_id),
        json!({ "query": "Alice" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["code"], "RETRIEVAL_ERROR");

    // Empty question is rejected
    let (status, _) = post_json(
        &app,
        format!("/api/v1/projects/{}/questions", project_id),
        json!({ "question": "   " }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_delete_source_removes_its_chunks() {
    let (app, _dir) = setup_test_app().await;
    let project_id = create_project(&app, "Chunk Cleanup").await;
    let source_id = register_csv_source(&app, &project_id).await;

    let (status, gen) = post_json(
        &app,
        format!("/api/v1/projects/{}/embeddings", project_id),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(gen["chunks_created"], 3);

    let delete_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/projects/{}/sources/{}",
                    project_id, source_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_res.status(), StatusCode::NO_CONTENT);

    let (status, status_json) =
        get_json(&app, format!("/api/v1/projects/{}/embeddings", project_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(status_json["profiles"][0]["chunk_count"], 0);
}

/// Live check against a real Ollama server. Not part of the default suite:
/// run explicitly with `cargo test -p relating-server --test
/// retrieval_integration_tests -- --ignored` while Ollama serves
/// OLLAMA_EMBEDDING_MODEL (default nomic-embed-text).
#[tokio::test]
#[ignore]
async fn test_ollama_live_embeddings_and_chat() {
    use relating_ai::openai_compatible::OpenAiCompatibleProvider;
    use relating_ai::{AiProvider, ChatMessage, ChatRequest, EmbeddingRequest};

    let base_url = std::env::var("OLLAMA_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
    let embedding_model =
        std::env::var("OLLAMA_EMBEDDING_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string());
    let chat_model = std::env::var("OLLAMA_CHAT_MODEL").unwrap_or_else(|_| "llama3.1".to_string());

    let provider = OpenAiCompatibleProvider::new(
        Some(base_url.clone()),
        None,
        chat_model.clone(),
        embedding_model.clone(),
    );

    let health = provider.health_check().await.unwrap_or_else(|e| {
        panic!(
            "Ollama is not reachable at {} ({}). Start Ollama with `ollama serve` and pull the models, or skip this live test.",
            base_url, e
        )
    });
    assert!(health.healthy);

    let embed_resp = provider
        .embed(&EmbeddingRequest {
            inputs: vec!["Relating connects graphs and language models.".to_string()],
        })
        .await
        .expect("Ollama embeddings call failed");
    assert_eq!(embed_resp.embeddings.len(), 1);
    assert!(embed_resp.dimensions > 0);

    let chat_resp = provider
        .chat_completion(&ChatRequest {
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Reply with the single word: ok".to_string(),
            }],
            temperature: Some(0.0),
            max_tokens: Some(10),
        })
        .await
        .expect("Ollama chat call failed");
    assert!(!chat_resp.content.trim().is_empty());
}
