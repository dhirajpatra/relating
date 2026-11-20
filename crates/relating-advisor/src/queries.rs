use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryFinding {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    pub suggestion: String,
}

fn finding(
    rule_id: &str,
    severity: Severity,
    message: impl Into<String>,
    suggestion: impl Into<String>,
) -> QueryFinding {
    QueryFinding {
        rule_id: rule_id.to_string(),
        severity,
        message: message.into(),
        suggestion: suggestion.into(),
    }
}

fn strip_strings(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut quote: Option<char> = None;
    for c in query.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        out.push(c);
    }
    out
}

fn aggregate_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(count|sum|avg|min|max|collect)\s*\(").expect("valid regex")
    })
}

fn cartesian_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bMATCH\b[^;]*\)\s*,\s*\(").expect("valid regex"))
}

fn unbounded_varlen_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\s*\w*\s*\*\s*\]").expect("valid regex"))
}

fn bounded_varlen_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\s*\w*\s*\*\s*\d").expect("valid regex"))
}

fn string_predicate_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(CONTAINS|STARTS\s+WITH|ENDS\s+WITH)\b").expect("valid regex")
    })
}

fn equality_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\(\s*\w+\s*:\s*(`?\w+`?)\s*\{([^}]*)\}\)").expect("valid regex"))
}

fn where_equality_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)WHERE\s+(\w+)\.(\w+)\s*=\s*(\$[\w]+|'[^']*'|\d+)").expect("valid regex")
    })
}

fn shortest_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(ALL\s*)?SHORTEST\s*PATH\s*\(").expect("valid regex"))
}

fn limit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bLIMIT\s+(\d+|\$[\w]+)").expect("valid regex"))
}

/// Static performance review of one Cypher statement. Heuristic and
/// explainable: every finding says what was seen and what to do.
pub fn analyze_query(name: &str, cypher: &str) -> Vec<QueryFinding> {
    let _ = name;
    let mut findings = Vec::new();
    let scrubbed = strip_strings(cypher);
    let upper = scrubbed.to_uppercase();

    // ADV-UNBOUNDED: result-producing scans without LIMIT or aggregation.
    let has_limit = limit_re().is_match(&scrubbed);
    let has_aggregate = aggregate_re().is_match(&scrubbed);
    let returns_entities = upper.contains("RETURN");
    if returns_entities && !has_limit && !has_aggregate {
        findings.push(finding(
            "ADV-UNBOUNDED",
            Severity::Warning,
            "Result scan without LIMIT or aggregation can stream the whole graph",
            "Add LIMIT (or aggregate) and paginate with SKIP for large results",
        ));
    }

    // ADV-CARTESIAN: comma-separated patterns.
    if cartesian_re().is_match(&scrubbed) {
        findings.push(finding(
            "ADV-CARTESIAN",
            Severity::Warning,
            "Comma-separated MATCH patterns form a cartesian product",
            "Connect the patterns with a relationship or filter early with WHERE",
        ));
    }

    // ADV-EXPAND: unbounded variable-length expansion.
    if unbounded_varlen_re().is_match(&scrubbed) {
        findings.push(finding(
            "ADV-EXPAND",
            Severity::Warning,
            "Unbounded [*] traversal can explode on dense graphs",
            "Bound the depth (e.g. [*1..3]) and filter relationship types",
        ));
    } else if bounded_varlen_re().is_match(&scrubbed) {
        findings.push(finding(
            "ADV-EXPAND",
            Severity::Info,
            "Bounded variable-length traversal: verify depth against dense areas",
            "Profile with EXPLAIN on production-like data volumes",
        ));
    }

    // ADV-SCAN: substring predicates defeat b-tree lookups.
    if string_predicate_re().is_match(&scrubbed) {
        findings.push(finding(
            "ADV-SCAN",
            Severity::Warning,
            "CONTAINS/STARTS WITH/ENDS WITH scans property values instead of seeking",
            "Use exact matches with an index, or a full-text index for substring search",
        ));
    }

    // ADV-SHORTEST: path search cost.
    if shortest_re().is_match(&scrubbed) {
        findings.push(finding(
            "ADV-SHORTEST",
            Severity::Info,
            "shortestPath cost grows with graph density; bound the search space",
            "Constrain start/end labels and relationship types, then measure",
        ));
    }

    // ADV-INDEX: equality predicates are index candidates.
    let mut candidates: Vec<String> = Vec::new();
    for caps in equality_re().captures_iter(&scrubbed) {
        let label = caps.get(1).map(|m| m.as_str()).unwrap_or("?");
        let props = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        for prop in props.split(',') {
            let key = prop
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('`');
            if !key.is_empty() {
                candidates.push(format!("{}({})", label.trim_matches('`'), key));
            }
        }
    }
    for caps in where_equality_re().captures_iter(&scrubbed) {
        candidates.push(format!("{}.{}", &caps[1], &caps[2]));
    }
    candidates.sort();
    candidates.dedup();
    for candidate in candidates {
        findings.push(finding(
            "ADV-INDEX",
            Severity::Info,
            format!("Equality predicate on '{}' is an index candidate", candidate),
            "Confirm with EXPLAIN that the planner seeks (candidate only, not proof of a missing index)",
        ));
    }

    // ADV-PARAMS: literals defeat plan caching.
    let literal_re = Regex::new(r"(?i)(=|<|>)\s*('[^']*'|\b\d+\b)").expect("valid regex");
    if literal_re.is_match(&scrubbed) && !scrubbed.contains('$') {
        findings.push(finding(
            "ADV-PARAMS",
            Severity::Info,
            "Literals instead of parameters defeat server-side plan caching",
            "Bind values as $parameters",
        ));
    }

    if findings.is_empty() {
        findings.push(finding(
            "ADV-OK",
            Severity::Info,
            "No static performance risks detected",
            "Validate on production-like data before relying on it",
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(cypher: &str) -> Vec<String> {
        analyze_query("q", cypher)
            .into_iter()
            .map(|f| f.rule_id)
            .collect()
    }

    #[test]
    fn test_unbounded_and_cartesian() {
        let found = rules("MATCH (a), (b) RETURN a, b");
        assert!(found.contains(&"ADV-UNBOUNDED".to_string()));
        assert!(found.contains(&"ADV-CARTESIAN".to_string()));
    }

    #[test]
    fn test_bounded_query_is_quiet() {
        let found = rules("MATCH (p:Person {uid: $uid}) RETURN p LIMIT 1");
        assert!(!found.contains(&"ADV-UNBOUNDED".to_string()));
        assert!(found.iter().any(|r| r == "ADV-INDEX"));
    }

    #[test]
    fn test_varlen_and_scans() {
        assert!(
            rules("MATCH p = (a)-[*]->(b) RETURN p LIMIT 5").contains(&"ADV-EXPAND".to_string())
        );
        assert!(
            rules("MATCH (n) WHERE n.name CONTAINS 'x' RETURN n LIMIT 5")
                .contains(&"ADV-SCAN".to_string())
        );
    }

    #[test]
    fn test_literals_flagged_but_params_quiet() {
        assert!(rules("MATCH (n:Person) WHERE n.age = 36 RETURN n LIMIT 1")
            .contains(&"ADV-PARAMS".to_string()));
        assert!(
            !rules("MATCH (n:Person) WHERE n.age = $age RETURN n LIMIT 1")
                .contains(&"ADV-PARAMS".to_string())
        );
    }
}
