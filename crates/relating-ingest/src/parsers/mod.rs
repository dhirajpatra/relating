pub mod csv_parser;
pub mod docx_parser;
pub mod json_parser;
pub mod pdf_parser;
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
    let content_type = content_type.unwrap_or_default().to_lowercase();
    let is_json = lower_name.ends_with(".json") || content_type.contains("json");
    let is_csv = lower_name.ends_with(".csv") || content_type.contains("csv");
    let is_pdf = lower_name.ends_with(".pdf") || content_type.contains("pdf");
    let is_docx = lower_name.ends_with(".docx")
        || content_type.contains("wordprocessingml")
        || content_type.contains("officedocument");
    let is_legacy_doc = lower_name.ends_with(".doc") && !is_docx;

    if is_json {
        json_parser::parse_json_content(source_id, filename, content)
    } else if is_csv {
        csv_parser::parse_csv_content(source_id, filename, content)
    } else if is_pdf {
        pdf_parser::parse_pdf_content(source_id, filename, content)
    } else if is_docx {
        docx_parser::parse_docx_content(source_id, filename, content)
    } else if is_legacy_doc {
        Err(IngestError::UnsupportedFormat(
            "Legacy .doc (OLE) documents are not supported; convert to .docx and re-upload"
                .to_string(),
        ))
    } else {
        // Plain text and Markdown.
        text_parser::parse_text_content(source_id, filename, content)
    }
}
