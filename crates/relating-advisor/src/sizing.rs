use relating_migrate::Neo4jBackend;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizingEstimate {
    pub tier_hint: String,
    pub estimated_elements: u64,
    pub estimated_storage_mb: f64,
    pub assumptions: Vec<String>,
    pub caveats: Vec<String>,
}

/// Rough Aura sizing guidance. This is an ESTIMATE from element counts with
/// stated assumptions — never a quote or a guarantee. Teams must measure
/// with shadow testing on production-like data before committing.
pub fn estimate_sizing(node_count: u64, rel_count: u64) -> SizingEstimate {
    let elements = node_count + rel_count;
    // Documented assumptions, intentionally conservative.
    let bytes_per_node = 512u64; // properties + overhead + indexes headroom
    let bytes_per_rel = 256u64;
    let bytes = node_count * bytes_per_node + rel_count * bytes_per_rel;
    let estimated_storage_mb = bytes as f64 / 1_048_576.0;

    let tier_hint = if elements < 50_000 {
        "Smallest tiers comfortably fit this graph; start small and measure"
    } else if elements < 1_000_000 {
        "Small production tier is a reasonable starting point; verify with load tests"
    } else if elements < 10_000_000 {
        "Mid tier starting point; index strategy and query patterns dominate cost"
    } else {
        "Large graph: size from measured workload (shadow latency + memory), not from counts"
    }
    .to_string();

    SizingEstimate {
        tier_hint,
        estimated_elements: elements,
        estimated_storage_mb: (estimated_storage_mb * 10.0).round() / 10.0,
        assumptions: vec![
            format!(
                "~{} bytes per node, ~{} bytes per relationship (properties + index headroom)",
                bytes_per_node, bytes_per_rel
            ),
            "Vector embeddings and full-text indexes are excluded; add them separately".to_string(),
            "Write throughput and concurrent sessions are excluded".to_string(),
        ],
        caveats: vec![
            "Estimate only: measure planning/execution on the target before committing".to_string(),
            "Aura pricing and tiers change; confirm against current Neo4j documentation"
                .to_string(),
        ],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanMeasurement {
    pub query_name: String,
    pub planned: bool,
    pub planning_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMeasurements {
    pub target_nodes: u64,
    pub target_relationships: u64,
    pub plans: Vec<PlanMeasurement>,
}

/// Measured evidence from the live target: real counts plus per-query
/// EXPLAIN planning checks (planning only — EXPLAIN never executes writes).
pub async fn measure_target(
    backend: &dyn Neo4jBackend,
    queries: &[(String, String)],
) -> Result<TargetMeasurements, relating_migrate::MigrateError> {
    let target_nodes = backend
        .run_read("MATCH (n) RETURN count(n) AS c", BTreeMap::new())
        .await?
        .first()
        .and_then(|row| row.first())
        .and_then(|v| v.as_int())
        .unwrap_or(0)
        .max(0) as u64;
    let target_relationships = backend
        .run_read("MATCH ()-[r]->() RETURN count(r) AS c", BTreeMap::new())
        .await?
        .first()
        .and_then(|row| row.first())
        .and_then(|v| v.as_int())
        .unwrap_or(0)
        .max(0) as u64;

    let mut plans = Vec::new();
    for (name, cypher) in queries {
        let start = std::time::Instant::now();
        match backend
            .run_read(&format!("EXPLAIN {}", cypher), BTreeMap::new())
            .await
        {
            Ok(_) => plans.push(PlanMeasurement {
                query_name: name.clone(),
                planned: true,
                planning_ms: start.elapsed().as_secs_f64() * 1000.0,
                error: None,
            }),
            Err(e) => plans.push(PlanMeasurement {
                query_name: name.clone(),
                planned: false,
                planning_ms: start.elapsed().as_secs_f64() * 1000.0,
                error: Some(e.to_string()),
            }),
        }
    }

    Ok(TargetMeasurements {
        target_nodes,
        target_relationships,
        plans,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use relating_migrate::MockNeo4jBackend;

    #[test]
    fn test_sizing_bands_and_caveats() {
        let small = estimate_sizing(100, 50);
        assert!(small.tier_hint.contains("Smallest"));
        assert!(!small.assumptions.is_empty());
        assert!(!small.caveats.is_empty());
        let large = estimate_sizing(5_000_000, 8_000_000);
        assert!(large.tier_hint.contains("Large graph"));
        assert!(large.estimated_storage_mb > small.estimated_storage_mb);
    }

    #[tokio::test]
    async fn test_measure_target_counts() {
        let backend = MockNeo4jBackend::new();
        let measurements = measure_target(
            &backend,
            &[("q".to_string(), "MATCH (n) RETURN n".to_string())],
        )
        .await
        .unwrap();
        assert_eq!(measurements.target_nodes, 0);
        assert_eq!(measurements.plans.len(), 1);
        assert!(measurements.plans[0].planned);
    }
}
