use relating_core::{AiProviderConfig, Project, ProjectId};
use relating_graph::{GraphBackend, RelateBackend};
use relating_metadata::{init_db, insert_project};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn relating_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_relating"))
}

struct Fixture {
    _dir: tempfile::TempDir,
    data_dir: String,
    db_url: String,
    project_id: String,
}

async fn setup_clean_project() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db_url = format!("sqlite://{}/relating_metadata.db", data_dir.display());
    let pool = init_db(&db_url).await.unwrap();
    let project = Project {
        id: ProjectId::new(),
        name: "CLI Test".to_string(),
        description: String::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        ai_config: AiProviderConfig::mock(),
    };
    insert_project(&pool, &project).await.unwrap();

    let backend = RelateBackend::new();
    let graph_path = data_dir
        .join("projects")
        .join(project.id.to_string())
        .join("graph.rl");
    backend.ensure_store(&graph_path).unwrap();
    backend
        .execute_write(
            &graph_path,
            "CREATE (a:Person {uid: 'Person:1', name: 'Ada'})-[:KNOWS]->(b:Person {uid: 'Person:2', name: 'Bob'})",
            HashMap::new(),
        )
        .unwrap();

    Fixture {
        _dir: dir,
        data_dir: data_dir.to_string_lossy().to_string(),
        db_url,
        project_id: project.id.to_string(),
    }
}

fn run_cli(fixture: &Fixture, args: &[&str]) -> (i32, String) {
    let output = Command::new(relating_bin())
        .args([
            "--data-dir",
            &fixture.data_dir,
            "--database-url",
            &fixture.db_url,
        ])
        .args(args)
        .output()
        .expect("failed to run relating binary");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (code, stdout)
}

