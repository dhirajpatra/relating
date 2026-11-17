use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_ai::create_ai_provider;
use relating_core::{ProjectId, RetrievalStrategy};
use relating_debug::{
    build_trace, diagnose_ai, diagnose_rules, likely_causes, DebugTrace, Diagnosis, Severity,
};
use relating_metadata::{get_project as db_get_project, get_retrieval_config};
use relating_retrieval::DEFAULT_TOP_K;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct DebugTraceRequest {
    pub question: String,
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default = "default_true")]
    pub ai_hypothesis: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct DebugTraceResponse {
    pub trace: DebugTrace,
    pub diagnosis: Diagnosis,
}

pub async fn debug_trace_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<DebugTraceRequest>,
) -> Result<Json<DebugTraceResponse>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let question = payload.question.trim().to_string();
    if question.is_empty() {
        return Err(AppError::BadRequest("Question cannot be empty".to_string()));
    }

    let stored = get_retrieval_config(&state.pool, &project_id).await?;
    let strategy = payload
        .strategy
        .or_else(|| stored.as_ref().map(|c| c.default_strategy.clone()))
        .unwrap_or_default();
    let top_k = payload
        .top_k
        .or_else(|| stored.as_ref().map(|c| c.default_top_k))
        .unwrap_or(DEFAULT_TOP_K);

    let provider = create_ai_provider(&project.ai_config)?;
    let graph_path = state.project_graph_path(&project_id);
    let trace = build_trace(
        &state.pool,
        state.graph.as_ref(),
        &graph_path,
        &project,
        &provider,
        &question,
        strategy,
        top_k,
    )
    .await?;

    let mut rules = diagnose_rules(&trace);
    let ai_hypothesis = if payload.ai_hypothesis && !trace.passages.is_empty() {
        match diagnose_ai(&provider, &trace, &rules, &project.ai_config.chat_model).await {
            Some(hypothesis) => Some(hypothesis),
            None => {
                rules.push(relating_debug::DiagnosisRule {
                    code: "DBG-I003".to_string(),
                    severity: Severity::Info,
                    message:
                        "AI hypothesis unavailable: provider call failed; rule findings stand alone"
                            .to_string(),
                });
                None
            }
        }
    } else {
        None
    };
    let likely_causes = likely_causes(&rules);

    Ok(Json(DebugTraceResponse {
        trace,
        diagnosis: Diagnosis {
            rules,
            likely_causes,
            ai_hypothesis,
        },
    }))
}
