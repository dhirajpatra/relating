pub mod csv_parser;
pub mod json_parser;
pub mod text_parser;

use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use relating_core::SourceId;

pub fn parse_source_content(
    source_id: SourceId,
    filename: &str,
    content_type: Option<&str>,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let lower_name = filename.to_lowercase();
    let is_json = lower_name.ends_with(".json")
        || content_type.map(|c| c.contains("json")).unwrap_or(false);
    let is_csv = lower_name.ends_with(".csv")
        || content_type.map(|c| c.contains("csv")).unwrap_or(false);

    if is_json {
        json_parser::parse_json_content(source_id, filename, content)
    } else if is_csv {
        csv_parser::parse_csv_content(source_id, filename, content)
    } else {
        // Plain text, Markdown, DOCX/PDF text fallback
        text_parser::parse_text_content(source_id, filename, content)
    }
}
