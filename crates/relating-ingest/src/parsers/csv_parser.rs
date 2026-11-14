use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use relating_core::SourceId;
use std::collections::HashMap;

pub fn parse_csv_content(
    source_id: SourceId,
    filename: &str,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content);

    let headers = reader
        .headers()
        .map_err(|e| IngestError::ParseError {
            filename: filename.to_string(),
            message: format!("Failed to read CSV headers: {}", e),
        })?
        .iter()
        .map(|h| h.trim().to_string())
        .collect::<Vec<String>>();

    let mut records = Vec::new();
    let mut row_idx = 1;

    for result in reader.records() {
        let record = result.map_err(|e| IngestError::ParseError {
            filename: filename.to_string(),
            message: format!("Failed to parse CSV row {}: {}", row_idx, e),
        })?;

        let mut fields = HashMap::new();
        for (i, val) in record.iter().enumerate() {
            if let Some(col_name) = headers.get(i) {
                let json_val = parse_inferred_value(val);
                fields.insert(col_name.clone(), json_val);
            }
        }

        records.push(NormalizedRecord {
            source_id: source_id.clone(),
            lineage_ref: format!("row_{}", row_idx),
            fields,
        });

        row_idx += 1;
    }

    let sample_records = records
        .iter()
        .take(5)
        .map(|r| r.fields.clone())
        .collect::<Vec<_>>();

    let profile = SourceProfile {
        total_records: records.len(),
        field_names: headers,
        sample_records,
    };

    Ok((records, profile))
}

fn parse_inferred_value(s: &str) -> serde_json::Value {
    let s = s.trim();
    if s.is_empty() {
        return serde_json::Value::Null;
    }

    if let Ok(b) = s.parse::<bool>() {
        return serde_json::Value::Bool(b);
    }

    if let Ok(i) = s.parse::<i64>() {
        return serde_json::Value::Number(i.into());
    }

    if let Ok(f) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(num);
        }
    }

    serde_json::Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_csv_basic() {
        let csv_data = "name,age,active,balance\nAlice,30,true,1250.50\nBob,42,false,0.0";
        let (records, profile) =
            parse_csv_content(SourceId::new(), "users.csv", csv_data.as_bytes()).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(profile.total_records, 2);
        assert_eq!(profile.field_names, vec!["name", "age", "active", "balance"]);

        assert_eq!(records[0].fields["name"], "Alice");
        assert_eq!(records[0].fields["age"], 30);
        assert_eq!(records[0].fields["active"], true);
        assert_eq!(records[0].fields["balance"], 1250.5);
        assert_eq!(records[0].lineage_ref, "row_1");

        assert_eq!(records[1].fields["name"], "Bob");
        assert_eq!(records[1].lineage_ref, "row_2");
    }
}
