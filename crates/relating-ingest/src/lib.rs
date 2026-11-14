pub mod error;
pub mod extractor;
pub mod hasher;
pub mod loader;
pub mod ontology_generator;
pub mod parsers;
pub mod record;

pub use error::IngestError;
pub use extractor::extract_entities_and_relationships;
pub use hasher::compute_sha256;
pub use loader::load_extracted_to_graph;
pub use ontology_generator::{derive_rule_based_ontology, propose_ontology};
pub use parsers::parse_source_content;
pub use record::{NormalizedRecord, SourceProfile};
