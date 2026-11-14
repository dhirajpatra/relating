use crate::error::IngestError;
use crate::record::SourceProfile;
use relating_ai::{AiProvider, ChatMessage, ChatRequest};
use relating_core::{
    EntityDefinition, OntologySchema, PropertyDefinition, RelationshipDefinition,
};
use std::sync::Arc;
use tracing::{info, warn};

pub async fn propose_ontology(
    ai_provider: Arc<dyn AiProvider>,
    profiles: &[(String, SourceProfile)], // (filename, profile)
    domain_hint: Option<&str>,
) -> Result<OntologySchema, IngestError> {
    // If profiles are empty, return default empty schema
    if profiles.is_empty() {
        return Ok(OntologySchema::default());
    }

    // Build profiling summary for AI
    let mut summary = String::new();
    for (filename, profile) in profiles {
        summary.push_str(&format!(
            "File: {}\nTotal Records: {}\nFields: {}\nSample:\n{}\n\n",
            filename,
            profile.total_records,
            profile.field_names.join(", "),
            serde_json::to_string_pretty(&profile.sample_records).unwrap_or_default()
        ));
    }

    let prompt = format!(
        r#"You are a graph database ontology engineer.
Analyze the following dataset samples and propose a graph ontology schema with Entity node labels and Relationship types.

Domain context: {}

Data Samples:
{}

Output ONLY valid JSON matching this exact structure:
{{
  "entities": [
    {{
      "label": "PascalCaseLabel",
      "display_name": "Readable Name",
      "description": "Entity description",
      "properties": [
        {{ "name": "field_name", "data_type": "string|integer|float|boolean", "is_primary_key": true|false, "description": "Field description" }}
      ]
    }}
  ],
  "relationships": [
    {{
      "rel_type": "SCREAMING_SNAKE_CASE",
      "source_label": "SourceEntityLabel",
      "target_label": "TargetEntityLabel",
      "description": "Relationship description"
    }}
  ]
}}"#,
        domain_hint.unwrap_or("General multi-domain data"),
        summary
    );

    let chat_req = ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are an expert graph architect. Respond only with valid JSON.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: prompt,
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(2000),
    };

    match ai_provider.chat_completion(&chat_req).await {
        Ok(resp) => {
            let content = resp.content.trim();
            // Try extracting JSON if wrapped in markdown codeblock
            let json_str = if content.contains("```json") {
                content
                    .split("```json")
                    .nth(1)
                    .and_then(|s| s.split("```").next())
                    .unwrap_or(content)
                    .trim()
            } else if content.contains("```") {
                content
                    .split("```")
                    .nth(1)
                    .and_then(|s| s.split("```").next())
                    .unwrap_or(content)
                    .trim()
            } else {
                content
            };

            if let Ok(schema) = serde_json::from_str::<OntologySchema>(json_str) {
                if !schema.entities.is_empty() {
                    info!("AI successfully proposed ontology with {} entities", schema.entities.len());
                    return Ok(schema);
                }
            }
            warn!("AI response was not valid ontology JSON. Using rule-based fallback.");
        }
        Err(e) => {
            warn!("AI ontology call failed ({}). Using rule-based fallback.", e);
        }
    }

    // Deterministic rule-based fallback
    Ok(derive_rule_based_ontology(profiles))
}

pub fn derive_rule_based_ontology(profiles: &[(String, SourceProfile)]) -> OntologySchema {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();

    for (filename, profile) in profiles {
        // Derive entity label from filename (e.g. "customers.csv" -> "Customer")
        let stem = filename
            .split('.')
            .next()
            .unwrap_or("Item")
            .trim_end_matches('s');
        let label = to_pascal_case(stem);

        let mut properties = Vec::new();
        for field in &profile.field_names {
            let is_pk = field.eq_ignore_ascii_case("id")
                || field.eq_ignore_ascii_case(&format!("{}_id", stem));

            let data_type = profile
                .sample_records
                .first()
                .and_then(|r| r.get(field))
                .map(|val| match val {
                    serde_json::Value::Number(n) if n.is_i64() => "integer",
                    serde_json::Value::Number(_) => "float",
                    serde_json::Value::Bool(_) => "boolean",
                    _ => "string",
                })
                .unwrap_or("string")
                .to_string();

            properties.push(PropertyDefinition {
                name: field.clone(),
                data_type,
                is_primary_key: is_pk,
                description: Some(format!("Attribute {}", field)),
            });

            // Check if field looks like a foreign key (e.g. company_id -> Company)
            if field.ends_with("_id") && !is_pk {
                let target_stem = field.trim_end_matches("_id");
                let target_label = to_pascal_case(target_stem);
                relationships.push(RelationshipDefinition {
                    rel_type: format!("HAS_{}", target_stem.to_uppercase()),
                    source_label: label.clone(),
                    target_label,
                    description: format!("References {} via {}", target_stem, field),
                });
            }
        }

        entities.push(EntityDefinition {
            label,
            display_name: filename.clone(),
            properties,
            description: format!("Inferred from {}", filename),
        });
    }

    OntologySchema {
        entities,
        relationships,
    }
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' || c.is_whitespace() {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    if result.is_empty() {
        "Entity".to_string()
    } else {
        result
    }
}
