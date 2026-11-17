use crate::document::{ProfileSpec, RetrievalSpec, SchemaDocument, UniqueConstraint};
use relating_core::{OntologySchema, ProjectId};
use relating_graph::GraphBackend;
use relating_metadata::{
    get_approved_ontology, get_latest_ontology, get_retrieval_config, list_embedding_profiles,
    DbPool,
};
use std::path::Path;

/// Build the live schema document from current project state: approved (or
/// latest) ontology, uid merge-key conventions, embedding profiles, and the
/// stored retrieval configuration.
pub async fn build_live_document(
    pool: &DbPool,
    graph: &dyn GraphBackend,
    project_id: &ProjectId,
    graph_path: &Path,
    project_name: Option<String>,
) -> Result<SchemaDocument, crate::error::SchemaError> {
    let schema = graph.get_schema(graph_path).unwrap_or_default();
    let ontology: OntologySchema = get_approved_ontology(pool, project_id)
        .await?
        .or(get_latest_ontology(pool, project_id).await?)
        .map(|o| o.schema)
        .unwrap_or_default();

    let mut constraints: Vec<UniqueConstraint> = schema
        .labels
        .iter()
        .map(|label| UniqueConstraint {
            label: label.clone(),
            properties: vec!["uid".to_string()],
        })
        .collect();
    for entity in &ontology.entities {
        for prop in &entity.properties {
            if prop.is_primary_key && prop.name != "uid" {
                constraints.push(UniqueConstraint {
                    label: entity.label.clone(),
                    properties: vec![prop.name.clone()],
                });
            }
        }
    }

    let profiles = list_embedding_profiles(pool, project_id).await?;
    let embedding_profiles = profiles
        .into_iter()
        .map(|p| ProfileSpec {
            name: p.name,
            model: p.model,
            dimensions: p.dimensions,
            modality: p.modality,
        })
        .collect();

    let stored = get_retrieval_config(pool, project_id).await?;
    let retrieval = match stored {
        Some(config) => RetrievalSpec {
            default_strategy: config.default_strategy,
            default_top_k: config.default_top_k,
        },
        None => RetrievalSpec::default(),
    };

    Ok(SchemaDocument {
        format_version: crate::document::SCHEMA_FORMAT_VERSION.to_string(),
        project_name,
        ontology,
        constraints,
        indexes: Vec::new(),
        embedding_profiles,
        retrieval,
    })
}
