use chrono::{DateTime, Utc};
use relating_core::{
    EvaluationId, EvaluationMetrics, EvaluationRun, ProjectId, QuestionEvaluation, RelatingError,
    RetrievalStrategy,
};
use sqlx::{Row, SqlitePool};

fn strategy_to_str(strategy: &RetrievalStrategy) -> &'static str {
    match strategy {
        RetrievalStrategy::Vector => "vector",
        RetrievalStrategy::Keyword => "keyword",
        RetrievalStrategy::Hybrid => "hybrid",
    }
}

fn strategy_from_str(s: &str) -> RetrievalStrategy {
    match s {
        "vector" => RetrievalStrategy::Vector,
        "keyword" => RetrievalStrategy::Keyword,
        _ => RetrievalStrategy::Hybrid,
    }
}

pub async fn insert_evaluation(
    pool: &SqlitePool,
    run: &EvaluationRun,
) -> Result<(), RelatingError> {
    let metrics_json = serde_json::to_string(&run.metrics)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize metrics: {}", e)))?;
    let questions_json = serde_json::to_string(&run.questions)
        .map_err(|e| RelatingError::Internal(format!("Failed to serialize questions: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO evaluations (id, project_id, strategy, chat_model, embedding_model, prompt_version, metrics_version, metrics_json, questions_json, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(run.id.to_string())
    .bind(run.project_id.to_string())
    .bind(strategy_to_str(&run.strategy))
    .bind(&run.chat_model)
    .bind(&run.embedding_model)
    .bind(&run.prompt_version)
    .bind(&run.metrics_version)
    .bind(metrics_json)
    .bind(questions_json)
    .bind(run.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert evaluation: {}", e)))?;

    Ok(())
}

fn map_evaluation_row(row: sqlx::sqlite::SqliteRow) -> Result<EvaluationRun, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let strategy_str: String = row.get("strategy");
    let chat_model: String = row.get("chat_model");
    let embedding_model: String = row.get("embedding_model");
    let prompt_version: String = row.get("prompt_version");
    let metrics_version: String = row.get("metrics_version");
    let metrics_json: String = row.get("metrics_json");
    let questions_json: String = row.get("questions_json");
    let created_at_str: String = row.get("created_at");

    let id = EvaluationId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Evaluation UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);

    let metrics: EvaluationMetrics = serde_json::from_str(&metrics_json)
        .map_err(|e| RelatingError::Internal(format!("Failed to deserialize metrics: {}", e)))?;
    let questions: Vec<QuestionEvaluation> = serde_json::from_str(&questions_json)
        .map_err(|e| RelatingError::Internal(format!("Failed to deserialize questions: {}", e)))?;

    Ok(EvaluationRun {
        id,
        project_id,
        strategy: strategy_from_str(&strategy_str),
        chat_model,
        embedding_model,
        prompt_version,
        metrics_version,
        metrics,
        questions,
        created_at,
    })
}

pub async fn get_evaluation(
    pool: &SqlitePool,
    id: &EvaluationId,
) -> Result<Option<EvaluationRun>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, strategy, chat_model, embedding_model, prompt_version, metrics_version, metrics_json, questions_json, created_at
        FROM evaluations
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch evaluation: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_evaluation_row(row)?)),
        None => Ok(None),
    }
}

pub async fn list_evaluations(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<EvaluationRun>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, strategy, chat_model, embedding_model, prompt_version, metrics_version, metrics_json, questions_json, created_at
        FROM evaluations
        WHERE project_id = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list evaluations: {}", e)))?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        runs.push(map_evaluation_row(row)?);
    }
    Ok(runs)
}
