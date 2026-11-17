use crate::document::SchemaDocument;
use crate::error::SchemaError;

fn pretty(value: &impl serde::Serialize) -> Result<String, SchemaError> {
    serde_json::to_string_pretty(value)
        .map(|mut s| {
            s.push('\n');
            s
        })
        .map_err(|e| SchemaError::Internal(format!("Failed to serialize schema file: {}", e)))
}

fn constraints_file(document: &SchemaDocument) -> String {
    let mut out = String::from("-- Generated unique constraints (Neo4j 5 syntax).\n");
    for constraint in &document.constraints {
        if constraint.properties.len() == 1 {
            out.push_str(&relating_migrate::constraint_ddl(
                &constraint.label,
                &constraint.properties[0],
                None,
            ));
            out.push_str(";\n");
        }
    }
    out
}

fn indexes_file(document: &SchemaDocument) -> String {
    let mut out = String::from("-- Generated indexes (Neo4j 5 syntax).\n");
    for index in &document.indexes {
        if let Some(stmt) = relating_migrate::index_ddl(&index.label, &index.properties) {
            out.push_str(&stmt);
            out.push_str(";\n");
        }
    }
    out
}

fn contracts_file(document: &SchemaDocument) -> Result<String, SchemaError> {
    let contracts: Vec<serde_json::Value> = document
        .ontology
        .entities
        .iter()
        .map(|entity| {
            serde_json::json!({
                "label": entity.label,
                "required_properties": entity.properties.iter().map(|p| serde_json::json!({
                    "name": p.name,
                    "data_type": p.data_type,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let unique: Vec<serde_json::Value> = document
        .constraints
        .iter()
        .map(|c| serde_json::json!({"label": c.label, "properties": c.properties}))
        .collect();
    pretty(&serde_json::json!({
        "format_version": crate::document::SCHEMA_FORMAT_VERSION,
        "entity_contracts": contracts,
        "unique_constraints": unique,
    }))
}

/// The five committable as-code files for a schema document.
pub fn export_files(document: &SchemaDocument) -> Result<Vec<(String, String)>, SchemaError> {
    Ok(vec![
        ("graph/schema.json".to_string(), pretty(document)?),
        (
            "graph/ontology.json".to_string(),
            pretty(&document.ontology)?,
        ),
        (
            "graph/constraints.cypher".to_string(),
            constraints_file(document),
        ),
        ("graph/indexes.cypher".to_string(), indexes_file(document)),
        (
            "tests/graph_contracts.json".to_string(),
            contracts_file(document)?,
        ),
    ])
}
