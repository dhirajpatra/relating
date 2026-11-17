pub mod conformance;
pub mod diff;
pub mod document;
pub mod error;
pub mod export;
pub mod live;
pub mod proposal;

pub use conformance::{check_conformance, ConformanceReport, ContractCheck};
pub use diff::{diff_documents, ChangeKind, SchemaChange, SchemaDiff};
pub use document::{
    IndexDef, ProfileSpec, RetrievalSpec, SchemaDocument, UniqueConstraint, SCHEMA_FORMAT_VERSION,
};
pub use error::SchemaError;
pub use export::export_files;
pub use live::build_live_document;
pub use proposal::{propose_migration, MigrationProposal};
