use crate::error::ShadowError;
use relating_graph::{GraphBackend, GraphQueryResult};
use relating_migrate::{bolt_to_json, json_to_bolt, Neo4jBackend};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowQuery {
    pub name: String,
    pub cypher: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub columns: Vec<String>,
    pub row_count: usize,
    /// Column name -> sorted distinct JSON type names observed.
    pub column_types: BTreeMap<String, Vec<String>>,
    pub content_hash: String,
    pub latency_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rows: Option<Vec<serde_json::Value>>,
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn hash_rows(rows: &[serde_json::Value]) -> String {
    // Order-insensitive: sort canonical row encodings before hashing.
    let mut encodings: Vec<String> = rows
        .iter()
        .map(|r| serde_json::to_string(r).unwrap_or_default())
        .collect();
    encodings.sort();
    let mut hasher = Sha256::new();
    for encoding in encodings {
        hasher.update(encoding.as_bytes());
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn profile_rows(
    columns: &[String],
    rows: &[HashMap<String, serde_json::Value>],
) -> BTreeMap<String, Vec<String>> {
    let mut profile: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for column in columns {
        profile.entry(column.clone()).or_default();
    }
    for row in rows {
        for (column, value) in row {
            if let Some(set) = profile.get_mut(column) {
                set.insert(json_type_name(value).to_string());
            }
        }
    }
    profile
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// Only read-shaped queries may be replayed: anything that could mutate is
/// refused before touching either backend.
pub fn is_read_only(cypher: &str) -> bool {
    let upper = cypher.to_uppercase();
    let mut stripped = String::with_capacity(upper.len());
    let mut quote: Option<char> = None;
    for c in upper.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' || c == '`' {
            quote = Some(c);
            continue;
        }
        stripped.push(c);
    }
    const WRITE_KEYWORDS: &[&str] = &[
        "CREATE", "MERGE", "DELETE", "DETACH", "SET", "REMOVE", "FOREACH",
    ];
    !WRITE_KEYWORDS.iter().any(|kw| {
        stripped
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| word == *kw)
    })
}

pub fn mask_params(params: &HashMap<String, serde_json::Value>) -> HashMap<String, String> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), format!("<{} redacted>", json_type_name(v))))
        .collect()
}

fn rows_from_graph(result: &GraphQueryResult) -> Vec<HashMap<String, serde_json::Value>> {
    result.rows.clone()
}

pub async fn replay_source(
    graph: &dyn GraphBackend,
    store_path: &Path,
    query: &ShadowQuery,
    limit: u32,
    include_samples: bool,
) -> Result<QueryResult, ShadowError> {
    if !is_read_only(&query.cypher) {
        return Err(ShadowError::Invalid(format!(
            "Refusing to replay non-read query '{}': shadow testing is read-only",
            query.name
        )));
    }
    let start = std::time::Instant::now();
    match graph.execute_read(store_path, &query.cypher, query.params.clone(), Some(limit)) {
        Ok(result) => {
            let rows = rows_from_graph(&result);
            let row_values: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::Value::Object(
                        row.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    )
                })
                .collect();
            Ok(QueryResult {
                success: true,
                error: None,
                columns: result.columns.clone(),
                row_count: rows.len(),
                column_types: profile_rows(&result.columns, &rows),
                content_hash: hash_rows(&row_values),
                latency_ms: start.elapsed().as_secs_f64() * 1000.0,
                sample_rows: if include_samples {
                    Some(row_values.into_iter().take(5).collect())
                } else {
                    None
                },
            })
        }
        Err(e) => Ok(QueryResult {
            success: false,
            error: Some(e.to_string()),
            columns: Vec::new(),
            row_count: 0,
            column_types: BTreeMap::new(),
            content_hash: String::new(),
            latency_ms: start.elapsed().as_secs_f64() * 1000.0,
            sample_rows: None,
        }),
    }
}

pub async fn replay_target(
    backend: &dyn Neo4jBackend,
    query: &ShadowQuery,
    limit: u32,
) -> Result<QueryResult, ShadowError> {
    if !is_read_only(&query.cypher) {
        return Err(ShadowError::Invalid(format!(
            "Refusing to replay non-read query '{}': shadow testing is read-only",
            query.name
        )));
    }
    // Bound the target read the same way the source is bounded.
    let mut cypher = query.cypher.trim().to_string();
    if !cypher.to_lowercase().contains("limit ") {
        cypher.push_str(&format!(" LIMIT {}", limit));
    }
    let mut params = BTreeMap::new();
    for (k, v) in &query.params {
        params.insert(k.clone(), json_to_bolt(v));
    }

    let start = std::time::Instant::now();
    match backend.run_read(&cypher, params).await {
        Ok(rows) => {
            // Bolt rows are positional; synthesize c0..cN columns.
            let width = rows.first().map(|r| r.len()).unwrap_or(0);
            let columns: Vec<String> = (0..width).map(|i| format!("c{}", i)).collect();
            let row_values: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let obj: serde_json::Map<String, serde_json::Value> = row
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (format!("c{}", i), bolt_to_json(v)))
                        .collect();
                    serde_json::Value::Object(obj)
                })
                .collect();
            let row_maps: Vec<HashMap<String, serde_json::Value>> = row_values
                .iter()
                .map(|v| {
                    v.as_object()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default()
                })
                .collect();
            Ok(QueryResult {
                success: true,
                error: None,
                columns: columns.clone(),
                row_count: rows.len(),
                column_types: profile_rows(&columns, &row_maps),
                content_hash: hash_rows(&row_values),
                latency_ms: start.elapsed().as_secs_f64() * 1000.0,
                sample_rows: None, // target samples stay off by design
            })
        }
        Err(e) => Ok(QueryResult {
            success: false,
            error: Some(e.to_string()),
            columns: Vec::new(),
            row_count: 0,
            column_types: BTreeMap::new(),
            content_hash: String::new(),
            latency_ms: start.elapsed().as_secs_f64() * 1000.0,
            sample_rows: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_only_gate() {
        assert!(is_read_only("MATCH (n) RETURN n LIMIT 5"));
        assert!(is_read_only(
            "MATCH (n) WHERE n.note = 'please CREATE me' RETURN n"
        ));
        assert!(!is_read_only("CREATE (n:Person)"));
        assert!(!is_read_only("MATCH (n) SET n.x = 1"));
        assert!(!is_read_only("MERGE (n:X) ON CREATE SET n.y = 1"));
    }

    #[test]
    fn test_hash_is_order_insensitive() {
        let a = vec![serde_json::json!({"x": 1}), serde_json::json!({"x": 2})];
        let b = vec![serde_json::json!({"x": 2}), serde_json::json!({"x": 1})];
        assert_eq!(hash_rows(&a), hash_rows(&b));
        let c = vec![serde_json::json!({"x": 3})];
        assert_ne!(hash_rows(&a), hash_rows(&c));
    }

    #[test]
    fn test_mask_params_hides_values() {
        let params = HashMap::from([("ssn".to_string(), serde_json::json!("123"))]);
        let masked = mask_params(&params);
        assert_eq!(masked["ssn"], "<string redacted>");
    }
}
