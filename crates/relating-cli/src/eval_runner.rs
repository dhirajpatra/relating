use crate::error::CliError;
use relating_ai::AiProvider;
use relating_core::{EvaluationMetrics, Project, QuestionEvaluation, RetrievalStrategy};
use relating_metadata::DbPool;
use relating_retrieval::{
    aggregate_metrics, build_qa_request, citation_precision, extract_citation_markers,
    run_retrieval, RetrievalError, INSUFFICIENT_EVIDENCE_MESSAGE, RAG_PROMPT_VERSION,
};
use std::sync::Arc;

pub struct EvalCliReport {
    pub metrics: EvaluationMetrics,
    pub questions: Vec<QuestionEvaluation>,
    pub prompt_version: String,
}

pub async fn run_eval_questions(
    pool: &DbPool,
    provider: &Arc<dyn AiProvider>,
    project: &Project,
    questions: &[String],
    strategy: RetrievalStrategy,
) -> Result<EvalCliReport, CliError> {
    let mut evaluated = Vec::with_capacity(questions.len());
    for raw_question in questions {
        let question = raw_question.trim();
        if question.is_empty() {
            continue;
        }
        let trace =
            match run_retrieval(pool, provider, project, question, strategy.clone(), 5, None).await
            {
                Ok(trace) => trace,
                Err(RetrievalError::NoEvidence(_)) => {
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
                Err(other) => return Err(CliError::Retrieval(other)),
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

    if evaluated.is_empty() {
        return Err(CliError::Check(
            "No usable evaluation questions".to_string(),
        ));
    }
    let metrics = aggregate_metrics(&evaluated);
    Ok(EvalCliReport {
        metrics,
        questions: evaluated,
        prompt_version: RAG_PROMPT_VERSION.to_string(),
    })
}
