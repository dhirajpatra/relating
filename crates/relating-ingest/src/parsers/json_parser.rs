use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use relating_core::SourceId;
use std::collections::HashMap;

pub fn parse_json_content(
    source_id: SourceId,
    filename: &str,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let parsed: serde_json::Value =
        serde_json::from_slice(content).map_err(|e| IngestError::ParseError {
            filename: filename.to_string(),
            message: format!("Invalid JSON: {}", e),
        })?;

    let mut records = Vec::new();
    let mut field_names_set = std::collections::BTreeSet::new();

    let items = match parsed {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(obj) => {
            // Check if there is an array property like "items", "data", "records"
            let array_val = obj
                .values()
                .find(|v| v.is_array())
                .cloned();

            if let Some(serde_json::Value::Array(arr)) = array_val {
                arr
            } else {
                vec![serde_json::Value::Object(obj)]
            }
        }
        _ => {
            return Err(IngestError::ParseError {
                filename: filename.to_string(),
                message: "JSON content must be an array of objects or an object".to_string(),
            })
        }
    };

    for (idx, item) in items.into_iter().enumerate() {
        if let serde_json::Value::Object(map) = item {
            let mut fields = HashMap::new();
            for (k, v) in map {
                field_names_set.insert(k.clone());
                fields.insert(k, v);
            }

            records.push(NormalizedRecord {
                source_id: source_id.clone(),
                lineage_ref: format!("item_{}", idx + 1),
                fields,
            });
        }
    }

    let sample_records = records
        .iter()
        .take(5)
        .map(|r| r.fields.clone())
        .collect::<Vec<_>>();

    let profile = SourceProfile {
        total_records: records.len(),
        field_names: field_names_set.into_iter().collect(),
        sample_records,
    };

    Ok((records, profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_array() {
        let json_data = r#"[
            {"id": "comp_1", "name": "Acme Inc", "industry": "Tech"},
            {"id": "comp_2", "name": "Global Logistics", "industry": "Supply Chain"}
        ]"#;

        let (records, profile) =
            parse_json_content(SourceId::new(), "companies.json", json_data.as_bytes()).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(profile.total_records, 2);
        assert_eq!(records[0].fields["name"], "Acme Inc");
        assert_eq!(records[0].lineage_ref, "item_1");
        assert_eq!(records[1].fields["industry"], "Supply Chain");
    }
}
