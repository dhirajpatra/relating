use crate::error::IngestError;
use crate::record::NormalizedRecord;
use relating_core::{ExtractedNode, ExtractedRelationship, OntologySchema, SourceId};
use std::collections::HashMap;

pub fn extract_entities_and_relationships(
    source_id: SourceId,
    filename: &str,
    records: &[NormalizedRecord],
    ontology: &OntologySchema,
) -> Result<(Vec<ExtractedNode>, Vec<ExtractedRelationship>), IngestError> {
    if records.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut nodes = Vec::new();
    let mut relationships = Vec::new();

    // Match ontology entity for this source (by filename stem or best property match)
    let matched_entity = ontology
        .entities
        .iter()
        .find(|e| {
            filename
                .to_lowercase()
                .contains(&e.label.to_lowercase())
                || filename
                    .to_lowercase()
                    .contains(&e.display_name.to_lowercase())
        })
        .or_else(|| ontology.entities.first());

    let entity_def = match matched_entity {
        Some(e) => e,
        None => {
            return Err(IngestError::Extraction(
                "No entity definition in ontology matches source data".to_string(),
            ));
        }
    };

    let pk_prop = entity_def
        .properties
        .iter()
        .find(|p| p.is_primary_key)
        .map(|p| p.name.as_str())
        .or_else(|| {
            records
                .first()
                .and_then(|r| r.fields.keys().find(|k| k.eq_ignore_ascii_case("id")).map(|s| s.as_str()))
        });

    for record in records {
        // Compute node ID
        let raw_id = pk_prop
            .and_then(|pk| record.fields.get(pk))
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| format!("{}_{}", source_id, record.lineage_ref));

        let node_id = format!("{}:{}", entity_def.label, raw_id);

        let mut node_props = HashMap::new();
        // Copy record fields
        for (k, v) in &record.fields {
            node_props.insert(k.clone(), v.clone());
        }

        // Add mandatory provenance properties
        node_props.insert(
            "_source_id".to_string(),
            serde_json::Value::String(source_id.to_string()),
        );
        node_props.insert(
            "_source_file".to_string(),
            serde_json::Value::String(filename.to_string()),
        );
        node_props.insert(
            "_lineage_ref".to_string(),
            serde_json::Value::String(record.lineage_ref.clone()),
        );
        node_props.insert(
            "name".to_string(),
            record
                .fields
                .get("name")
                .or_else(|| record.fields.get("title"))
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String(raw_id.clone())),
        );

        nodes.push(ExtractedNode {
            id: node_id.clone(),
            labels: vec![entity_def.label.clone()],
            properties: node_props,
            source_id: source_id.clone(),
            record_ref: record.lineage_ref.clone(),
            confidence: 1.0,
        });

        // Check for defined relationships in ontology
        for rel_def in &ontology.relationships {
            if rel_def.source_label == entity_def.label {
                // Look for target key (e.g. company_id for Company target)
                let target_key = format!("{}_id", rel_def.target_label.to_lowercase());
                if let Some(target_val) = record.fields.get(&target_key) {
                    let target_raw_id = match target_val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => continue,
                    };

                    let target_node_id = format!("{}:{}", rel_def.target_label, target_raw_id);
                    let mut rel_props = HashMap::new();
                    rel_props.insert(
                        "_source_id".to_string(),
                        serde_json::Value::String(source_id.to_string()),
                    );

                    relationships.push(ExtractedRelationship {
                        rel_type: rel_def.rel_type.clone(),
                        source_node_id: node_id.clone(),
                        target_node_id,
                        properties: rel_props,
                        source_id: source_id.clone(),
                        confidence: 1.0,
                    });
                }
            }
        }
    }

    Ok((nodes, relationships))
}
