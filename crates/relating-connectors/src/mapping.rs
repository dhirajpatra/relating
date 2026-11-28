//! Row-to-record mapping shared by SQL and REST connectors.
//!
//! Source rows are string-keyed JSON maps; this module resolves the stable
//! `uid` merge key, applies renames/drops, and stamps lineage refs that stay
//! identical across syncs so delta pushes update in place.

use relating_core::SourceId;
use relating_ingest::NormalizedRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

pub type RowMap = HashMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldMapping {
    /// Source field holding the stable uid. Defaults to `uid`, then `id`.
    /// Rows without a usable uid are skipped, never merged unpredictably.
    #[serde(default)]
    pub uid_field: Option<String>,
    /// Rename map: source field -> record field.
    #[serde(default)]
    pub renames: HashMap<String, String>,
    /// Source fields to drop before extraction.
    #[serde(default)]
    pub drop: Vec<String>,
}

impl FieldMapping {
    fn uid_candidates(&self) -> Vec<&str> {
        let mut out = Vec::with_capacity(3);
        if let Some(custom) = self.uid_field.as_deref() {
            if !custom.is_empty() && custom != "uid" {
                out.push(custom);
            }
        }
        out.push("uid");
        out.push("id");
        out
    }
}

fn uid_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Map source rows to normalized records. Returns the records plus the
/// count of rows skipped for missing uid keys.
pub fn map_rows_to_records(
    source_id: SourceId,
    rows: &[RowMap],
    mapping: &FieldMapping,
    lineage_prefix: &str,
) -> (Vec<NormalizedRecord>, u64) {
    let mut out = Vec::with_capacity(rows.len());
    let mut skipped = 0u64;
    let now = chrono::Utc::now().to_rfc3339();
    for row in rows {
        let uid = mapping
            .uid_candidates()
            .iter()
            .filter_map(|field| row.get(*field).and_then(uid_string))
            .next();
        let Some(uid) = uid else {
            skipped += 1;
            continue;
        };
        let mut fields = HashMap::with_capacity(row.len() + 1);
        for (key, value) in row {
            if mapping.drop.iter().any(|d| d == key) {
                continue;
            }
            let target = mapping
                .renames
                .get(key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            fields.insert(target, value.clone());
        }
        fields.insert(
            "_ingested_at".to_string(),
            serde_json::Value::String(now.clone()),
        );
        out.push(NormalizedRecord {
            source_id: source_id.clone(),
            lineage_ref: format!("{}:{}", lineage_prefix, uid),
            fields,
        });
    }
    (out, skipped)
}

/// Deterministic content hash over source rows: keys sorted, values in row
/// order. Syncs compare it against the stored hash to skip unchanged
/// snapshots without touching the graph.
pub fn rows_content_hash(rows: &[RowMap]) -> String {
    let normalized: Vec<BTreeMap<&str, &serde_json::Value>> = rows
        .iter()
        .map(|row| row.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_string(&normalized)
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, serde_json::Value)]) -> RowMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_uid_fallback_and_skip_without_key() {
        let rows = vec![
            row(&[("uid", serde_json::json!("u-1"))]),
            row(&[("id", serde_json::json!(7))]),
            row(&[("name", serde_json::json!("keyless"))]),
        ];
        let (records, skipped) = map_rows_to_records(
            SourceId::new(),
            &rows,
            &FieldMapping::default(),
            "sql:people",
        );
        assert_eq!(records.len(), 2);
        assert_eq!(skipped, 1);
        assert_eq!(records[0].lineage_ref, "sql:people:u-1");
        assert_eq!(records[1].lineage_ref, "sql:people:7");
        assert!(records[0].fields.contains_key("_ingested_at"));
    }

    #[test]
    fn test_custom_uid_field_renames_and_drops() {
        let rows = vec![row(&[
            ("email", serde_json::json!("a@x.io")),
            ("full_name", serde_json::json!("Ada")),
            ("internal", serde_json::json!(true)),
        ])];
        let mapping = FieldMapping {
            uid_field: Some("email".to_string()),
            renames: HashMap::from([("full_name".to_string(), "name".to_string())]),
            drop: vec!["internal".to_string()],
        };
        let (records, skipped) =
            map_rows_to_records(SourceId::new(), &rows, &mapping, "rest:users");
        assert_eq!(skipped, 0);
        assert_eq!(records[0].lineage_ref, "rest:users:a@x.io");
        assert_eq!(
            records[0].fields.get("name"),
            Some(&serde_json::json!("Ada"))
        );
        assert!(!records[0].fields.contains_key("internal"));
    }

    #[test]
    fn test_rows_content_hash_is_order_stable_for_keys() {
        let a = vec![row(&[
            ("b", serde_json::json!(1)),
            ("a", serde_json::json!(2)),
        ])];
        let b = vec![row(&[
            ("a", serde_json::json!(2)),
            ("b", serde_json::json!(1)),
        ])];
        assert_eq!(rows_content_hash(&a), rows_content_hash(&b));
        let c = vec![row(&[("a", serde_json::json!(3))])];
        assert_ne!(rows_content_hash(&a), rows_content_hash(&c));
    }
}
