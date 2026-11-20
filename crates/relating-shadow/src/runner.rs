use crate::compare::{compare_query, summarize, QueryComparison, ShadowSummary};
use crate::error::ShadowError;
use crate::replay::{is_read_only, replay_source, replay_target, ShadowQuery};
use relating_graph::GraphBackend;
use relating_migrate::Neo4jBackend;
use std::path::Path;

pub const MAX_SUITE_QUERIES: usize = 100;
pub const MAX_SUITE_CYPHER_LEN: usize = 20000;

pub struct SuiteOptions {
    pub row_limit: u32,
    pub include_samples: bool,
}

impl Default for SuiteOptions {
    fn default() -> Self {
        Self {
            row_limit: 200,
            include_samples: false,
        }
    }
}

/// Replay every query against source and target, then compare. Shared by the
/// HTTP route and the CLI so both report identical verdicts.
pub async fn replay_suite(
    graph: &dyn GraphBackend,
    store_path: &Path,
    backend: &dyn Neo4jBackend,
    queries: &[ShadowQuery],
    options: &SuiteOptions,
) -> Result<(Vec<QueryComparison>, ShadowSummary), ShadowError> {
    if queries.is_empty() {
        return Err(ShadowError::Invalid(
            "At least one query is required".to_string(),
        ));
    }
    if queries.len() > MAX_SUITE_QUERIES {
        return Err(ShadowError::Invalid(format!(
            "At most {} queries per shadow run",
            MAX_SUITE_QUERIES
        )));
    }

    let mut comparisons = Vec::with_capacity(queries.len());
    for query in queries {
        if query.cypher.trim().is_empty() {
            return Err(ShadowError::Invalid(format!(
                "Query '{}' is empty",
                query.name
            )));
        }
        if query.cypher.len() > MAX_SUITE_CYPHER_LEN {
            return Err(ShadowError::Invalid(format!(
                "Query '{}' exceeds {} characters",
                query.name, MAX_SUITE_CYPHER_LEN
            )));
        }
        if !is_read_only(&query.cypher) {
            return Err(ShadowError::Invalid(format!(
                "Refusing non-read query '{}': shadow testing is read-only",
                query.name
            )));
        }

        let source = replay_source(
            graph,
            store_path,
            query,
            options.row_limit,
            options.include_samples,
        )
        .await?;
        let target = replay_target(backend, query, options.row_limit).await?;
        let mut comparison = compare_query(&query.name, &source, &target);
        if options.include_samples {
            comparison.detail = format!(
                "{}. source_sample={:?}",
                comparison.detail,
                source
                    .sample_rows
                    .unwrap_or_default()
                    .iter()
                    .take(2)
                    .collect::<Vec<_>>()
            );
        }
        comparisons.push(comparison);
    }

    let summary = summarize(&comparisons);
    Ok((comparisons, summary))
}
