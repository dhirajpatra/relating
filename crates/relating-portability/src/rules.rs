use crate::cypher::CypherFeatures;
use crate::scanner::{CodeSignal, QueryCandidate, SignalKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortVerdict {
    Portable,
    Transform,
    NeedsRewrite,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRule {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortFinding {
    pub file: String,
    pub line: u32,
    pub rule_id: String,
    pub verdict: PortVerdict,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

fn finding(
    file: &str,
    line: u32,
    rule_id: &str,
    verdict: PortVerdict,
    message: impl Into<String>,
    remediation: Option<String>,
) -> PortFinding {
    PortFinding {
        file: file.to_string(),
        line,
        rule_id: rule_id.to_string(),
        verdict,
        message: message.into(),
        remediation,
    }
}

pub fn check_query(candidate: &QueryCandidate, features: &CypherFeatures) -> Vec<PortFinding> {
    let mut out = Vec::new();
    let (file, line) = (candidate.file.as_str(), candidate.line);

    // R-APOC: namespaced procedures must exist on the target.
    for proc_name in &features.procedures {
        if proc_name.to_lowercase().starts_with("apoc.") {
            out.push(finding(
                file,
                line,
                "R-APOC",
                PortVerdict::NeedsReview,
                format!(
                    "APOC procedure '{}' must be available on the Aura target",
                    proc_name
                ),
                Some(format!(
                    "Confirm '{0}' exists in the target APOC version, or rewrite without it",
                    proc_name
                )),
            ));
        } else if !proc_name.to_lowercase().starts_with("db.") {
            out.push(finding(
                file,
                line,
                "R-PROC",
                PortVerdict::NeedsReview,
                format!(
                    "Custom procedure '{}' has no known Aura equivalent",
                    proc_name
                ),
                Some("Verify the procedure on the target or inline its logic".to_string()),
            ));
        }
    }

    // R-PERIODIC: USING PERIODIC COMMIT is Neo4j-only batching.
    if features.has_periodic_commit {
        out.push(finding(
            file,
            line,
            "R-PERIODIC",
            PortVerdict::NeedsRewrite,
            "USING PERIODIC COMMIT is Neo4j-only; Relate and portable code use explicit batches",
            Some(crate::remediation::rewrite_periodic_commit(
                &candidate.query,
            )),
        ));
    }

    // R-LOADCSV: file access differs between embedded and managed servers.
    if features.has_load_csv {
        out.push(finding(
            file,
            line,
            "R-LOADCSV",
            PortVerdict::NeedsReview,
            "LOAD CSV file paths resolve differently on Aura (import directory / HTTPS)",
            Some(
                "Host the CSV where Aura can reach it, or ingest through Relating sources"
                    .to_string(),
            ),
        ));
    }

    // R-SHORTEST: path algorithms vary by engine and version.
    if features.has_shortest_path {
        out.push(finding(
            file,
            line,
            "R-SHORTEST",
            PortVerdict::NeedsReview,
            "shortestPath semantics and limits vary by engine version",
            Some(
                "Validate result counts and ordering on the target with a generated test"
                    .to_string(),
            ),
        ));
    }

    // R-SUBQUERY: CALL {} requires Neo4j 4.0+ / recent Relate.
    if features.has_call_subquery {
        out.push(finding(
            file,
            line,
            "R-SUBQUERY",
            PortVerdict::NeedsReview,
            "CALL {} subqueries need Neo4j 4.0+ and a recent Relate; verify both ends",
            None,
        ));
    }

    // R-NOPARAMS: literal-heavy queries (heuristic: no $params and long).
    if !features.has_parameters && candidate.query.len() > 300 {
        out.push(finding(
            file,
            line,
            "R-NOPARAMS",
            PortVerdict::Transform,
            "Long query without bound parameters; parameterization is safer and faster",
            Some("Bind literal values as $parameters".to_string()),
        ));
    }

    // R-MULTI: multi-statement strings need explicit transaction handling.
    if features.statement_count > 1 {
        out.push(finding(
            file,
            line,
            "R-MULTI",
            PortVerdict::NeedsReview,
            format!(
                "String holds {} statements; drivers execute one per call unless wrapped in a transaction",
                features.statement_count
            ),
            Some("Split into single statements or run inside an explicit transaction".to_string()),
        ));
    }

    // R-DDL-OLD: legacy ASSERT constraint syntax.
    if candidate.query.to_uppercase().contains(" ASSERT ")
        && candidate.query.to_uppercase().contains("CONSTRAINT")
    {
        out.push(finding(
            file,
            line,
            "R-DDL-OLD",
            PortVerdict::Transform,
            "Legacy ASSERT constraint syntax; Neo4j 5 prefers REQUIRE",
            crate::remediation::rewrite_legacy_constraint(&candidate.query),
        ));
    }

    if out.is_empty() {
        out.push(finding(
            file,
            line,
            "R-OK",
            PortVerdict::Portable,
            "No portability risks detected; standard read/write Cypher",
            None,
        ));
    }
    out
}

pub fn check_signal(signal: &CodeSignal) -> Vec<PortFinding> {
    let (file, line) = (signal.file.as_str(), signal.line);
    match signal.kind {
        SignalKind::DriverImport => {
            let lower = signal.text.to_lowercase();
            let (verdict, message) = if lower.contains("relate") {
                (
                    PortVerdict::Transform,
                    "Relate driver import: swap for the Neo4j driver on the Aura path",
                )
            } else {
                (
                    PortVerdict::Portable,
                    "Neo4j driver import: portable to Aura",
                )
            };
            vec![finding(file, line, "R-DRIVER", verdict, message, None)]
        }
        SignalKind::ConnectionUri => {
            let lower = signal.text.to_lowercase();
            let verdict = if lower.contains("+s://") {
                PortVerdict::Portable
            } else {
                PortVerdict::NeedsReview
            };
            let message = if verdict == PortVerdict::Portable {
                "Encrypted Bolt URI: Aura-compatible"
            } else {
                "Plaintext Bolt URI: Aura requires an encrypted (+s) URI"
            };
            vec![finding(file, line, "R-URI", verdict, message, None)]
        }
        SignalKind::Transaction => vec![finding(
            file,
            line,
            "R-TX",
            PortVerdict::Portable,
            "Explicit transaction usage transfers to Aura; keep batches bounded",
            None,
        )],
        SignalKind::ApocUsage => vec![finding(
            file,
            line,
            "R-APOC",
            PortVerdict::NeedsReview,
            "APOC usage in application code: confirm availability on the target",
            None,
        )],
        SignalKind::ConstraintDdl | SignalKind::IndexDdl => vec![finding(
            file,
            line,
            "R-DDL",
            PortVerdict::Transform,
            "Inline schema DDL: regenerate from schema-as-code for the target version",
            Some("Export constraints/indexes from Relating schema-as-code".to_string()),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cypher::extract_features;
    use crate::scanner::SourceLanguage;

    fn candidate(query: &str) -> QueryCandidate {
        QueryCandidate {
            file: "app.py".to_string(),
            line: 1,
            language: SourceLanguage::Python,
            query: query.to_string(),
        }
    }

    #[test]
    fn test_plain_query_is_portable() {
        let c = candidate("MATCH (p:Person) WHERE p.age > $age RETURN p");
        let findings = check_query(&c, &extract_features(&c.query));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].verdict, PortVerdict::Portable);
    }

    #[test]
    fn test_periodic_commit_needs_rewrite_with_candidate() {
        let c = candidate("USING PERIODIC COMMIT 500 LOAD CSV FROM 'file:///x.csv' AS row CREATE (n:N) SET n += row");
        let findings = check_query(&c, &extract_features(&c.query));
        let f = findings.iter().find(|f| f.rule_id == "R-PERIODIC").unwrap();
        assert_eq!(f.verdict, PortVerdict::NeedsRewrite);
        assert!(f.remediation.as_ref().unwrap().contains("UNWIND"));
    }

    #[test]
    fn test_apoc_needs_review() {
        let c = candidate("CALL apoc.meta.schema() YIELD value RETURN value");
        let findings = check_query(&c, &extract_features(&c.query));
        assert!(findings
            .iter()
            .any(|f| f.rule_id == "R-APOC" && f.verdict == PortVerdict::NeedsReview));
    }

    #[test]
    fn test_relate_driver_is_transform() {
        let signal = CodeSignal {
            file: "app.py".to_string(),
            line: 3,
            kind: SignalKind::DriverImport,
            text: "from relate import Database".to_string(),
        };
        let findings = check_signal(&signal);
        assert_eq!(findings[0].verdict, PortVerdict::Transform);
    }

    #[test]
    fn test_plaintext_uri_needs_review() {
        let signal = CodeSignal {
            file: "app.py".to_string(),
            line: 4,
            kind: SignalKind::ConnectionUri,
            text: "URI = \"bolt://localhost:7687\"".to_string(),
        };
        let findings = check_signal(&signal);
        assert_eq!(findings[0].verdict, PortVerdict::NeedsReview);
    }
}
