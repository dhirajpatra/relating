use crate::compat::{CapabilityVerdict, CompatFinding};
use crate::error::MigrateError;
use serde::{Deserialize, Serialize};

/// Neo4j vector index dimension limits (Neo4j 5): 1–4096 dimensions.
/// Vectors outside this range cannot be indexed on Aura at all.
pub const MIN_VECTOR_DIMENSIONS: usize = 1;
pub const MAX_VECTOR_DIMENSIONS: usize = 4096;

/// Float32 vector storage per dimension.
pub const VECTOR_BYTES_PER_DIMENSION: u64 = 4;

/// Conservative share of tier RAM budgeted for raw vector bytes. The index
/// itself (HNSW graph), page cache, heap, and query working memory need the
/// rest, so vectors should stay well under total RAM. This is a planning
/// heuristic, never a guarantee — Aura tiers and limits change, and teams
/// must confirm against current Neo4j documentation and measured usage.
pub const VECTOR_RAM_BUDGET_FRACTION: f64 = 0.25;

/// Above this fraction of tier RAM, vectors are flagged for manual review
/// even when they still fit the nominal budget.
pub const VECTOR_RAM_REVIEW_FRACTION: f64 = 0.5;

/// An Aura target tier, identified by RAM. RAM figures for named presets are
/// documented estimates (Aura does not publish exact per-tier RAM for every
/// offering); callers can always pass an explicit `custom:<mb>` spec.
///
/// Capacity caps (`max_*`) are likewise documented estimates for tiers with
/// published hard limits (notably AuraDB Free); `None` means "no published
/// cap — sized by memory, not by count". Teams must confirm against the
/// current Neo4j documentation before migrating production data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuraTier {
    pub id: String,
    pub display_name: String,
    pub ram_mb: u64,
    pub note: String,
    /// Published node cap, when the tier documents one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u64>,
    /// Published relationship cap, when the tier documents one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_relationships: Option<u64>,
}

/// Above this fraction of a published cap, the fit is flagged for manual
/// review even though the data still fits nominally.
pub const CAPACITY_REVIEW_FRACTION: f64 = 0.8;

impl AuraTier {
    pub fn ram_bytes(&self) -> u64 {
        self.ram_mb.saturating_mul(1024 * 1024)
    }

    pub fn vector_budget_bytes(&self) -> u64 {
        (self.ram_bytes() as f64 * VECTOR_RAM_BUDGET_FRACTION) as u64
    }
}

pub fn well_known_tiers() -> Vec<AuraTier> {
    vec![
        AuraTier {
            id: "free".to_string(),
            display_name: "AuraDB Free (~1 GB, shared)".to_string(),
            ram_mb: 1024,
            note: "Estimate: Free tier resources are shared and limited; confirm vector support and caps in the Aura console before migrating embeddings."
                .to_string(),
            // Documented AuraDB Free estimates: hard caps on graph size.
            max_nodes: Some(200_000),
            max_relationships: Some(400_000),
        },
        AuraTier {
            id: "professional-2gb".to_string(),
            display_name: "AuraDB Professional 2 GB".to_string(),
            ram_mb: 2048,
            note: "Estimate: pick the preset matching the provisioned instance size.".to_string(),
            max_nodes: None,
            max_relationships: None,
        },
        AuraTier {
            id: "professional-4gb".to_string(),
            display_name: "AuraDB Professional 4 GB".to_string(),
            ram_mb: 4096,
            note: "Estimate: pick the preset matching the provisioned instance size.".to_string(),
            max_nodes: None,
            max_relationships: None,
        },
        AuraTier {
            id: "professional-8gb".to_string(),
            display_name: "AuraDB Professional 8 GB".to_string(),
            ram_mb: 8192,
            note: "Estimate: pick the preset matching the provisioned instance size.".to_string(),
            max_nodes: None,
            max_relationships: None,
        },
        AuraTier {
            id: "dedicated-16gb".to_string(),
            display_name: "Aura Dedicated 16 GB+".to_string(),
            ram_mb: 16384,
            note: "Starting point for dedicated tiers; use custom:<mb> for the exact instance size."
                .to_string(),
            max_nodes: None,
            max_relationships: None,
        },
    ]
}

