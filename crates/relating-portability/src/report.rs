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

fn sarif_level(verdict: PortVerdict) -> &'static str {
    match verdict {
        PortVerdict::Portable => "none",
        PortVerdict::Transform => "note",
        PortVerdict::NeedsReview => "warning",
        PortVerdict::NeedsRewrite => "error",
    }
}

fn sarif_rule_name(rule_id: &str) -> &'static str {
    match rule_id {
        "R-APOC" => "APOC / procedure availability",
        "R-PROC" => "Custom procedure portability",
        "R-PERIODIC" => "USING PERIODIC COMMIT rewrite",
        "R-LOADCSV" => "LOAD CSV path portability",
        "R-SHORTEST" => "Shortest-path semantics review",
        "R-SUBQUERY" => "CALL subquery version review",
        "R-NOPARAMS" => "Query parameterization",
        "R-MULTI" => "Multi-statement transaction review",
        "R-DDL-OLD" => "Legacy constraint syntax",
        "R-DRIVER" => "Driver import portability",
        "R-URI" => "Encrypted Bolt URI",
        "R-TX" => "Explicit transaction usage",
        "R-DDL" => "Inline schema DDL",
        _ => "Portable Cypher",
    }
}

/// Render the report as SARIF 2.1.0 so findings show up as code-scanning
/// alerts (upload with `github/codeql-action/upload-sarif`, or open in any
/// SARIF viewer). Only non-portable findings become results; portable files
/// are omitted to keep the alert list actionable.
pub fn to_sarif(report: &PortableReport) -> serde_json::Value {
    use std::collections::BTreeMap;
    let mut rules: BTreeMap<String, &PortFinding> = BTreeMap::new();
    for finding in &report.findings {
        if finding.verdict != PortVerdict::Portable {
            rules.entry(finding.rule_id.clone()).or_insert(finding);
        }
    }
    let rules_json: Vec<serde_json::Value> = rules
        .iter()
        .map(|(rule_id, sample)| {
            serde_json::json!({
                "id": rule_id,
                "name": sarif_rule_name(rule_id),
                "shortDescription": {"text": sample.message},
            })
        })
        .collect();
    let results: Vec<serde_json::Value> = report
        .findings
        .iter()
        .filter(|f| f.verdict != PortVerdict::Portable)
        .map(|finding| {
            // Note: remediation is prose guidance, not a code replacement, so
            // it rides in `properties` — a SARIF `fix` object requires
            // `artifactChanges`, which we cannot produce, and upload-sarif
            // rejects results that carry a `fixes` entry without one.
            let mut result = serde_json::json!({
                "ruleId": finding.rule_id,
                "level": sarif_level(finding.verdict),
                "message": {"text": finding.message},
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": finding.file},
                        "region": {"startLine": finding.line.max(1)}
                    }
                }],
            });
            if let Some(text) = finding.remediation.as_ref() {
                result["properties"] = serde_json::json!({"remediation": text});
            }
            result
        })
        .collect();
    serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "relating-portability",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/dhirajpatra/relating",
                    "rules": rules_json,
                }
            },
            "results": results,
        }],
    })
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

    #[test]
    fn test_sarif_reports_only_actionable_findings() {
        let scan = scan_with(vec![
            "MATCH (n) RETURN n",
            "USING PERIODIC COMMIT 100 LOAD CSV FROM 'f' AS r CREATE (n:N)",
        ]);
        let report = build_report(&scan);
        let sarif = to_sarif(&report);
        assert_eq!(sarif["version"], "2.1.0");
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        // The portable query is omitted; the rewrite is an error.
        assert_eq!(results.len(), 2); // R-PERIODIC + R-LOADCSV
        assert!(results.iter().any(|r| r["ruleId"] == "R-PERIODIC"
            && r["level"] == "error"
            && r["locations"][0]["physicalLocation"]["region"]["startLine"] == 2));
        let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        assert!(rules.iter().any(|r| r["id"] == "R-PERIODIC"));
    }

    #[test]
    fn test_sarif_results_carry_no_invalid_fixes() {
        // upload-sarif rejects any result whose `fixes` entry lacks the
        // required `artifactChanges` property, so remediation must never be
        // emitted as a fix — it rides in `properties` instead.
        let scan = scan_with(vec![
            "USING PERIODIC COMMIT 100 LOAD CSV FROM 'f' AS r CREATE (n:N)",
        ]);
        let report = build_report(&scan);
        let sarif = to_sarif(&report);
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert!(!results.is_empty());
        for result in results {
            assert!(
                result.get("fixes").is_none(),
                "result must not carry fixes: {}",
                result
            );
        }
        let periodic = results
            .iter()
            .find(|r| r["ruleId"] == "R-PERIODIC")
            .unwrap();
        assert!(periodic["properties"]["remediation"]
            .as_str()
            .unwrap()
            .contains("UNWIND"));
    }
}
