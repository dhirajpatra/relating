use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use relating_core::SourceId;
use std::collections::HashMap;

pub fn parse_text_content(
    source_id: SourceId,
    _filename: &str,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let text = String::from_utf8_lossy(content);
    let blocks: Vec<&str> = text
        .split("\n\n")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let mut records = Vec::new();
    for (idx, block) in blocks.iter().enumerate() {
        let mut fields = HashMap::new();
        fields.insert(
            "text".to_string(),
            serde_json::Value::String(block.to_string()),
        );
        fields.insert(
            "length".to_string(),
            serde_json::Value::Number(block.len().into()),
        );
        fields.insert(
            "block_index".to_string(),
            serde_json::Value::Number((idx + 1).into()),
        );

        records.push(NormalizedRecord {
            source_id: source_id.clone(),
            lineage_ref: format!("block_{}", idx + 1),
            fields,
        });
    }

    let sample_records = records
        .iter()
        .take(5)
        .map(|r| r.fields.clone())
        .collect::<Vec<_>>();

    let profile = SourceProfile {
        total_records: records.len(),
        field_names: vec![
            "text".to_string(),
            "length".to_string(),
            "block_index".to_string(),
        ],
        sample_records,
    };

    Ok((records, profile))
}
