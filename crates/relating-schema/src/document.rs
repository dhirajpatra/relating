use crate::error::SchemaError;
use relating_core::{OntologySchema, RetrievalStrategy};
use serde::{Deserialize, Serialize};

pub const SCHEMA_FORMAT_VERSION: &str = "relating-schema-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniqueConstraint {
    pub label: String,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    pub label: String,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSpec {
    pub name: String,
    pub model: String,
    pub dimensions: usize,
    #[serde(default = "default_modality")]
    pub modality: String,
}

fn default_modality() -> String {
    "prose".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalSpec {
    #[serde(default)]
    pub default_strategy: RetrievalStrategy,
    #[serde(default = "default_top_k")]
    pub default_top_k: usize,
}

fn default_top_k() -> usize {
    5
}

impl Default for RetrievalSpec {
    fn default() -> Self {
        Self {
            default_strategy: RetrievalStrategy::Hybrid,
            default_top_k: 5,
        }
    }
}

/// Version-controlled contract for a project's graph: ontology, constraints,
/// indexes, embedding profiles, and retrieval configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDocument {
    pub format_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default)]
    pub ontology: OntologySchema,
    #[serde(default)]
    pub constraints: Vec<UniqueConstraint>,
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    #[serde(default)]
    pub embedding_profiles: Vec<ProfileSpec>,
    #[serde(default)]
    pub retrieval: RetrievalSpec,
}

impl SchemaDocument {
    pub fn new(project_name: Option<String>) -> Self {
        Self {
            format_version: SCHEMA_FORMAT_VERSION.to_string(),
            project_name,
            ontology: OntologySchema::default(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            embedding_profiles: Vec::new(),
            retrieval: RetrievalSpec::default(),
        }
    }

    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.format_version != SCHEMA_FORMAT_VERSION {
            return Err(SchemaError::UnsupportedFormat(
                self.format_version.clone(),
                SCHEMA_FORMAT_VERSION.to_string(),
            ));
        }
        for entity in &self.ontology.entities {
            if entity.label.trim().is_empty() {
                return Err(SchemaError::Invalid(
                    "Entity label cannot be empty".to_string(),
                ));
            }
            for prop in &entity.properties {
                if prop.name.trim().is_empty() {
                    return Err(SchemaError::Invalid(format!(
                        "Property name cannot be empty on '{}'",
                        entity.label
                    )));
                }
                match prop.data_type.as_str() {
                    "string" | "integer" | "float" | "boolean" => {}
                    other => {
                        return Err(SchemaError::Invalid(format!(
                            "Unsupported data type '{}' on '{}.{}'",
                            other, entity.label, prop.name
                        )))
                    }
                }
            }
        }
        for rel in &self.ontology.relationships {
            if rel.rel_type.trim().is_empty() {
                return Err(SchemaError::Invalid(
                    "Relationship type cannot be empty".to_string(),
                ));
            }
        }
        for constraint in &self.constraints {
            if constraint.label.trim().is_empty() || constraint.properties.is_empty() {
                return Err(SchemaError::Invalid(
                    "Constraints need a label and at least one property".to_string(),
                ));
            }
        }
        for index in &self.indexes {
            if index.label.trim().is_empty() || index.properties.is_empty() {
                return Err(SchemaError::Invalid(
                    "Indexes need a label and at least one property".to_string(),
                ));
            }
        }
        for profile in &self.embedding_profiles {
            if profile.name.trim().is_empty() || profile.model.trim().is_empty() {
                return Err(SchemaError::Invalid(
                    "Embedding profiles need a name and a model".to_string(),
                ));
            }
            if profile.dimensions == 0 {
                return Err(SchemaError::Invalid(format!(
                    "Embedding profile '{}' has zero dimensions",
                    profile.name
                )));
            }
        }
        if self.retrieval.default_top_k == 0 || self.retrieval.default_top_k > 50 {
            return Err(SchemaError::Invalid(
                "retrieval.default_top_k must be between 1 and 50".to_string(),
            ));
        }
        Ok(())
    }

    pub fn content_hash(&self) -> Result<String, SchemaError> {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(self)
            .map_err(|e| SchemaError::Internal(format!("Failed to serialize document: {}", e)))?;
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_document_passes() {
        let doc = SchemaDocument::new(Some("demo".to_string()));
        assert!(doc.validate().is_ok());
        assert!(doc.content_hash().unwrap().len() == 64);
    }

    #[test]
    fn test_rejects_bad_format_version() {
        let mut doc = SchemaDocument::new(None);
        doc.format_version = "v0".to_string();
        assert!(matches!(
            doc.validate(),
            Err(SchemaError::UnsupportedFormat(_, _))
        ));
    }

    #[test]
    fn test_rejects_bad_types_and_ranges() {
        let mut doc = SchemaDocument::new(None);
        doc.ontology.entities.push(relating_core::EntityDefinition {
            label: "Person".to_string(),
            display_name: "Person".to_string(),
            properties: vec![relating_core::PropertyDefinition {
                name: "born".to_string(),
                data_type: "datetime".to_string(),
                is_primary_key: false,
                description: None,
            }],
            description: String::new(),
        });
        assert!(doc.validate().is_err());

        let mut doc = SchemaDocument::new(None);
        doc.retrieval.default_top_k = 0;
        assert!(doc.validate().is_err());

        let mut doc = SchemaDocument::new(None);
        doc.embedding_profiles.push(ProfileSpec {
            name: "default".to_string(),
            model: "m".to_string(),
            dimensions: 0,
            modality: "prose".to_string(),
        });
        assert!(doc.validate().is_err());
    }

    #[test]
    fn test_hash_is_stable() {
        let doc = SchemaDocument::new(Some("demo".to_string()));
        assert_eq!(doc.content_hash().unwrap(), doc.content_hash().unwrap());
    }
}
