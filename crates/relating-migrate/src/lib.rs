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
    analyze_project, analyze_project_with_options, preview_migration,
    preview_migration_with_options, property_shape_blockers, property_shape_findings,
    scan_property_shapes, source_blockers_for_dry_run, test_target, uidless_node_count,
    vector_blockers_for_project, AnalysisOptions, AnalysisReport, PropertyShapeReport,
    PROPERTY_SCAN_SAMPLE_LIMIT, PROPERTY_SHAPE_CAP,
};
pub use aura_tiers::{
    assess_workloads, capacity_blockers, check_capacity_fit, check_vector_fit,
    estimate_vector_bytes, format_mb, parse_tier, vector_dimension_blockers, well_known_tiers,
    AuraTier, VectorFit, VectorWorkload, CAPACITY_REVIEW_FRACTION, MAX_VECTOR_DIMENSIONS,
    MIN_VECTOR_DIMENSIONS, VECTOR_BYTES_PER_DIMENSION, VECTOR_RAM_BUDGET_FRACTION,
};
pub use backend::{BoltBackend, MockNeo4jBackend, Neo4jBackend, QueryRecords, TargetInfo};
pub use bolt::{chunk_message, BoltClient, BoltConfig, BoltUri};
pub use compat::{analyze_source, analyze_target, CapabilityVerdict, CompatFinding};
pub use error::MigrateError;
pub use execute::{
    is_hash_fallback_uid, migrate_node_batch, migrate_node_batch_with_options,
    migrate_relationship_batch, migrate_relationship_batch_with_options, propagate_node_deletes,
    propagate_rel_deletes, relationship_uid, rollback_migration, BatchOutcome,
    DeletePropagationOutcome, RollbackOutcome, MIGRATION_TAG_PROPERTY, UPDATED_AT_PROPERTY,
    VERSION_PROPERTY,
};
pub use packstream::{
    bolt_to_json, decode_message, encode_message, json_to_bolt, BoltNode, BoltRelationship,
    BoltValue,
};
pub use plan::{
    build_plan, constraint_ddl, dry_run, index_ddl, BatchKind, BatchPlan, DryRunReport,
    MigrationPlan, DEFAULT_BATCH_SIZE, MAX_RECOMMENDED_BATCH_SIZE,
};
pub use proxy::{apply_proxy_limit, proxied_read, summarize_write_meta};
pub use verify::{
    uid_list_hash, verify_migration, CountCheck, DegreeCheck, DegreeExpectation, HashCheck,
    SampleExpectation, SourceCounts, VerificationReport, MAX_HASH_UIDS_PER_LABEL,
};
