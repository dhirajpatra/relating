use relating_core::SourceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedRecord {
    pub source_id: SourceId,
    pub lineage_ref: String,
    pub fields: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceProfile {
    pub total_records: usize,
    pub field_names: Vec<String>,
    pub sample_records: Vec<HashMap<String, serde_json::Value>>,
}
