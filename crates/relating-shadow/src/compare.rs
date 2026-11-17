use crate::replay::QueryResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompareVerdict {
    Match,
    StructureDiff,
    CountDiff,
    ErrorDiff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryComparison {
    pub name: String,
    pub verdict: CompareVerdict,
    pub detail: String,
    pub source_rows: usize,
    pub target_rows: usize,
    pub source_latency_ms: f64,
    pub target_latency_ms: f64,
    /// Target/source latency ratio; >1 means the target was slower.
    pub latency_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowSummary {
    pub total: usize,
    pub matched: usize,
    pub structure_diffs: usize,
    pub count_diffs: usize,
    pub error_diffs: usize,
    pub source_success_rate: f32,
    pub target_success_rate: f32,
    pub confidence: Confidence,
    pub confidence_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReport {
    pub comparisons: Vec<QueryComparison>,
    pub summary: ShadowSummary,
}

pub fn compare_query(name: &str, source: &QueryResult, target: &QueryResult) -> QueryComparison {
    let latency_ratio = if source.latency_ms > 0.0 {
        target.latency_ms / source.latency_ms
    } else {
        1.0
    };

    if source.success != target.success {
        return QueryComparison {
            name: name.to_string(),
            verdict: CompareVerdict::ErrorDiff,
            detail: format!(
                "Success differs: source {} vs target {}. source_err={:?} target_err={:?}",
                status_word(source.success),
                status_word(target.success),
                source.error,
                target.error
            ),
            source_rows: source.row_count,
            target_rows: target.row_count,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    if !source.success {
        // Both failed: same failure class is still a match on behavior.
        let same_class = error_class(&source.error) == error_class(&target.error);
        return QueryComparison {
            name: name.to_string(),
            verdict: if same_class {
                CompareVerdict::Match
            } else {
                CompareVerdict::ErrorDiff
            },
            detail: format!(
                "Both sides failed ({} vs {})",
                error_class(&source.error),
                error_class(&target.error)
            ),
            source_rows: 0,
            target_rows: 0,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    // Both succeeded: compare structure, then counts, then content.
    if source.row_count == 0 && target.row_count == 0 {
        return QueryComparison {
            name: name.to_string(),
            verdict: CompareVerdict::Match,
            detail: "Both sides returned zero rows".to_string(),
            source_rows: 0,
            target_rows: 0,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    // Positional columns (c0..cN) from Bolt carry no names; compare arity and
    // per-position types instead of names.
    let arity_match = source.columns.len() == target.columns.len();
    let types_match = source.column_types.len() == target.column_types.len()
        && source
            .column_types
            .values()
            .zip(target.column_types.values())
            .all(|(a, b)| a == b);
    if !arity_match || !types_match {
        return QueryComparison {
            name: name.to_string(),
            verdict: CompareVerdict::StructureDiff,
            detail: format!(
                "Shape differs: source {} col(s) {:?} vs target {} col(s) {:?}",
                source.columns.len(),
                source.column_types,
                target.columns.len(),
                target.column_types
            ),
            source_rows: source.row_count,
            target_rows: target.row_count,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    if source.row_count != target.row_count {
        return QueryComparison {
            name: name.to_string(),
            verdict: CompareVerdict::CountDiff,
            detail: format!(
                "Row counts differ: source {} vs target {}",
                source.row_count, target.row_count
            ),
            source_rows: source.row_count,
            target_rows: target.row_count,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    if source.content_hash != target.content_hash {
        return QueryComparison {
            name: name.to_string(),
            verdict: CompareVerdict::CountDiff,
            detail: "Same shape and row count, but content hashes differ (row values differ)"
                .to_string(),
            source_rows: source.row_count,
            target_rows: target.row_count,
            source_latency_ms: source.latency_ms,
            target_latency_ms: target.latency_ms,
            latency_ratio,
        };
    }

    QueryComparison {
        name: name.to_string(),
        verdict: CompareVerdict::Match,
        detail: format!("{} row(s), identical content", source.row_count),
        source_rows: source.row_count,
        target_rows: target.row_count,
        source_latency_ms: source.latency_ms,
        target_latency_ms: target.latency_ms,
        latency_ratio,
    }
}

fn status_word(success: bool) -> &'static str {
    if success {
        "ok"
    } else {
        "error"
    }
}

fn error_class(error: &Option<String>) -> String {
    match error {
        None => "ok".to_string(),
        Some(message) => {
            let lower = message.to_lowercase();
            if lower.contains("syntax") {
                "syntax".to_string()
            } else if lower.contains("not found") || lower.contains("unknown") {
                "not_found".to_string()
            } else if lower.contains("timeout") {
                "timeout".to_string()
            } else {
                "other".to_string()
            }
        }
    }
}

/// Confidence = 0.7 * match_rate + 0.3 * min(success rates), mapped to
/// HIGH (>=0.9), MEDIUM (>=0.6), LOW otherwise. Any error-diff caps at MEDIUM.
pub fn summarize(comparisons: &[QueryComparison]) -> ShadowSummary {
    let total = comparisons.len();
    if total == 0 {
        return ShadowSummary {
            total: 0,
            matched: 0,
            structure_diffs: 0,
            count_diffs: 0,
            error_diffs: 0,
            source_success_rate: 0.0,
            target_success_rate: 0.0,
            confidence: Confidence::Low,
            confidence_reasons: vec!["No queries were replayed".to_string()],
        };
    }

    let matched = comparisons
        .iter()
        .filter(|c| c.verdict == CompareVerdict::Match)
        .count();
    let structure_diffs = comparisons
        .iter()
        .filter(|c| c.verdict == CompareVerdict::StructureDiff)
        .count();
    let count_diffs = comparisons
        .iter()
        .filter(|c| c.verdict == CompareVerdict::CountDiff)
        .count();
    let error_diffs = comparisons
        .iter()
        .filter(|c| c.verdict == CompareVerdict::ErrorDiff)
        .count();

    // Success rates are derived from verdicts: only ErrorDiff implies failure.
    let error_rate = error_diffs as f32 / total as f32;
    let source_success_rate = 1.0 - error_rate;
    let target_success_rate = 1.0 - error_rate;

    let match_rate = matched as f32 / total as f32;
    let score = 0.7 * match_rate + 0.3 * source_success_rate.min(target_success_rate);
    let mut confidence = if score >= 0.9 {
        Confidence::High
    } else if score >= 0.6 {
        Confidence::Medium
    } else {
        Confidence::Low
    };
    if error_diffs > 0 && confidence == Confidence::High {
        confidence = Confidence::Medium;
    }

    let mut reasons = vec![format!(
        "{}/{} queries matched (score {:.2})",
        matched, total, score
    )];
    if error_diffs > 0 {
        reasons.push(format!(
            "{} {} failed on exactly one side",
            error_diffs,
            if error_diffs == 1 { "query" } else { "queries" }
        ));
    }
    if structure_diffs > 0 {
        reasons.push(format!("{} structure difference(s)", structure_diffs));
    }
    if count_diffs > 0 {
        reasons.push(format!("{} count/content difference(s)", count_diffs));
    }

    ShadowSummary {
        total,
        matched,
        structure_diffs,
        count_diffs,
        error_diffs,
        source_success_rate,
        target_success_rate,
        confidence,
        confidence_reasons: reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn ok_result(rows: usize, hash: &str) -> QueryResult {
        QueryResult {
            success: true,
            error: None,
            columns: vec!["c0".to_string()],
            row_count: rows,
            column_types: BTreeMap::from([("c0".to_string(), vec!["string".to_string()])]),
            content_hash: hash.to_string(),
            latency_ms: 2.0,
            sample_rows: None,
        }
    }

    #[test]
    fn test_match_verdict() {
        let c = compare_query("q", &ok_result(2, "h"), &ok_result(2, "h"));
        assert_eq!(c.verdict, CompareVerdict::Match);
    }

    #[test]
    fn test_count_and_content_diffs() {
        let c = compare_query("q", &ok_result(2, "h"), &ok_result(3, "h"));
        assert_eq!(c.verdict, CompareVerdict::CountDiff);
        let c = compare_query("q", &ok_result(2, "h1"), &ok_result(2, "h2"));
        assert_eq!(c.verdict, CompareVerdict::CountDiff);
    }

    #[test]
    fn test_structure_diff_on_arity() {
        let mut target = ok_result(1, "h");
        target.columns = vec!["c0".to_string(), "c1".to_string()];
        target
            .column_types
            .insert("c1".to_string(), vec!["string".to_string()]);
        let c = compare_query("q", &ok_result(1, "h"), &target);
        assert_eq!(c.verdict, CompareVerdict::StructureDiff);
    }

    #[test]
    fn test_error_diff_and_both_failed() {
        let mut failed = ok_result(0, "");
        failed.success = false;
        failed.error = Some("syntax boom".to_string());
        let c = compare_query("q", &ok_result(0, ""), &failed);
        assert_eq!(c.verdict, CompareVerdict::ErrorDiff);

        let mut failed2 = failed.clone();
        failed2.error = Some("syntax bust".to_string());
        let c = compare_query("q", &failed, &failed2);
        assert_eq!(c.verdict, CompareVerdict::Match); // same failure class
    }

    #[test]
    fn test_confidence_formula() {
        let comparisons = vec![
            compare_query("a", &ok_result(1, "h"), &ok_result(1, "h")),
            compare_query("b", &ok_result(1, "h"), &ok_result(1, "h")),
        ];
        let summary = summarize(&comparisons);
        assert_eq!(summary.confidence, Confidence::High);

        let comparisons = vec![
            compare_query("a", &ok_result(1, "h"), &ok_result(1, "h")),
            compare_query("b", &ok_result(1, "h"), &ok_result(2, "h")),
        ];
        let summary = summarize(&comparisons);
        assert_eq!(summary.confidence, Confidence::Medium);
    }
}
