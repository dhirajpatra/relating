use regex::Regex;
use std::sync::OnceLock;

fn periodic_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)USING\s+PERIODIC\s+COMMIT\s*(\d+)?\s*").expect("valid regex")
    })
}

fn legacy_constraint_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)CREATE\s+CONSTRAINT\s+(?:IF\s+NOT\s+EXISTS\s+)?ON\s*\(\s*\w+\s*:\s*(`?\w+`?)\s*\)\s*ASSERT\s+\w+\s*\.\s*(`?\w+`?)\s+IS\s+UNIQUE",
        )
        .expect("valid regex")
    })
}

/// Rewrite `USING PERIODIC COMMIT` into an explicit UNWIND batch pattern.
/// Returns explanatory text with the corrected candidate.
pub fn rewrite_periodic_commit(query: &str) -> String {
    let stripped = periodic_re().replace(query, "").trim().to_string();
    format!(
        "Remove USING PERIODIC COMMIT and batch explicitly, e.g.:\nUNWIND $batch AS row\n{}",
        stripped
    )
}

/// Rewrite legacy `ASSERT ... IS UNIQUE` constraints to Neo4j 5 `REQUIRE`.
/// Returns None when the pattern does not match.
pub fn rewrite_legacy_constraint(query: &str) -> Option<String> {
    let caps = legacy_constraint_re().captures(query)?;
    let label = caps.get(1)?.as_str();
    let prop = caps.get(2)?.as_str();
    Some(format!(
        "CREATE CONSTRAINT IF NOT EXISTS FOR (n:{}) REQUIRE n.{} IS UNIQUE",
        label.trim_matches('`'),
        prop.trim_matches('`')
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_periodic_rewrite_suggests_unwind() {
        let out = rewrite_periodic_commit(
            "USING PERIODIC COMMIT 500 LOAD CSV FROM 'file:///x.csv' AS row CREATE (n:N)",
        );
        assert!(out.contains("UNWIND $batch AS row"));
        let candidate = out.split("e.g.:\n").nth(1).unwrap_or("");
        assert!(!candidate.contains("PERIODIC"));
        assert!(candidate.contains("LOAD CSV"));
    }

    #[test]
    fn test_legacy_constraint_rewrite() {
        let out =
            rewrite_legacy_constraint("CREATE CONSTRAINT ON (n:Person) ASSERT n.uid IS UNIQUE")
                .unwrap();
        assert_eq!(
            out,
            "CREATE CONSTRAINT IF NOT EXISTS FOR (n:Person) REQUIRE n.uid IS UNIQUE"
        );
    }

    #[test]
    fn test_legacy_rewrite_rejects_non_match() {
        assert!(rewrite_legacy_constraint("MATCH (n) RETURN n").is_none());
    }
}
