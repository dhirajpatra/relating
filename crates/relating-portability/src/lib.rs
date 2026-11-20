pub mod cypher;
pub mod error;
pub mod remediation;
pub mod report;
pub mod rules;
pub mod scanner;

pub use cypher::{extract_features, split_statements, CypherFeatures};
pub use error::PortError;
pub use remediation::{rewrite_legacy_constraint, rewrite_periodic_commit};
pub use report::{
    build_report, GeneratedTest, PortSummary, PortableReport, TestAssertion, MAX_GENERATED_TESTS,
};
pub use rules::{check_query, check_signal, PortFinding, PortRule, PortVerdict};
pub use scanner::{
    scan_repository, CodeSignal, QueryCandidate, RepoScan, SignalKind, SourceLanguage, MAX_FILES,
    MAX_FILE_BYTES, MAX_QUERIES,
};
