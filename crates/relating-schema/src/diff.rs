use crate::document::SchemaDocument;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaChange {
    pub category: String,
    pub subject: String,
    pub kind: ChangeKind,
    pub breaking: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDiff {
    pub changes: Vec<SchemaChange>,
    pub breaking_count: usize,
}

impl SchemaDiff {
    pub fn has_breaking_changes(&self) -> bool {
        self.breaking_count > 0
    }
}

fn change(
    category: &str,
    subject: impl Into<String>,
    kind: ChangeKind,
    breaking: bool,
    detail: impl Into<String>,
) -> SchemaChange {
    SchemaChange {
        category: category.to_string(),
        subject: subject.into(),
        kind,
        breaking,
        detail: detail.into(),
    }
}

/// Diff `old` against `new`, classifying every change as breaking or safe.
/// Removals and type/space changes break consumers; pure additions do not.
pub fn diff_documents(old: &SchemaDocument, new: &SchemaDocument) -> SchemaDiff {
    let mut changes = Vec::new();

    // Entities keyed by label.
    let old_entities: std::collections::HashMap<&str, _> = old
        .ontology
        .entities
        .iter()
        .map(|e| (e.label.as_str(), e))
        .collect();
    let new_entities: std::collections::HashMap<&str, _> = new
        .ontology
        .entities
        .iter()
        .map(|e| (e.label.as_str(), e))
        .collect();

    for label in old_entities.keys() {
        if !new_entities.contains_key(label) {
            changes.push(change(
                "entity",
                label.to_string(),
                ChangeKind::Removed,
                true,
                format!("Node label '{}' was removed", label),
            ));
        }
    }
    for label in new_entities.keys() {
        if !old_entities.contains_key(label) {
            changes.push(change(
                "entity",
                label.to_string(),
                ChangeKind::Added,
                false,
                format!("Node label '{}' was added", label),
            ));
        }
    }

    // Properties within surviving entities.
    for (label, old_entity) in &old_entities {
        let Some(new_entity) = new_entities.get(label) else {
            continue;
        };
        let old_props: std::collections::HashMap<&str, _> = old_entity
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p))
            .collect();
        let new_props: std::collections::HashMap<&str, _> = new_entity
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p))
            .collect();
        for name in old_props.keys() {
            if !new_props.contains_key(name) {
                changes.push(change(
                    "property",
                    format!("{}.{}", label, name),
                    ChangeKind::Removed,
                    true,
                    format!("Property '{}.{}' was removed", label, name),
                ));
            }
        }
        for (name, new_prop) in &new_props {
            match old_props.get(name) {
                None => changes.push(change(
                    "property",
                    format!("{}.{}", label, name),
                    ChangeKind::Added,
                    false,
                    format!("Property '{}.{}' was added", label, name),
                )),
                Some(old_prop) => {
                    if old_prop.data_type != new_prop.data_type {
                        changes.push(change(
                            "property",
                            format!("{}.{}", label, name),
                            ChangeKind::Modified,
                            true,
                            format!(
                                "Property '{}.{}' changed type {} -> {}",
                                label, name, old_prop.data_type, new_prop.data_type
                            ),
                        ));
                    }
                    if old_prop.is_primary_key != new_prop.is_primary_key {
                        changes.push(change(
                            "property",
                            format!("{}.{}", label, name),
                            ChangeKind::Modified,
                            true,
                            format!("Property '{}.{}' primary-key flag changed", label, name),
                        ));
                    }
                }
            }
        }
    }

    // Relationships keyed by rel_type + endpoints.
    let rel_key = |r: &relating_core::RelationshipDefinition| {
        format!("{}:{}->{}", r.rel_type, r.source_label, r.target_label)
    };
    let old_rels: std::collections::HashSet<String> =
        old.ontology.relationships.iter().map(rel_key).collect();
    let new_rels: std::collections::HashSet<String> =
        new.ontology.relationships.iter().map(rel_key).collect();
    for key in old_rels.difference(&new_rels) {
        changes.push(change(
            "relationship",
            key.clone(),
            ChangeKind::Removed,
            true,
            format!("Relationship '{}' was removed", key),
        ));
    }
    for key in new_rels.difference(&old_rels) {
        changes.push(change(
            "relationship",
            key.clone(),
            ChangeKind::Added,
            false,
            format!("Relationship '{}' was added", key),
        ));
    }

    // Constraints: removals break guarantees; additions may fail on dirty data.
    let constraint_key =
        |c: &crate::document::UniqueConstraint| format!("{}({})", c.label, c.properties.join(","));
    let old_constraints: std::collections::HashSet<String> =
        old.constraints.iter().map(constraint_key).collect();
    let new_constraints: std::collections::HashSet<String> =
        new.constraints.iter().map(constraint_key).collect();
    for key in old_constraints.difference(&new_constraints) {
        changes.push(change(
            "constraint",
            key.clone(),
            ChangeKind::Removed,
            true,
            format!("Unique constraint '{}' was removed", key),
        ));
    }
    for key in new_constraints.difference(&old_constraints) {
        changes.push(change(
            "constraint",
            key.clone(),
            ChangeKind::Added,
            false,
            format!(
                "Unique constraint '{}' was added (verify existing data complies)",
                key
            ),
        ));
    }

    // Indexes are never breaking (performance-only).
    let index_key =
        |i: &crate::document::IndexDef| format!("{}({})", i.label, i.properties.join(","));
    let old_indexes: std::collections::HashSet<String> =
        old.indexes.iter().map(index_key).collect();
    let new_indexes: std::collections::HashSet<String> =
        new.indexes.iter().map(index_key).collect();
    for key in old_indexes.symmetric_difference(&new_indexes) {
        let kind = if new_indexes.contains(key) {
            ChangeKind::Added
        } else {
            ChangeKind::Removed
        };
        changes.push(change(
            "index",
            key.clone(),
            kind,
            false,
            format!("Index '{}' changed (performance-only, non-breaking)", key),
        ));
    }

    // Embedding profiles: model/dimension changes break the vector space.
    let old_profiles: std::collections::HashMap<&str, _> = old
        .embedding_profiles
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let new_profiles: std::collections::HashMap<&str, _> = new
        .embedding_profiles
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    for name in old_profiles.keys() {
        if !new_profiles.contains_key(name) {
            changes.push(change(
                "embedding_profile",
                name.to_string(),
                ChangeKind::Removed,
                true,
                format!(
                    "Embedding profile '{}' was removed; its index is orphaned",
                    name
                ),
            ));
        }
    }
    for (name, new_profile) in &new_profiles {
        match old_profiles.get(name) {
            None => changes.push(change(
                "embedding_profile",
                name.to_string(),
                ChangeKind::Added,
                false,
                format!("Embedding profile '{}' was added", name),
            )),
            Some(old_profile) => {
                if old_profile.model != new_profile.model
                    || old_profile.dimensions != new_profile.dimensions
                {
                    changes.push(change(
                        "embedding_profile",
                        name.to_string(),
                        ChangeKind::Modified,
                        true,
                        format!(
                            "Embedding profile '{}' changed space {}:{}d -> {}:{}d; existing vectors are incompatible",
                            name,
                            old_profile.model,
                            old_profile.dimensions,
                            new_profile.model,
                            new_profile.dimensions
                        ),
                    ));
                }
            }
        }
    }

    // Retrieval config is behavior-tuning, never breaking.
    if old.retrieval.default_strategy != new.retrieval.default_strategy {
        changes.push(change(
            "retrieval",
            "default_strategy".to_string(),
            ChangeKind::Modified,
            false,
            "Default retrieval strategy changed".to_string(),
        ));
    }
    if old.retrieval.default_top_k != new.retrieval.default_top_k {
        changes.push(change(
            "retrieval",
            "default_top_k".to_string(),
            ChangeKind::Modified,
            false,
            format!(
                "Default top_k changed {} -> {}",
                old.retrieval.default_top_k, new.retrieval.default_top_k
            ),
        ));
    }

    changes.sort_by(|a, b| (&a.category, &a.subject).cmp(&(&b.category, &b.subject)));
    let breaking_count = changes.iter().filter(|c| c.breaking).count();
    SchemaDiff {
        changes,
        breaking_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ProfileSpec, SchemaDocument, UniqueConstraint};
    use relating_core::{EntityDefinition, PropertyDefinition, RelationshipDefinition};

    fn entity(label: &str, props: Vec<(&str, &str)>) -> EntityDefinition {
        EntityDefinition {
            label: label.to_string(),
            display_name: label.to_string(),
            properties: props
                .into_iter()
                .map(|(name, data_type)| PropertyDefinition {
                    name: name.to_string(),
                    data_type: data_type.to_string(),
                    is_primary_key: false,
                    description: None,
                })
                .collect(),
            description: String::new(),
        }
    }

    #[test]
    fn test_identical_documents_have_no_changes() {
        let doc = SchemaDocument::new(None);
        let diff = diff_documents(&doc, &doc);
        assert!(diff.changes.is_empty());
        assert!(!diff.has_breaking_changes());
    }

    #[test]
    fn test_removals_and_type_changes_are_breaking() {
        let mut old = SchemaDocument::new(None);
        old.ontology.entities.push(entity(
            "Person",
            vec![("name", "string"), ("age", "integer")],
        ));
        old.ontology.relationships.push(RelationshipDefinition {
            rel_type: "KNOWS".to_string(),
            source_label: "Person".to_string(),
            target_label: "Person".to_string(),
            description: String::new(),
        });

        let mut new = SchemaDocument::new(None);
        new.ontology.entities.push(entity(
            "Person",
            vec![("name", "string"), ("age", "string")],
        ));

        let diff = diff_documents(&old, &new);
        assert!(diff.has_breaking_changes());
        assert!(diff
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Removed && c.category == "relationship" && c.breaking));
        assert!(diff
            .changes
            .iter()
            .any(|c| c.subject == "Person.age" && c.kind == ChangeKind::Modified && c.breaking));
    }

    #[test]
    fn test_additions_are_safe() {
        let old = SchemaDocument::new(None);
        let mut new = SchemaDocument::new(None);
        new.ontology
            .entities
            .push(entity("Person", vec![("name", "string")]));
        new.constraints.push(UniqueConstraint {
            label: "Person".to_string(),
            properties: vec!["uid".to_string()],
        });

        let diff = diff_documents(&old, &new);
        assert!(!diff.changes.is_empty());
        assert!(!diff.has_breaking_changes());
    }

    #[test]
    fn test_embedding_space_change_is_breaking() {
        let mut old = SchemaDocument::new(None);
        old.embedding_profiles.push(ProfileSpec {
            name: "default".to_string(),
            model: "a".to_string(),
            dimensions: 768,
            modality: "prose".to_string(),
        });
        let mut new = SchemaDocument::new(None);
        new.embedding_profiles.push(ProfileSpec {
            name: "default".to_string(),
            model: "b".to_string(),
            dimensions: 1024,
            modality: "prose".to_string(),
        });

        let diff = diff_documents(&old, &new);
        assert_eq!(diff.breaking_count, 1);
    }

    #[test]
    fn test_constraint_removal_is_breaking() {
        let mut old = SchemaDocument::new(None);
        old.constraints.push(UniqueConstraint {
            label: "Person".to_string(),
            properties: vec!["uid".to_string()],
        });
        let new = SchemaDocument::new(None);
        let diff = diff_documents(&old, &new);
        assert!(diff.has_breaking_changes());
    }
}
