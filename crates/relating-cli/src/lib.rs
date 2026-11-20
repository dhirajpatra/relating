pub mod checks;
pub mod error;
pub mod eval_runner;
pub mod output;

pub use checks::{CheckOutcome, Context};
pub use error::CliError;
pub use eval_runner::{run_eval_questions, EvalCliReport};
pub use output::{render_validate, OutputFormat, ValidateReport};
