use relating_core::{EvaluationMetrics, QuestionEvaluation};

/// Fraction of cited passages that were actually retrieved (valid markers).
/// A question with no citations scores 0.0: an uncited answer is ungrounded.
pub fn citation_precision(cited_markers: &[u32], retrieved_count: usize) -> f32 {
    if cited_markers.is_empty() || retrieved_count == 0 {
        return 0.0;
    }
    let valid = cited_markers
        .iter()
        .filter(|m| **m >= 1 && (**m as usize) <= retrieved_count)
        .count();
    valid as f32 / cited_markers.len() as f32
}

pub fn aggregate_metrics(questions: &[QuestionEvaluation]) -> EvaluationMetrics {
    if questions.is_empty() {
        return EvaluationMetrics {
            question_count: 0,
            grounded_count: 0,
            grounded_rate: 0.0,
            mean_citation_precision: 0.0,
            mean_latency_ms: 0.0,
        };
    }
    let grounded_count = questions.iter().filter(|q| q.grounded).count();
    let mean_precision =
        questions.iter().map(|q| q.citation_precision).sum::<f32>() / questions.len() as f32;
    let mean_latency = questions
        .iter()
        .map(|q| q.retrieval_latency_ms + q.answer_latency_ms)
        .sum::<f64>()
        / questions.len() as f64;

    EvaluationMetrics {
        question_count: questions.len(),
        grounded_count,
        grounded_rate: grounded_count as f32 / questions.len() as f32,
        mean_citation_precision: mean_precision,
        mean_latency_ms: mean_latency,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_citation_precision_counts_valid_markers() {
        assert!((citation_precision(&[1, 2], 2) - 1.0).abs() < 1e-5);
        assert!((citation_precision(&[1, 5], 2) - 0.5).abs() < 1e-5);
        assert_eq!(citation_precision(&[], 3), 0.0);
        assert_eq!(citation_precision(&[1], 0), 0.0);
    }

    #[test]
    fn test_aggregate_metrics_empty_is_zeroed() {
        let metrics = aggregate_metrics(&[]);
        assert_eq!(metrics.question_count, 0);
        assert_eq!(metrics.grounded_rate, 0.0);
    }

    #[test]
    fn test_aggregate_metrics_averages_questions() {
        let questions = vec![
            QuestionEvaluation {
                question: "q1".to_string(),
                answer: "a1".to_string(),
                grounded: true,
                citation_count: 2,
                citation_precision: 1.0,
                retrieval_latency_ms: 10.0,
                answer_latency_ms: 20.0,
            },
            QuestionEvaluation {
                question: "q2".to_string(),
                answer: "a2".to_string(),
                grounded: false,
                citation_count: 0,
                citation_precision: 0.0,
                retrieval_latency_ms: 10.0,
                answer_latency_ms: 0.0,
            },
        ];
        let metrics = aggregate_metrics(&questions);
        assert_eq!(metrics.question_count, 2);
        assert_eq!(metrics.grounded_count, 1);
        assert!((metrics.grounded_rate - 0.5).abs() < 1e-5);
        assert!((metrics.mean_citation_precision - 0.5).abs() < 1e-5);
        assert!((metrics.mean_latency_ms - 20.0).abs() < 1e-5);
    }
}
