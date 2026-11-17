pub mod error;
pub mod generate;
pub mod materialize;
pub mod profile;

pub use error::SynthError;
pub use generate::{
    generate_dataset, EdgeCase, SynthConfig, SynthDataset, SynthNode, SynthRel, SynthStats,
};
pub use materialize::{materialize_dataset, MaterializeOutcome};
pub use profile::{
    profile_graph, GraphProfile, LabelProfile, PropertyProfile, RelProfile, RelSample,
};
