use crate::diff::{ChangeKind, SchemaDiff};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProposal {
    /// Suggested versioned filename, e.g. `V003__add_person_email.cypher`.
    pub filename: String,
    /// Additive, idempotent Cypher statements safe to run in order.
    pub cypher: Vec<String>,
    /// Manual remediation steps for breaking changes, in order.
    pub steps: Vec<String>,
    pub notes: Vec<String>,
}

fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("_")
}

/// Build an ordered migration proposal from a schema diff: additive DDL for
/// the safe changes plus explicit manual steps for every breaking change.
pub fn propose_migration(diff: &SchemaDiff, next_version: u32) -> MigrationProposal {
    let mut cypher = Vec::new();
    let mut steps = Vec::new();
    let mut notes = Vec::new();

    for change in &diff.changes {
        match (&change.category[..], change.kind) {
            ("constraint", ChangeKind::Added) => {
                // Subject looks like `Label(prop1,prop2)`.
                if let Some((label, props)) = change.subject.split_once('(') {
                    let props: Vec<String> = props
                        .trim_end_matches(')')
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if props.len() == 1 {
                        cypher.push(relating_migrate::constraint_ddl(label, &props[0], None));
                    } else if !props.is_empty() {
                        notes.push(format!(
                            "Composite constraint '{}' needs a tailored statement; single-property form only",
                            change.subject
                        ));
                    }
                }
                notes.push(format!(
                    "Added constraint '{}': verify existing data complies before enforcing",
                    change.subject
                ));
            }
            ("index", ChangeKind::Added) => {
                if let Some((label, props)) = change.subject.split_once('(') {
                    let props: Vec<String> = props
                        .trim_end_matches(')')
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if let Some(stmt) = relating_migrate::index_ddl(label, &props) {
                        cypher.push(stmt);
                    }
                }
            }
            (_, kind) if change.breaking => {
                steps.push(format!(
                    "[{:?}] {}: {}. Backfill or rewrite affected data, then re-run conformance.",
                    kind, change.subject, change.detail
                ));
            }
            ("embedding_profile", ChangeKind::Added) => {
                notes.push(format!(
                    "New embedding profile '{}': create a matching vector index before querying it",
                    change.subject
                ));
            }
            _ => {}
        }
    }

    if diff.changes.is_empty() {
        notes.push("No changes detected; nothing to migrate.".to_string());
    }

    let headline = diff
        .changes
        .iter()
        .find(|c| !c.breaking)
        .or_else(|| diff.changes.first())
        .map(|c| format!("{:?}_{}", c.kind, c.subject))
        .unwrap_or_else(|| "no_op".to_string());
    let filename = format!("V{:03}__{}.cypher", next_version, slug(&headline));

    MigrationProposal {
        filename,
        cypher,
        steps,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{diff_documents, ChangeKind};
    use crate::document::{IndexDef, SchemaDocument, UniqueConstraint};

    #[test]
    fn test_proposal_generates_additive_ddl() {
        let old = SchemaDocument::new(None);
        let mut new = SchemaDocument::new(None);
        new.constraints.push(UniqueConstraint {
            label: "Person".to_string(),
            properties: vec!["uid".to_string()],
        });
        new.indexes.push(IndexDef {
            label: "Person".to_string(),
            properties: vec!["name".to_string()],
        });

        let diff = diff_documents(&old, &new);
        let proposal = propose_migration(&diff, 3);
        assert_eq!(proposal.filename, "V003__added_person_uid.cypher");
        assert!(proposal.steps.is_empty());
        assert!(proposal
            .cypher
            .iter()
            .any(|s| s.contains("CONSTRAINT") && s.contains("`Person`")));
        assert!(proposal.cypher.iter().any(|s| s.contains("CREATE INDEX")));
    }

    #[test]
    fn test_proposal_lists_manual_steps_for_breaking() {
        let mut old = SchemaDocument::new(None);
        old.ontology.entities.push(relating_core::EntityDefinition {
            label: "Legacy".to_string(),
            display_name: "Legacy".to_string(),
            properties: Vec::new(),
            description: String::new(),
        });
        let new = SchemaDocument::new(None);

        let diff = diff_documents(&old, &new);
        assert!(diff.has_breaking_changes());
        let proposal = propose_migration(&diff, 4);
        assert!(proposal.cypher.is_empty());
        assert_eq!(proposal.steps.len(), 1);
        assert!(proposal.steps[0].contains("Legacy"));
        assert!(proposal.steps[0].contains("Removed"));
        let _ = ChangeKind::Added;
    }

    #[test]
    fn test_empty_diff_produces_noop() {
        let doc = SchemaDocument::new(None);
        let diff = diff_documents(&doc, &doc);
        let proposal = propose_migration(&diff, 1);
        assert!(proposal.filename.starts_with("V001__no_op"));
        assert!(proposal.cypher.is_empty() && proposal.steps.is_empty());
    }
}
