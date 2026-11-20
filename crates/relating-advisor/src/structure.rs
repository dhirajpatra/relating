use crate::queries::{QueryFinding, Severity};
use relating_graph::GraphBackend;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureNote {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    pub suggestion: String,
    /// Measured numbers are observed; heuristic thresholds say so explicitly.
    pub basis: String,
}

fn note(
    rule_id: &str,
    severity: Severity,
    message: impl Into<String>,
    suggestion: impl Into<String>,
    basis: impl Into<String>,
) -> StructureNote {
    StructureNote {
        rule_id: rule_id.to_string(),
        severity,
        message: message.into(),
        suggestion: suggestion.into(),
        basis: basis.into(),
    }
}

/// Structural review of the live graph: uid coverage, label scale, skew.
/// Counts are measured; threshold judgments are labeled heuristic.
pub fn analyze_structure(
    graph: &dyn GraphBackend,
    store_path: &Path,
) -> Result<Vec<StructureNote>, relating_graph::GraphError> {
    let mut notes = Vec::new();
    let schema = graph.get_schema(store_path).unwrap_or_default();

    // ADV-UID: merge-key coverage gates idempotent migration and lookups.
    match graph.execute_read(
        store_path,
        "MATCH (n) WHERE n.uid IS NULL RETURN count(n) AS c",
        HashMap::new(),
        None,
    ) {
        Ok(page) => {
            let missing = page
                .rows
                .first()
                .and_then(|r| r.get("c"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if missing > 0 {
                notes.push(note(
                    "ADV-UID",
                    Severity::Warning,
                    format!("{} node(s) lack uid merge keys", missing),
                    "Backfill uid, then add a unique constraint per label",
                    "measured: live count query",
                ));
            } else if schema.node_count > 0 {
                notes.push(note(
                    "ADV-UID",
                    Severity::Info,
                    format!("All {} node(s) carry uid merge keys", schema.node_count),
                    "Keep uid unique per label for fast MERGE lookups",
                    "measured: live count query",
                ));
            }
        }
        Err(e) => notes.push(note(
            "ADV-UID",
            Severity::Info,
            format!("uid coverage unchecked: {}", e),
            "Verify merge-key coverage manually",
            "check failed; not measured",
        )),
    }

    // ADV-SCALE: heuristic scale bands per label (thresholds are heuristic).
    let label_counts = graph.get_label_counts(store_path).unwrap_or_default();
    for (label, count) in &label_counts {
        if *count >= 1_000_000 {
            notes.push(note(
                "ADV-SCALE",
                Severity::Warning,
                format!(
                    "Label '{}' holds {} nodes: traversals need selective indexed entry points",
                    label, count
                ),
                "Enter through indexed lookups and bound every expansion",
                "heuristic threshold (>=1M); counts measured",
            ));
        }
    }

    // ADV-SKEW: one relationship type dominating the graph.
    let rel_counts = graph.get_rel_type_counts(store_path).unwrap_or_default();
    let total_rels: u64 = rel_counts.iter().map(|(_, c)| c).sum();
    for (rel_type, count) in &rel_counts {
        if total_rels > 0 && *count as f64 / total_rels as f64 > 0.9 && total_rels >= 10_000 {
            notes.push(note(
                "ADV-SKEW",
                Severity::Info,
                format!(
                    "Relationship '{}' is {:.0}% of all relationships: dense areas need bounded traversals",
                    rel_type,
                    *count as f64 / total_rels as f64 * 100.0
                ),
                "Bound variable-length traversals crossing this type",
                "heuristic threshold (>90% of >=10k); counts measured",
            ));
        }
    }

    if notes.is_empty() {
        notes.push(note(
            "ADV-OK",
            Severity::Info,
            "No structural concerns detected",
            "Re-run after significant data growth",
            "measured: live schema inspection",
        ));
    }
    Ok(notes)
}

/// Re-export for report assembly without leaking internals.
pub fn finding_from_note(note: &StructureNote) -> QueryFinding {
    QueryFinding {
        rule_id: note.rule_id.clone(),
        severity: note.severity,
        message: note.message.clone(),
        suggestion: format!("{} [{}]", note.suggestion, note.basis),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_graph::RelateBackend;
    use tempfile::tempdir;

    #[test]
    fn test_uid_coverage_detected() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        backend
            .execute_write(
                &store,
                "CREATE (a:Person {uid: 'Person:1'}) CREATE (b:Person {name: 'x'})",
                HashMap::new(),
            )
            .unwrap();

        let notes = analyze_structure(&backend, &store).unwrap();
        let uid = notes.iter().find(|n| n.rule_id == "ADV-UID").unwrap();
        assert_eq!(uid.severity, Severity::Warning);
        assert!(uid.message.contains('1'));
        assert!(uid.basis.contains("measured"));
    }

    #[test]
    fn test_empty_graph_is_quiet() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("graph.rl");
        let backend = RelateBackend::new();
        backend.ensure_store(&store).unwrap();
        let notes = analyze_structure(&backend, &store).unwrap();
        assert!(notes.iter().any(|n| n.rule_id == "ADV-OK"));
    }
}
