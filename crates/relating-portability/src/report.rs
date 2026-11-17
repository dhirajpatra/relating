use crate::cypher::extract_features;
use crate::rules::{check_query, check_signal, PortFinding, PortVerdict};
use crate::scanner::RepoScan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSummary {
    pub files_scanned: usize,
    pub queries_total: usize,
    pub portable: usize,
    pub transform: usize,
    pub needs_rewrite: usize,
    pub needs_review: usize,
    pub signals_total: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestAssertion {
    #[default]
    NoError,
    RowCountEq {
        expected: u64,
    },
    RowCountGte {
        expected: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTest {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub cypher: String,
    pub read_only: bool,
    pub assertion: TestAssertion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortableReport {
    pub summary: PortSummary,
    pub findings: Vec<PortFinding>,
    pub breaking: Vec<PortFinding>,
    pub tests: Vec<GeneratedTest>,
    pub tests_truncated: bool,
}

pub const MAX_GENERATED_TESTS: usize = 200;

const WRITE_KEYWORDS: &[&str] = &[
    "CREATE", "MERGE", "DELETE", "DETACH", "SET", "REMOVE", "FOREACH",
];

fn is_read_only(query: &str) -> bool {
    let upper = query.to_uppercase();
    // Strip string literals to avoid keyword false positives.
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
    !WRITE_KEYWORDS.iter().any(|kw| {
        stripped
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| word == *kw)
    })
}

pub fn build_report(scan: &RepoScan) -> PortableReport {
    let mut findings = Vec::new();

    for candidate in &scan.queries {
        let features = extract_features(&candidate.query);
        findings.extend(check_query(candidate, &features));
    }
    for signal in &scan.signals {
        findings.extend(check_signal(signal));
    }

    let count = |verdict: PortVerdict| findings.iter().filter(|f| f.verdict == verdict).count();
    let summary = PortSummary {
        files_scanned: scan.files_scanned,
        queries_total: scan.queries.len(),
        portable: count(PortVerdict::Portable),
        transform: count(PortVerdict::Transform),
        needs_rewrite: count(PortVerdict::NeedsRewrite),
        needs_review: count(PortVerdict::NeedsReview),
        signals_total: scan.signals.len(),
    };

    let breaking: Vec<PortFinding> = findings
        .iter()
        .filter(|f| f.verdict == PortVerdict::NeedsRewrite)
        .cloned()
        .collect();

    let mut tests = Vec::new();
    for candidate in scan.queries.iter().take(MAX_GENERATED_TESTS) {
        tests.push(GeneratedTest {
            name: format!("{}:{} runs on target", candidate.file, candidate.line),
            file: candidate.file.clone(),
            line: candidate.line,
            cypher: candidate.query.clone(),
            read_only: is_read_only(&candidate.query),
            assertion: TestAssertion::NoError,
        });
    }

    PortableReport {
        summary,
        findings,
        breaking,
        tests_truncated: scan.queries.len() > MAX_GENERATED_TESTS,
        tests,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{QueryCandidate, SourceLanguage};

    fn scan_with(queries: Vec<&str>) -> RepoScan {
        RepoScan {
            root: "/tmp/repo".to_string(),
            files_scanned: 1,
            files_skipped: 0,
            queries: queries
                .into_iter()
                .enumerate()
                .map(|(i, q)| QueryCandidate {
                    file: "app.py".to_string(),
                    line: i as u32 + 1,
                    language: SourceLanguage::Python,
                    query: q.to_string(),
                })
                .collect(),
            signals: Vec::new(),
        }
    }

    #[test]
    fn test_report_counts_and_breaking() {
        let scan = scan_with(vec![
            "MATCH (n) RETURN n",
            "USING PERIODIC COMMIT 100 LOAD CSV FROM 'f' AS r CREATE (n:N)",
        ]);
        let report = build_report(&scan);
        assert_eq!(report.summary.queries_total, 2);
        assert_eq!(report.summary.portable, 1);
        assert_eq!(report.summary.needs_rewrite, 1);
        assert_eq!(report.breaking.len(), 1);
        assert_eq!(report.tests.len(), 2);
        assert!(report.tests[0].read_only);
        assert!(!report.tests[1].read_only);
    }

    #[test]
    fn test_read_only_ignores_string_contents() {
        assert!(is_read_only(
            "MATCH (n) WHERE n.note = 'please CREATE me' RETURN n"
        ));
        assert!(!is_read_only("MATCH (n) SET n.x = 1"));
    }
}
