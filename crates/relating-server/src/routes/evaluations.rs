use crate::error::AppError;
use crate::routes::retrieval::{run_retrieval, DEFAULT_TOP_K};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use relating_ai::create_ai_provider;
use relating_core::{
    EvaluationId, EvaluationRun, ProjectId, QuestionEvaluation, RetrievalStrategy,
};
use relating_metadata::{
    get_evaluation as db_get_evaluation, get_project as db_get_project, insert_evaluation,
    list_evaluations as db_list_evaluations,
};
use relating_retrieval::{
    aggregate_metrics, build_qa_request, citation_precision, extract_citation_markers,
    RetrievalError, INSUFFICIENT_EVIDENCE_MESSAGE, RAG_PROMPT_VERSION,
};
use serde::Deserialize;

const MAX_EVAL_QUESTIONS: usize = 50;

#[derive(Deserialize)]
pub struct CreateEvaluationRequest {
    pub questions: Vec<String>,
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub top_k: Option<usize>,
}

pub async fn create_evaluation_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<CreateEvaluationRequest>,
) -> Result<(StatusCode, Json<EvaluationRun>), AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    if payload.questions.is_empty() {
        return Err(AppError::BadRequest(
            "At least one evaluation question is required".to_string(),
        ));
    }
    if payload.questions.len() > MAX_EVAL_QUESTIONS {
        return Err(AppError::BadRequest(format!(
            "At most {} questions per evaluation run",
            MAX_EVAL_QUESTIONS
        )));
    }

    let strategy = payload.strategy.unwrap_or_default();
    let top_k = payload.top_k.unwrap_or(DEFAULT_TOP_K);
    let provider = create_ai_provider(&project.ai_config)?;

    let mut evaluated = Vec::with_capacity(payload.questions.len());
    for raw_question in &payload.questions {
        let question = raw_question.trim();
        if question.is_empty() {
            return Err(AppError::BadRequest(
                "Evaluation questions cannot be empty".to_string(),
            ));
        }

        let trace = match run_retrieval(
            &state,
            &project,
            &provider,
            question,
            strategy.clone(),
            top_k,
            None,
        )
        .await
        {
            Ok(trace) => trace,
            Err(AppError::Retrieval(RetrievalError::NoEvidence(_))) => {
                evaluated.push(QuestionEvaluation {
                    question: question.to_string(),
                    answer: INSUFFICIENT_EVIDENCE_MESSAGE.to_string(),
                    grounded: false,
                    citation_count: 0,
                    citation_precision: 0.0,
                    retrieval_latency_ms: 0.0,
                    answer_latency_ms: 0.0,
                });
                continue;
            }
            Err(other) => return Err(other),
        };

        let answer_start = std::time::Instant::now();
        let chat_req = build_qa_request(question, &trace.passages);
        let chat_resp = provider.chat_completion(&chat_req).await?;
        let answer_latency_ms = answer_start.elapsed().as_secs_f64() * 1000.0;

        let markers = extract_citation_markers(&chat_resp.content);
        evaluated.push(QuestionEvaluation {
            question: question.to_string(),
            answer: chat_resp.content,
            grounded: !trace.passages.is_empty(),
            citation_count: markers.len(),
            citation_precision: citation_precision(&markers, trace.passages.len()),
            retrieval_latency_ms: trace.latency_ms,
            answer_latency_ms,
        });
    }

    let run = EvaluationRun {
        id: EvaluationId::new(),
        project_id,
        strategy,
        chat_model: project.ai_config.chat_model.clone(),
        embedding_model: project.ai_config.embedding_model.clone(),
        prompt_version: RAG_PROMPT_VERSION.to_string(),
        metrics: aggregate_metrics(&evaluated),
        questions: evaluated,
        created_at: Utc::now(),
    };

    insert_evaluation(&state.pool, &run).await?;

    Ok((StatusCode::CREATED, Json(run)))
}

pub async fn list_evaluations_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<Vec<EvaluationRun>>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let runs = db_list_evaluations(&state.pool, &project_id).await?;
    Ok(Json(runs))
}

pub async fn get_evaluation_handler(
    State(state): State<AppState>,
    Path((id_str, eval_id_str)): Path<(String, String)>,
) -> Result<Json<EvaluationRun>, AppError> {
    let _project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid Project UUID '{}': {}", id_str, e)))?;
    let eval_id = EvaluationId::parse(&eval_id_str).map_err(|e| {
        AppError::BadRequest(format!("Invalid Evaluation UUID '{}': {}", eval_id_str, e))
    })?;

    let run = db_get_evaluation(&state.pool, &eval_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Evaluation '{}' not found", eval_id)))?;

    Ok(Json(run))
}
