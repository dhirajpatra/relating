pub mod analysis;
pub mod backend;
pub mod bolt;
pub mod compat;
pub mod error;
pub mod execute;
pub mod packstream;
pub mod plan;
pub mod verify;

pub use analysis::{analyze_project, preview_migration, test_target, AnalysisReport};
pub use backend::{BoltBackend, MockNeo4jBackend, Neo4jBackend, TargetInfo};
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
