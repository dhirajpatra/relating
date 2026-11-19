use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use relating_core::SourceId;
use std::collections::HashMap;

/// Parse a PDF document into one record per text block, preserving the page
/// number on every record (`page_number` field, `page_<n>_block_<m>` lineage).
/// Empty pages produce no records but keep page numbering true.
pub fn parse_pdf_content(
    source_id: SourceId,
    filename: &str,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    if !content.starts_with(b"%PDF") {
        return Err(IngestError::ParseError {
            filename: filename.to_string(),
            message: "Not a PDF document: missing %PDF header. Encrypted PDFs must be decrypted before upload.".to_string(),
        });
    }
    let pages = pdf_extract::extract_text_from_mem_by_pages(content).map_err(|e| {
        IngestError::ParseError {
            filename: filename.to_string(),
            message: format!(
                "PDF text extraction failed ({}). Scanned-image PDFs need OCR before upload; encrypted PDFs must be decrypted first.",
                e
            ),
        }
    })?;

    let mut records = Vec::new();
    for (page_idx, page_text) in pages.iter().enumerate() {
        let page_number = page_idx + 1;
        let blocks: Vec<&str> = page_text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        for (block_idx, block) in blocks.iter().enumerate() {
            let mut fields = HashMap::new();
            fields.insert(
                "text".to_string(),
                serde_json::Value::String(block.to_string()),
            );
            fields.insert(
                "page_number".to_string(),
                serde_json::Value::Number(page_number.into()),
            );
            fields.insert(
                "block_index".to_string(),
                serde_json::Value::Number((block_idx + 1).into()),
            );
            fields.insert(
                "length".to_string(),
                serde_json::Value::Number(block.len().into()),
            );
            records.push(NormalizedRecord {
                source_id: source_id.clone(),
                lineage_ref: format!("page_{}_block_{}", page_number, block_idx + 1),
                fields,
            });
        }
    }

    let sample_records = records
        .iter()
        .take(5)
        .map(|r| r.fields.clone())
        .collect::<Vec<_>>();

    let profile = SourceProfile {
        total_records: records.len(),
        field_names: vec![
            "text".to_string(),
            "page_number".to_string(),
            "block_index".to_string(),
            "length".to_string(),
        ],
        sample_records,
    };

    Ok((records, profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal single-page PDF with a correct xref table so the
    /// parser test needs no fixture files.
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let content_stream = format!("BT /F1 12 Tf 10 180 Td ({}) Tj ET", text);
        let objects: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content_stream.len(),
                content_stream
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
        }
        let xref_pos = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            pdf.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                objects.len() + 1,
                xref_pos
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn test_pdf_extracts_text_with_page_lineage() {
        let pdf = minimal_pdf("Hello GraphRAG");
        let (records, profile) = parse_pdf_content(SourceId::new(), "hello.pdf", &pdf).unwrap();
        assert!(!records.is_empty(), "expected at least one record");
        assert!(records.iter().any(|r| r.fields["text"]
            .as_str()
            .unwrap()
            .contains("Hello GraphRAG")));
        assert!(records
            .iter()
            .all(|r| r.lineage_ref.starts_with("page_1_block_")));
        assert_eq!(records[0].fields["page_number"], 1);
        assert_eq!(profile.total_records, records.len());
        assert!(profile.field_names.contains(&"page_number".to_string()));
    }

    #[test]
    fn test_non_pdf_bytes_are_rejected_clearly() {
        let err = parse_pdf_content(SourceId::new(), "fake.pdf", b"not a pdf").unwrap_err();
        match err {
            IngestError::ParseError { filename, message } => {
                assert_eq!(filename, "fake.pdf");
                assert!(message.contains("%PDF"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }
}
