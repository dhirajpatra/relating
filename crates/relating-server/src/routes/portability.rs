use crate::error::AppError;
use crate::routes::aura::{build_backend, get_connection};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use relating_core::{AuraConnectionId, ProjectId};
use relating_metadata::get_project as db_get_project;
use relating_migrate::Neo4jBackend;
use relating_portability::{build_report, scan_repository, to_sarif, TestAssertion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_RUN_TESTS: usize = 200;
pub const MAX_TEST_CYPHER_LEN: usize = 20000;

#[derive(Deserialize)]
pub struct ScanRequest {
    pub repo_path: String,
}

#[derive(Deserialize)]
pub struct TestSpec {
    pub name: String,
    pub cypher: String,
    #[serde(default = "default_read_only")]
    pub read_only: bool,
    #[serde(default)]
    pub assertion: TestAssertion,
}

fn default_read_only() -> bool {
    true
}

#[derive(Deserialize)]
pub struct RunTestsRequest {
    pub connection_id: String,
    pub tests: Vec<TestSpec>,
}

#[derive(Serialize)]
pub struct TestOutcome {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    pub row_count: usize,
    pub elapsed_ms: f64,
}

#[derive(Serialize)]
pub struct RunTestsResponse {
    pub results: Vec<TestOutcome>,
    pub passed: usize,
    pub failed: usize,
}

fn parse_project(id_str: &str) -> Result<ProjectId, AppError> {
    ProjectId::parse(id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))
}

#[derive(Deserialize, Default)]
pub struct ScanQuery {
    /// `json` (default) for the native report, `sarif` for SARIF 2.1.0
    /// (upload with `github/codeql-action/upload-sarif` for PR annotations).
    #[serde(default)]
    pub format: Option<String>,
}

pub async fn scan_repo_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Query(query): Query<ScanQuery>,
    Json(payload): Json<ScanRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let project_id = parse_project(&id_str)?;
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.repo_path.trim().is_empty() {
        return Err(AppError::BadRequest(
            "repo_path cannot be empty".to_string(),
        ));
    }
    // Scanning is CPU/IO-bound but short; run inline like other endpoints.
    let scan = scan_repository(std::path::Path::new(payload.repo_path.trim()))?;
    let report = build_report(&scan);
    match query
        .format
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("json") => serde_json::to_value(&report)
            .map(Json)
            .map_err(|e| AppError::Internal(format!("Failed to serialize report: {}", e))),
        Some("sarif") => Ok(Json(to_sarif(&report))),
        Some(other) => Err(AppError::BadRequest(format!(
            "Unknown format '{}': expected 'json' or 'sarif'",
            other
        ))),
    }
}

pub async fn run_tests_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<RunTestsRequest>,
) -> Result<Json<RunTestsResponse>, AppError> {
    let project_id = parse_project(&id_str)?;
    let _ = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.tests.is_empty() {
        return Err(AppError::BadRequest(
            "At least one test is required".to_string(),
        ));
    }
    if payload.tests.len() > MAX_RUN_TESTS {
        return Err(AppError::BadRequest(format!(
            "At most {} tests per run",
            MAX_RUN_TESTS
        )));
    }

    let conn_id = AuraConnectionId::parse(&payload.connection_id).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid Connection UUID '{}': {}",
            payload.connection_id, e
        ))
    })?;
    let conn = get_connection(&state.pool, &project_id, &conn_id).await?;
    let backend = build_backend(&conn, None)?;

    let mut results = Vec::with_capacity(payload.tests.len());
    for spec in &payload.tests {
        let start = std::time::Instant::now();
        if spec.cypher.trim().is_empty() {
            results.push(TestOutcome {
                name: spec.name.clone(),
                passed: false,
                detail: "Empty query".to_string(),
                row_count: 0,
                elapsed_ms: 0.0,
            });
            continue;
        }
        if spec.cypher.len() > MAX_TEST_CYPHER_LEN {
            results.push(TestOutcome {
                name: spec.name.clone(),
                passed: false,
                detail: format!("Query exceeds {} characters", MAX_TEST_CYPHER_LEN),
                row_count: 0,
                elapsed_ms: 0.0,
            });
            continue;
        }

        // Write-shaped tests run behind EXPLAIN so validation never mutates.
        let effective = if spec.read_only {
            spec.cypher.clone()
        } else {
            format!("EXPLAIN {}", spec.cypher)
        };
        match backend.run_read(&effective, BTreeMap::new()).await {
            Ok(rows) => {
                let (passed, detail) = match &spec.assertion {
                    TestAssertion::NoError => (true, "Query ran without error".to_string()),
                    TestAssertion::RowCountEq { expected } => (
                        rows.len() as u64 == *expected,
                        format!("Expected {} row(s), got {}", expected, rows.len()),
                    ),
                    TestAssertion::RowCountGte { expected } => (
                        rows.len() as u64 >= *expected,
                        format!("Expected at least {} row(s), got {}", expected, rows.len()),
                    ),
                };
                results.push(TestOutcome {
                    name: spec.name.clone(),
                    passed,
                    detail,
                    row_count: rows.len(),
                    elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
                });
            }
            Err(e) => results.push(TestOutcome {
                name: spec.name.clone(),
                passed: false,
                detail: format!("Target error: {}", e),
                row_count: 0,
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            }),
        }
    }

    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    Ok(Json(RunTestsResponse {
        results,
        passed,
        failed,
    }))
}
