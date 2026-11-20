use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CypherFeatures {
    pub clauses: Vec<String>,
    pub functions: Vec<String>,
    pub procedures: Vec<String>,
    pub has_periodic_commit: bool,
    pub has_load_csv: bool,
    pub has_shortest_path: bool,
    pub has_call_subquery: bool,
    pub has_parameters: bool,
    pub statement_count: usize,
}

/// Split Cypher text into statements on `;`, respecting strings and comments.
pub fn split_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some(c) = chars.next() {
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                current.push(c);
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if in_single {
            current.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            } else if c == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            } else if c == '"' {
                in_double = false;
            }
            continue;
        }
        if in_backtick {
            current.push(c);
            if c == '`' {
                in_backtick = false;
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                current.push(c);
            }
            '"' => {
                in_double = true;
                current.push(c);
            }
            '`' => {
                in_backtick = true;
                current.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                in_line_comment = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block_comment = true;
            }
            ';' => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current = String::new();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn clause_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(MATCH|OPTIONAL MATCH|MERGE|CREATE|DELETE|DETACH DELETE|SET|REMOVE|WITH|UNWIND|RETURN|ORDER BY|SKIP|LIMIT|WHERE|ON CREATE|ON MATCH|FOREACH|CALL|YIELD|UNION)\b",
        )
        .expect("valid clause regex")
    })
}

fn function_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b([a-zA-Z][a-zA-Z0-9_]*)\s*\(").expect("valid function regex")
    })
}

fn procedure_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bCALL\s+([a-zA-Z0-9_.]+)").expect("valid procedure regex"))
}

const CLAUSE_WORDS: &[&str] = &[
    "MATCH", "MERGE", "CREATE", "DELETE", "SET", "WITH", "UNWIND", "RETURN", "WHERE", "CALL",
    "FOREACH", "REMOVE", "ORDER", "SKIP", "LIMIT", "YIELD", "UNION",
];

pub fn extract_features(query: &str) -> CypherFeatures {
    let upper = query.to_uppercase();
    let mut clauses: Vec<String> = clause_re()
        .captures_iter(query)
        .map(|c| c[1].to_uppercase())
        .collect();
    clauses.sort();
    clauses.dedup();

    let mut functions: Vec<String> = function_re()
        .captures_iter(query)
        .map(|c| c[1].to_string())
        .filter(|f| !CLAUSE_WORDS.contains(&f.to_uppercase().as_str()))
        .collect();
    functions.sort();
    functions.dedup();

    let mut procedures: Vec<String> = procedure_re()
        .captures_iter(query)
        .map(|c| c[1].to_string())
        .filter(|p| p.contains('.'))
        .collect();
    procedures.sort();
    procedures.dedup();

    CypherFeatures {
        clauses,
        functions,
        procedures,
        has_periodic_commit: upper.contains("USING PERIODIC COMMIT"),
        has_load_csv: upper.contains("LOAD CSV"),
        has_shortest_path: upper.contains("SHORTESTPATH(") || upper.contains("ALLSHORTESTPATHS("),
        has_call_subquery: Regex::new(r"(?i)\bCALL\s*\{")
            .expect("valid subquery regex")
            .is_match(query),
        has_parameters: query.contains('$'),
        statement_count: split_statements(query).len().max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_respects_strings_and_comments() {
        let stmts = split_statements("CREATE (n {t: 'a;b'}); // trailing;\nMATCH (n) RETURN n;");
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b'"));
        assert!(stmts[1].contains("MATCH"));

        let stmts = split_statements("/* ; */ MATCH (n) RETURN n");
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_extract_features() {
        let features = extract_features(
            "MATCH (p:Person)-[:KNOWS]->(f) WHERE p.age > $age RETURN f.name AS n, count(*) AS c",
        );
        assert!(features.clauses.contains(&"MATCH".to_string()));
        assert!(features.clauses.contains(&"RETURN".to_string()));
        assert!(features
            .functions
            .iter()
            .any(|f| f.eq_ignore_ascii_case("count")));
        assert!(features.has_parameters);
        assert!(!features.has_periodic_commit);
    }

    #[test]
    fn test_detects_procedures_and_risky_constructs() {
        let features = extract_features(
            "USING PERIODIC COMMIT 500 LOAD CSV FROM 'file:///x.csv' AS row CALL apoc.meta.schema() YIELD value RETURN value",
        );
        assert!(features.has_periodic_commit);
        assert!(features.has_load_csv);
        assert!(features.procedures.iter().any(|p| p == "apoc.meta.schema"));
    }
}
