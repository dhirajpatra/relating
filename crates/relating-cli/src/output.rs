use crate::checks::CheckOutcome;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

#[derive(Serialize)]
pub struct ValidateReport {
    pub passed: bool,
    pub gates: Vec<CheckOutcome>,
}

pub fn render_validate(report: &ValidateReport, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(report).unwrap_or_default(),
        OutputFormat::Text => {
            let mut out = String::new();
            for gate in &report.gates {
                out.push_str(&format!(
                    "[{}] {}\n",
                    if gate.passed { "PASS" } else { "FAIL" },
                    gate.name
                ));
                for error in &gate.errors {
                    out.push_str(&format!("  error: {}\n", error));
                }
                for warning in &gate.warnings {
                    out.push_str(&format!("  warning: {}\n", warning));
                }
            }
            out.push_str(if report.passed {
                "Result: PASS\n"
            } else {
                "Result: FAIL\n"
            });
            out
        }
    }
}
