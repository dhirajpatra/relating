pub mod analysis;
pub mod aura_tiers;
pub mod backend;
pub mod bolt;
pub mod compat;
pub mod error;
pub mod execute;
pub mod packstream;
pub mod plan;
pub mod proxy;
pub mod verify;

pub use analysis::{
    analyze_project, analyze_project_with_options, preview_migration, test_target, vector_blockers_for_project,
    AnalysisOptions, AnalysisReport,
};
pub use aura_tiers::{
    assess_workloads, check_vector_fit, estimate_vector_bytes, format_mb, parse_tier,
    vector_dimension_blockers, well_known_tiers, AuraTier, VectorFit, VectorWorkload,
    MAX_VECTOR_DIMENSIONS, MIN_VECTOR_DIMENSIONS, VECTOR_BYTES_PER_DIMENSION,
    VECTOR_RAM_BUDGET_FRACTION,
};
pub use backend::{
    BoltBackend, MockNeo4jBackend, Neo4jBackend, QueryRecords, TargetInfo,
};
pub use proxy::{apply_proxy_limit, proxied_read, summarize_write_meta};
pub use bolt::{chunk_message, BoltClient, BoltConfig, BoltUri};
pub use compat::{analyze_source, analyze_target, CapabilityVerdict, CompatFinding};
pub use error::MigrateError;
pub use execute::{migrate_node_batch, migrate_relationship_batch, BatchOutcome};
pub use packstream::{
    bolt_to_json, decode_message, encode_message, json_to_bolt, BoltNode, BoltRelationship,
    BoltValue,
};
pub use plan::{
    build_plan, constraint_ddl, dry_run, index_ddl, BatchKind, BatchPlan, DryRunReport,
    MigrationPlan, DEFAULT_BATCH_SIZE,
};
pub use verify::{verify_migration, SampleExpectation, SourceCounts, VerificationReport};