#[tokio::test]
async fn test_validate_passes_on_clean_project() {
    let fixture = setup_clean_project().await;
    let (code, stdout) = run_cli(&fixture, &["validate", "--project", &fixture.project_id]);
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(stdout.contains("Result: PASS"), "stdout: {}", stdout);
    assert!(stdout.contains("data-integrity"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_validate_fails_on_missing_uid() {
    let fixture = setup_clean_project().await;
    // Add a node without a uid merge key.
    let backend = RelateBackend::new();
    let graph_path = PathBuf::from(&fixture.data_dir)
        .join("projects")
        .join(&fixture.project_id)
        .join("graph.rl");
    backend
        .execute_write(
            &graph_path,
            "CREATE (n:Person {name: 'NoUid'})",
            HashMap::new(),
        )
        .unwrap();

    let (code, stdout) = run_cli(&fixture, &["validate", "--project", &fixture.project_id]);
    assert_eq!(code, 1, "stdout: {}", stdout);
    assert!(stdout.contains("uid"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_graph_contracts_against_revision() {
    let fixture = setup_clean_project().await;
    // test graph with no revisions: passes with a warning, never blocks fresh work.
    let (code, stdout) = run_cli(
        &fixture,
        &["test", "graph", "--project", &fixture.project_id],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
}

#[tokio::test]
async fn test_schema_export_and_diff() {
    let fixture = setup_clean_project().await;
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("relating");
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "schema",
            "export",
            "--project",
            &fixture.project_id,
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(out.join("graph/schema.json").exists());
    assert!(out.join("graph/ontology.json").exists());
    assert!(out.join("graph/constraints.cypher").exists());
    assert!(out.join("graph/indexes.cypher").exists());
    assert!(out.join("tests/graph_contracts.json").exists());

    // Diffing the just-exported file against live state is a no-op.
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "schema",
            "diff",
            "--project",
            &fixture.project_id,
            "--file",
            out.join("graph/schema.json").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);

    // A breaking edit exits 1.
    let breaking_path = out.join("breaking.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("graph/schema.json")).unwrap())
            .unwrap();
    doc["constraints"] = serde_json::json!([]);
    doc["ontology"]["entities"] = serde_json::json!([{
        "label": "Ghost", "display_name": "Ghost", "description": "",
        "properties": [{"name": "uid", "data_type": "string", "is_primary_key": true}]
    }]);
    std::fs::write(&breaking_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "schema",
            "diff",
            "--project",
            &fixture.project_id,
            "--file",
            breaking_path.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 1, "stdout: {}", stdout);
    assert!(stdout.contains("BREAKING"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_aura_analyze_unreachable_fails() {
    std::env::set_var("RELATING_TEST_CLI_AURA_PASSWORD", "test-password");
    let fixture = setup_clean_project().await;
    let pool = relating_metadata::init_db(&fixture.db_url).await.unwrap();
    let conn = relating_core::AuraConnection {
        id: relating_core::AuraConnectionId::new(),
        project_id: ProjectId::parse(&fixture.project_id).unwrap(),
        name: "down".to_string(),
        uri: "bolt://127.0.0.1:9".to_string(),
        username: "neo4j".to_string(),
        database: None,
        auth_env_var: "RELATING_TEST_CLI_AURA_PASSWORD".to_string(),
        created_at: chrono::Utc::now(),
    };
    relating_metadata::insert_aura_connection(&pool, &conn)
        .await
        .unwrap();

    let (code, stdout) = run_cli(
        &fixture,
        &[
            "aura",
            "analyze",
            "--project",
            &fixture.project_id,
            "--connection",
            &conn.id.to_string(),
        ],
    );
    assert_eq!(code, 1, "stdout: {}", stdout);
    assert!(stdout.contains("unreachable"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_eval_run_with_mock_provider() {
    let fixture = setup_clean_project().await;
    // Seed one embedded chunk directly with the deterministic mock provider.
    let pool = relating_metadata::init_db(&fixture.db_url).await.unwrap();
    let project_id = ProjectId::parse(&fixture.project_id).unwrap();
    let profile = relating_core::EmbeddingProfile {
        id: relating_core::EmbeddingProfileId::new(),
        project_id: project_id.clone(),
        name: "default".to_string(),
        provider: "mock".to_string(),
        model: "mock-embed-v1".to_string(),
        model_version: None,
        dimensions: 8,
        modality: "mixed".to_string(),
        created_at: chrono::Utc::now(),
    };
    relating_metadata::insert_embedding_profile(&pool, &profile)
        .await
        .unwrap();
    let source = relating_core::Source {
        id: relating_core::SourceId::new(),
        project_id: project_id.clone(),
        filename: "notes.txt".to_string(),
        content_hash: "hash".to_string(),
        content_type: "text/plain".to_string(),
        byte_size: 10,
        status: relating_core::SourceStatus::Pending,
        metadata: serde_json::json!({}),
        created_at: chrono::Utc::now(),
    };
    relating_metadata::insert_source(&pool, &source)
        .await
        .unwrap();
    let embedding = vec![0.1f32; 8];
    let chunk = relating_core::Chunk {
        id: relating_core::ChunkId::new(),
        project_id,
        profile_id: profile.id,
        source_id: source.id,
        ordinal: 0,
        content: "Ada is an engineer on the platform team.".to_string(),
        content_hash: "test-hash".to_string(),
        metadata: serde_json::json!({"kind": "prose"}),
        embedding,
        created_at: chrono::Utc::now(),
    };
    relating_metadata::insert_chunk(&pool, &chunk)
        .await
        .unwrap();

    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(questions.path(), "Who is Ada?\n").unwrap();
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "eval",
            "run",
            "--project",
            &fixture.project_id,
            "--questions",
            questions.path().to_str().unwrap(),
            "--strategy",
            "keyword",
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(stdout.contains("grounded 1/1"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_shadow_run_reports_unreachable_target() {
    std::env::set_var("RELATING_TEST_CLI_SHADOW_PASSWORD", "test-password");
    let fixture = setup_clean_project().await;
    let pool = relating_metadata::init_db(&fixture.db_url).await.unwrap();
    let conn = relating_core::AuraConnection {
        id: relating_core::AuraConnectionId::new(),
        project_id: ProjectId::parse(&fixture.project_id).unwrap(),
        name: "down".to_string(),
        uri: "bolt://127.0.0.1:9".to_string(),
        username: "neo4j".to_string(),
        database: None,
        auth_env_var: "RELATING_TEST_CLI_SHADOW_PASSWORD".to_string(),
        created_at: chrono::Utc::now(),
    };
    relating_metadata::insert_aura_connection(&pool, &conn)
        .await
        .unwrap();

    let queries = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        queries.path(),
        r#"[{"name": "all", "cypher": "MATCH (n) RETURN n"}]"#,
    )
    .unwrap();
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "shadow",
            "run",
            "--project",
            &fixture.project_id,
            "--connection",
            &conn.id.to_string(),
            "--queries",
            queries.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, 1, "stdout: {}", stdout);
    assert!(stdout.contains("Confidence:"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_debug_trace_without_evidence() {
    let fixture = setup_clean_project().await;
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "debug",
            "trace",
            "--project",
            &fixture.project_id,
            "--question",
            "Who?",
            "--no-ai",
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(stdout.contains("DBG-E001"), "stdout: {}", stdout);
    assert!(stdout.contains("Likely causes:"), "stdout: {}", stdout);
}

#[tokio::test]
async fn test_synth_generate_writes_dataset() {
    let fixture = setup_clean_project().await;
    let out = tempfile::NamedTempFile::new().unwrap();
    let (code, stdout) = run_cli(
        &fixture,
        &[
            "synth",
            "generate",
            "--project",
            &fixture.project_id,
            "--out",
            out.path().to_str().unwrap(),
            "--scale",
            "0.5",
            "--seed",
            "3",
            "--edge-cases",
            "orphans,duplicates",
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
    let dataset: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path()).unwrap()).unwrap();
    assert!(dataset["stats"]["node_count"].as_u64().unwrap() > 0);
    assert_eq!(
        dataset["stats"]["edge_cases_applied"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let (code, _) = run_cli(
        &fixture,
        &[
            "synth",
            "generate",
            "--project",
            &fixture.project_id,
            "--out",
            out.path().to_str().unwrap(),
            "--edge-cases",
            "bogus",
        ],
    );
    assert_eq!(code, 2);
}

#[tokio::test]
async fn test_advisor_analyze_with_gate() {
    let fixture = setup_clean_project().await;
    let queries = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        queries.path(),
        r#"[{"name": "open", "cypher": "MATCH (a), (b) RETURN a, b"}]"#,
    )
    .unwrap();

    let (code, stdout) = run_cli(
        &fixture,
        &[
            "advisor",
            "analyze",
            "--project",
            &fixture.project_id,
            "--queries",
            queries.path().to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(stdout.contains("ADV-UNBOUNDED"), "stdout: {}", stdout);
    assert!(stdout.contains("sizing (estimate)"), "stdout: {}", stdout);

    let (code, _) = run_cli(
        &fixture,
        &[
            "advisor",
            "analyze",
            "--project",
            &fixture.project_id,
            "--queries",
            queries.path().to_str().unwrap(),
            "--fail-on-warning",
        ],
    );
    assert_eq!(code, 1);
}

#[tokio::test]
async fn test_demo_seed_then_validate() {
    let fixture = setup_clean_project().await;
    // Seed a second project via the binary, then validate it by parsed ID.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_relating"))
        .args(["--data-dir", &fixture.data_dir, "--database-url", &fixture.db_url])
        .args(["demo", "seed", "--format", "json"])
        .output()
        .expect("failed to run relating binary");
    assert!(output.status.success());
    let seeded: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("seed prints JSON");
    let demo_id = seeded["project_id"].as_str().unwrap().to_string();
    assert_ne!(demo_id, fixture.project_id);

    let (code, stdout) = run_cli(&fixture, &["validate", "--project", &demo_id]);
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(stdout.contains("Result: PASS"), "stdout: {}", stdout);

    // Text mode also prints a parseable project_id line.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_relating"))
        .args(["--data-dir", &fixture.data_dir, "--database-url", &fixture.db_url])
        .args(["demo", "seed", "--name", "second"])
        .output()
        .expect("failed to run relating binary");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("project_id: "), "stdout: {}", text);
}

#[tokio::test]
async fn test_usage_errors_exit_2() {
    let fixture = setup_clean_project().await;
    let (code, _) = run_cli(&fixture, &["validate"]);
    assert_eq!(code, 2);
    let (code, _) = run_cli(&fixture, &["bogus-command"]);
    assert_eq!(code, 2);
    let (code, _) = run_cli(&fixture, &["validate", "--project", "not-a-uuid"]);
    assert_eq!(code, 2);
}
