use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use relating_ai::create_ai_provider;
use relating_core::{
    EmbeddingProfileId, ProjectId, QuestionAnswer, RetrievalStrategy, RetrievalTrace,
};
use relating_metadata::{get_project as db_get_project, get_retrieval_config};
use relating_retrieval::{
    build_citations, build_qa_request, extract_citation_markers, run_retrieval, RetrievalError,
    DEFAULT_TOP_K, INSUFFICIENT_EVIDENCE_MESSAGE, RAG_PROMPT_VERSION,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct QuestionRequest {
    pub question: String,
    #[serde(default)]
    pub strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

pub async fn ask_question_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(payload): Json<QuestionRequest>,
) -> Result<Json<QuestionAnswer>, AppError> {
    let project_id = ProjectId::parse(&id_str)
        .map_err(|e| AppError::BadRequest(format!("Invalid UUID '{}': {}", id_str, e)))?;

    let project = db_get_project(&state.pool, &project_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project with ID '{}' not found", project_id)))?;

    let question = payload.question.trim().to_string();
    if question.is_empty() {
        return Err(AppError::BadRequest("Question cannot be empty".to_string()));
    }

    let stored = get_retrieval_config(&state.pool, &project_id).await?;
    let strategy = payload.strategy.or_else(|| stored.as_ref().map(|c| c.default_strategy.clone())).unwrap_or_default();
    let top_k = payload.top_k.or_else(|| stored.as_ref().map(|c| c.default_top_k)).unwrap_or(DEFAULT_TOP_K);
    let provider = create_ai_provider(&project.ai_config)?;
    let start_time = std::time::Instant::now();

    let trace = match run_retrieval(
        &state.pool,
        &provider,
        &project,
        &question,
        strategy.clone(),
        top_k,
        payload.profile_name.as_deref(),
    )
    .await
    {
        Ok(trace) => trace,
        Err(RetrievalError::NoEvidence(_)) => {
            // Honest ungrounded answer: no evidence was retrieved, so the
            // model is never called and nothing is fabricated.
            return Ok(Json(QuestionAnswer {
                question,
                answer: INSUFFICIENT_EVIDENCE_MESSAGE.to_string(),
                grounded: false,
                citations: Vec::new(),
                trace: RetrievalTrace {
                    query: payload.question.trim().to_string(),
                    strategy,
                    profile_id: EmbeddingProfileId(uuid::Uuid::nil()),
                    profile_name: "none".to_string(),
                    embedding_model: project.ai_config.embedding_model.clone(),
                    dimensions: 0,
                    top_k,
                    passages: Vec::new(),
                    latency_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                },
                chat_model: project.ai_config.chat_model.clone(),
                prompt_version: RAG_PROMPT_VERSION.to_string(),
                latency_ms: start_time.elapsed().as_secs_f64() * 1000.0,
            }));
        }
        Err(other) => return Err(AppError::Retrieval(other)),
    };

    let chat_req = build_qa_request(&question, &trace.passages);
    let chat_resp = provider.chat_completion(&chat_req).await?;

    let markers = extract_citation_markers(&chat_resp.content);
    let (citations, _invalid_markers) = build_citations(&trace.passages, &markers);

    Ok(Json(QuestionAnswer {
        question,
        answer: chat_resp.content,
        grounded: !trace.passages.is_empty(),
        citations,
        trace,
        chat_model: project.ai_config.chat_model.clone(),
        prompt_version: RAG_PROMPT_VERSION.to_string(),
        latency_ms: start_time.elapsed().as_secs_f64() * 1000.0,
    }))
}