/// Parse a tier spec: a preset id (`free`, `professional-4gb`, ...),
/// `<N>gb` / `<N>mb`, or `custom:<mb>`.
pub fn parse_tier(spec: &str) -> Result<AuraTier, MigrateError> {
    let trimmed = spec.trim().to_ascii_lowercase();
    if let Some(preset) = well_known_tiers().into_iter().find(|t| t.id == trimmed) {
        return Ok(preset);
    }
    // Accept generated ids (custom-<N>mb) as well as the documented specs.
    let normalized = if trimmed.starts_with("custom-") && trimmed.ends_with("mb") {
        format!("custom:{}", &trimmed[7..trimmed.len() - 2])
    } else {
        trimmed.clone()
    };
    let spec = normalized.as_str();
    let (number_text, multiplier): (&str, u64) = if let Some(n) = spec.strip_prefix("custom:") {
        (n, 1)
    } else if let Some(n) = spec.strip_suffix("gb") {
        (n, 1024)
    } else if let Some(n) = spec.strip_suffix("mb") {
        (n, 1)
    } else {
        (spec, 0)
    };
    if multiplier == 0 {
        return Err(MigrateError::Plan(format!(
            "Unknown Aura tier '{}'. Use one of: {} — or '<N>gb', '<N>mb', 'custom:<mb>'.",
            spec,
            well_known_tiers()
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let number: u64 = number_text.trim().parse().map_err(|_| {
        MigrateError::Plan(format!(
            "Invalid Aura tier '{}': expected a number before gb/mb or after 'custom:'.",
            spec
        ))
    })?;
    let ram_mb = number.saturating_mul(multiplier);
    if ram_mb == 0 {
        return Err(MigrateError::Plan(format!(
            "Invalid Aura tier '{}': RAM must be at least 1 MB.",
            spec
        )));
    }
    Ok(AuraTier {
        id: format!("custom-{}mb", ram_mb),
        display_name: format!("Custom ({} MB RAM)", ram_mb),
        ram_mb,
        note: "Caller-supplied instance size.".to_string(),
        max_nodes: None,
        max_relationships: None,
    })
}

/// Pre-flight capacity check: source graph size against the tier's published
/// node/relationship caps. Tiers without published caps report Supported
/// (sized by memory, verified by the vector-fit and batch checks instead).
pub fn check_capacity_fit(
    node_count: u64,
    relationship_count: u64,
    tier: &AuraTier,
) -> Vec<CompatFinding> {
    let mut findings = Vec::new();
    for (kind, count, cap) in [
        ("nodes", node_count, tier.max_nodes),
        ("relationships", relationship_count, tier.max_relationships),
    ] {
        let Some(cap) = cap else {
            findings.push(CompatFinding {
                category: "tier_capacity".to_string(),
                subject: format!("{} on {}", kind, tier.display_name),
                verdict: CapabilityVerdict::Supported,
                detail: format!(
                    "Source has {} {}; {} publishes no hard {} cap (sized by memory)",
                    count, kind, tier.display_name, kind
                ),
                remediation: None,
            });
            continue;
        };
        let review_at = (cap as f64 * CAPACITY_REVIEW_FRACTION) as u64;
        let (verdict, detail, remediation) = if count > cap {
            (
                CapabilityVerdict::Unsupported,
                format!(
                    "Source has {} {} but {} caps at ~{}: the migration would exceed the tier",
                    count, kind, tier.display_name, cap
                ),
                Some("Use a larger Aura tier or migrate a subset of the graph".to_string()),
            )
        } else if count > review_at {
            (
                CapabilityVerdict::RequiresManualReview,
                format!(
                    "Source has {} {} against a ~{} cap on {}: fits, but leaves little headroom for growth",
                    count, kind, cap, tier.display_name
                ),
                Some(
                    "Confirm headroom in the Aura console; Delta pushes after this migration count toward the same cap"
                        .to_string(),
                ),
            )
        } else {
            (
                CapabilityVerdict::Supported,
                format!(
                    "Source has {} {} against a ~{} cap on {}",
                    count, kind, cap, tier.display_name
                ),
                None,
            )
        };
        findings.push(CompatFinding {
            category: "tier_capacity".to_string(),
            subject: format!("{} on {}", kind, tier.display_name),
            verdict,
            detail,
            remediation,
        });
    }
    findings
}

/// Deterministic blockers for the dry-run/approve gate: source sizes that
/// exceed the tier's published caps fail on Aura no matter what.
pub fn capacity_blockers(node_count: u64, relationship_count: u64, tier: &AuraTier) -> Vec<String> {
    let mut blockers = Vec::new();
    for (kind, count, cap) in [
        ("nodes", node_count, tier.max_nodes),
        ("relationships", relationship_count, tier.max_relationships),
    ] {
        if let Some(cap) = cap {
            if count > cap {
                blockers.push(format!(
                    "Source has {} {} but {} caps at ~{}",
                    count, kind, tier.display_name, cap
                ));
            }
        }
    }
    blockers
}

/// One embedding profile's vector load destined for a Neo4j vector index.
#[derive(Debug, Clone)]
pub struct VectorWorkload {
    pub profile_name: String,
    pub dimensions: usize,
    pub vector_count: u64,
}

pub fn estimate_vector_bytes(vector_count: u64, dimensions: usize) -> u64 {
    vector_count
        .saturating_mul(dimensions as u64)
        .saturating_mul(VECTOR_BYTES_PER_DIMENSION)
}

pub fn format_mb(bytes: u64) -> f64 {
    (bytes as f64 / 1_048_576.0 * 10.0).round() / 10.0
}

/// Per-profile fit assessment shared by the compatibility analyzer and the
/// performance advisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorFit {
    pub profile_name: String,
    pub dimensions: usize,
    pub vector_count: u64,
    pub estimated_bytes: u64,
    pub estimated_mb: f64,
    pub tier_id: Option<String>,
    pub budget_bytes: Option<u64>,
    pub verdict: CapabilityVerdict,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

pub fn assess_workloads(workloads: &[VectorWorkload], tier: Option<&AuraTier>) -> Vec<VectorFit> {
    workloads
        .iter()
        .map(|workload| {
            let estimated_bytes =
                estimate_vector_bytes(workload.vector_count, workload.dimensions);
            if workload.dimensions < MIN_VECTOR_DIMENSIONS
                || workload.dimensions > MAX_VECTOR_DIMENSIONS
            {
                return VectorFit {
                    profile_name: workload.profile_name.clone(),
                    dimensions: workload.dimensions,
                    vector_count: workload.vector_count,
                    estimated_bytes,
                    estimated_mb: format_mb(estimated_bytes),
                    tier_id: tier.map(|t| t.id.clone()),
                    budget_bytes: tier.map(|t| t.vector_budget_bytes()),
                    verdict: CapabilityVerdict::Unsupported,
                    detail: format!(
                        "Profile '{}' has {} dimensions; Neo4j vector indexes require {}–{} dimensions and this profile cannot be indexed on Aura",
                        workload.profile_name,
                        workload.dimensions,
                        MIN_VECTOR_DIMENSIONS,
                        MAX_VECTOR_DIMENSIONS
                    ),
                    remediation: Some(
                        "Re-embed with a model of at most 4096 dimensions before migrating"
                            .to_string(),
                    ),
                };
            }
            match tier {
                None => VectorFit {
                    profile_name: workload.profile_name.clone(),
                    dimensions: workload.dimensions,
                    vector_count: workload.vector_count,
                    estimated_bytes,
                    estimated_mb: format_mb(estimated_bytes),
                    tier_id: None,
                    budget_bytes: None,
                    verdict: CapabilityVerdict::RequiresManualReview,
                    detail: format!(
                        "Profile '{}': {} vectors x {} dims ~= {} MB of raw vector bytes; select a target Aura tier to verify RAM headroom before batch writes",
                        workload.profile_name,
                        workload.vector_count,
                        workload.dimensions,
                        format_mb(estimated_bytes)
                    ),
                    remediation: Some(
                        "Re-run analysis with an Aura tier (or custom RAM size)".to_string(),
                    ),
                },
                Some(tier) => {
                    let budget = tier.vector_budget_bytes();
                    let review_at = (tier.ram_bytes() as f64 * VECTOR_RAM_REVIEW_FRACTION) as u64;
                    let (verdict, detail, remediation) = if estimated_bytes > budget {
                        (
                            CapabilityVerdict::Unsupported,
                            format!(
                                "Profile '{}': {} vectors x {} dims ~= {} MB exceeds the {:.0} MB conservative vector budget for {} (25% of tier RAM); batch writes would risk Aura out-of-memory failures",
                                workload.profile_name,
                                workload.vector_count,
                                workload.dimensions,
                                format_mb(estimated_bytes),
                                format_mb(budget),
                                tier.display_name
                            ),
                            Some("Use a larger Aura tier, reduce vector count/dimensions, or migrate embeddings separately after the graph".to_string()),
                        )
                    } else if estimated_bytes > review_at {
                        (
                            CapabilityVerdict::RequiresManualReview,
                            format!(
                                "Profile '{}': {} vectors x {} dims ~= {} MB fits the {:.0} MB budget for {} but leaves limited headroom; measure index build memory on a staging instance first",
                                workload.profile_name,
                                workload.vector_count,
                                workload.dimensions,
                                format_mb(estimated_bytes),
                                format_mb(budget),
                                tier.display_name
                            ),
                            Some("Build the vector index on Aura staging and watch memory before the production migration".to_string()),
                        )
                    } else {
                        (
                            CapabilityVerdict::Supported,
                            format!(
                                "Profile '{}': {} vectors x {} dims ~= {} MB fits the {:.0} MB conservative vector budget for {}",
                                workload.profile_name,
                                workload.vector_count,
                                workload.dimensions,
                                format_mb(estimated_bytes),
                                format_mb(budget),
                                tier.display_name
                            ),
                            None,
                        )
                    };
                    VectorFit {
                        profile_name: workload.profile_name.clone(),
                        dimensions: workload.dimensions,
                        vector_count: workload.vector_count,
                        estimated_bytes,
                        estimated_mb: format_mb(estimated_bytes),
                        tier_id: Some(tier.id.clone()),
                        budget_bytes: Some(budget),
                        verdict,
                        detail,
                        remediation,
                    }
                }
            }
        })
        .collect()
}

/// Compatibility findings for the analyzer: dimension limits, per-tier RAM
/// fit, and the single-vector-per-index rule.
pub fn check_vector_fit(
    workloads: &[VectorWorkload],
    tier: Option<&AuraTier>,
) -> Vec<CompatFinding> {
    let mut findings: Vec<CompatFinding> = assess_workloads(workloads, tier)
        .into_iter()
        .map(|fit| CompatFinding {
            category: "vector_index_fit".to_string(),
            subject: format!(
                "profile '{}' ({} dims, {} vectors)",
                fit.profile_name, fit.dimensions, fit.vector_count
            ),
            verdict: fit.verdict,
            detail: fit.detail,
            remediation: fit.remediation,
        })
        .collect();

    if workloads.len() > 1 {
        findings.push(CompatFinding {
            category: "vector_index_fit".to_string(),
            subject: format!("{} embedding profiles", workloads.len()),
            verdict: CapabilityVerdict::Transform,
            detail: "Neo4j allows a single vector per index: each embedding profile needs its own vector index on its own node label/property. Plan one CREATE VECTOR INDEX per profile and size each separately.".to_string(),
            remediation: Some(
                "Map each profile to a distinct (label, property) vector index in the migration runbook"
                    .to_string(),
            ),
        });
    }

    findings
}

/// Deterministic blockers for the dry-run/approve gate: dimension violations
/// fail on Aura no matter which tier is chosen, so they block approval.
pub fn vector_dimension_blockers(workloads: &[VectorWorkload]) -> Vec<String> {
    workloads
        .iter()
        .filter(|w| w.dimensions < MIN_VECTOR_DIMENSIONS || w.dimensions > MAX_VECTOR_DIMENSIONS)
        .map(|w| {
            format!(
                "Embedding profile '{}' has {} dimensions; Neo4j vector indexes require {}–{}",
                w.profile_name, w.dimensions, MIN_VECTOR_DIMENSIONS, MAX_VECTOR_DIMENSIONS
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload(name: &str, dims: usize, count: u64) -> VectorWorkload {
        VectorWorkload {
            profile_name: name.to_string(),
            dimensions: dims,
            vector_count: count,
        }
    }

    #[test]
    fn test_parse_tier_presets_and_custom() {
        assert_eq!(parse_tier("free").unwrap().ram_mb, 1024);
        assert_eq!(parse_tier("Professional-4GB").unwrap().ram_mb, 4096);
        assert_eq!(parse_tier("8gb").unwrap().ram_mb, 8192);
        assert_eq!(parse_tier("1536mb").unwrap().ram_mb, 1536);
        assert_eq!(parse_tier("custom:2048").unwrap().ram_mb, 2048);
        assert!(parse_tier("mystery-tier").is_err());
        assert!(parse_tier("custom:0").is_err());
        assert!(parse_tier("gb").is_err());
    }

    #[test]
    fn test_dimensions_outside_1_to_4096_are_unsupported() {
        let fits = assess_workloads(&[workload("big", 5000, 10), workload("ok", 768, 10)], None);
        assert_eq!(fits[0].verdict, CapabilityVerdict::Unsupported);
        assert!(fits[0].remediation.is_some());
        // Valid dims without a tier ask for a tier instead of guessing RAM.
        assert_eq!(fits[1].verdict, CapabilityVerdict::RequiresManualReview);

        let blockers = vector_dimension_blockers(&[workload("big", 5000, 10)]);
        assert_eq!(blockers.len(), 1);
        assert!(blockers[0].contains("5000"));
        assert!(vector_dimension_blockers(&[workload("ok", 768, 10)]).is_empty());
    }

    #[test]
    fn test_ram_fit_bands_against_tier() {
        let tier = parse_tier("free").unwrap(); // 1024 MB RAM, 256 MB budget
                                                // 10k x 768 x 4B ~= 29 MB -> fits.
        let fits = assess_workloads(&[workload("small", 768, 10_000)], Some(&tier));
        assert_eq!(fits[0].verdict, CapabilityVerdict::Supported);
        // 10M x 768 x 4B ~= 29 GB -> exceeds budget.
        let fits = assess_workloads(&[workload("huge", 768, 10_000_000)], Some(&tier));
        assert_eq!(fits[0].verdict, CapabilityVerdict::Unsupported);
        assert!(fits[0].detail.contains("out-of-memory"));
    }

    #[test]
    fn test_multiple_profiles_flag_single_vector_per_index() {
        let findings = check_vector_fit(
            &[workload("a", 128, 100), workload("b", 128, 100)],
            Some(&parse_tier("free").unwrap()),
        );
        assert!(findings
            .iter()
            .any(|f| f.verdict == CapabilityVerdict::Transform
                && f.detail.contains("single vector per index")));
    }

    #[test]
    fn test_estimate_vector_bytes_saturates() {
        assert_eq!(estimate_vector_bytes(1000, 768), 1000 * 768 * 4);
        assert_eq!(estimate_vector_bytes(u64::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn test_capacity_fit_bands_against_free_caps() {
        let free = parse_tier("free").unwrap();
        assert_eq!(free.max_nodes, Some(200_000));

        let fits = check_capacity_fit(1_000, 2_000, &free);
        assert!(fits
            .iter()
            .all(|f| f.verdict == CapabilityVerdict::Supported));

        let review = check_capacity_fit(190_000, 10, &free);
        assert!(review
            .iter()
            .any(|f| f.subject.contains("nodes")
                && f.verdict == CapabilityVerdict::RequiresManualReview));

        let over = check_capacity_fit(1_000, 500_000, &free);
        assert!(over
            .iter()
            .any(|f| f.subject.contains("relationships")
                && f.verdict == CapabilityVerdict::Unsupported));
        assert_eq!(capacity_blockers(1_000, 500_000, &free).len(), 1);
        assert!(capacity_blockers(1_000, 2_000, &free).is_empty());

        // Uncapped tiers report Supported, never block.
        let pro = parse_tier("professional-8gb").unwrap();
        let fits = check_capacity_fit(10_000_000, 50_000_000, &pro);
        assert!(fits
            .iter()
            .all(|f| f.verdict == CapabilityVerdict::Supported));
        assert!(capacity_blockers(10_000_000, 50_000_000, &pro).is_empty());
    }
}
