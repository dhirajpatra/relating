use crate::error::IngestError;
use crate::record::{NormalizedRecord, SourceProfile};
use quick_xml::events::Event;
use quick_xml::Reader;
use relating_core::SourceId;
use std::collections::HashMap;
use std::io::{Cursor, Read};

#[derive(Default)]
struct ParagraphState {
    style: Option<String>,
    list_item: bool,
    text: String,
}

struct TableState {
    index: usize,
    rows: Vec<Vec<String>>,
    current_row: Option<Vec<String>>,
    current_cell_paras: Vec<String>,
    in_cell: bool,
}

/// Parse a DOCX document (a zip archive containing `word/document.xml`).
///
/// Paragraphs become records with heading-derived `section` context and
/// `para_<n>` lineage; tables become row records with cell coordinates and
/// `table_<t>_row_<r>` lineage so table structure survives ingestion.
pub fn parse_docx_content(
    source_id: SourceId,
    filename: &str,
    content: &[u8],
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let cursor = Cursor::new(content);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| IngestError::ParseError {
        filename: filename.to_string(),
        message: format!(
            "Not a valid DOCX file (DOCX must be a zip archive): {}. Legacy .doc files must be converted to .docx first.",
            e
        ),
    })?;
    let mut document_xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| IngestError::ParseError {
            filename: filename.to_string(),
            message: "Zip archive has no word/document.xml; not a DOCX document".to_string(),
        })?
        .read_to_string(&mut document_xml)
        .map_err(|e| IngestError::ParseError {
            filename: filename.to_string(),
            message: format!("Failed to read word/document.xml: {}", e),
        })?;

    parse_document_xml(source_id, filename, &document_xml)
}

fn unescape_text(reader: &Reader<&[u8]>, raw: &[u8]) -> String {
    let decoded = reader.decoder().decode(raw).unwrap_or_default();
    quick_xml::escape::unescape(&decoded)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| decoded.into_owned())
}

fn parse_document_xml(
    source_id: SourceId,
    filename: &str,
    xml: &str,
) -> Result<(Vec<NormalizedRecord>, SourceProfile), IngestError> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    reader.config_mut().trim_text(true);

    let mut records = Vec::new();
    let mut para_index = 0usize;
    let mut table_count = 0usize;
    let mut section = String::new();
    let mut current_para: Option<ParagraphState> = None;
    let mut table: Option<TableState> = None;
    let mut in_text_run = false;

    let mut buf = Vec::new();
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| IngestError::ParseError {
                filename: filename.to_string(),
                message: format!("Malformed word/document.xml: {}", e),
            })?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = e.name();
                let tag = name.as_ref();
                let is_empty = matches!(event, Event::Empty(_));
                if tag == b"w:p" && !is_empty {
                    current_para = Some(ParagraphState::default());
                } else if tag == b"w:pStyle" {
                    let mut style: Option<String> = None;
                    for attr in e.attributes() {
                        let attr = attr.map_err(|e| IngestError::ParseError {
                            filename: filename.to_string(),
                            message: format!("Malformed paragraph style: {}", e),
                        })?;
                        if attr.key.as_ref() == b"w:val" {
                            style = attr
                                .decode_and_unescape_value(reader.decoder())
                                .ok()
                                .map(|s| s.into_owned());
                        }
                    }
                    if let (Some(state), Some(style)) = (current_para.as_mut(), style) {
                        state.style = Some(style);
                    }
                } else if tag == b"w:numPr" {
                    if let Some(state) = current_para.as_mut() {
                        state.list_item = true;
                    }
                } else if tag == b"w:t" {
                    in_text_run = true;
                } else if tag == b"w:tab" {
                    if let Some(state) = current_para.as_mut() {
                        state.text.push(' ');
                    }
                } else if tag == b"w:br" {
                    if let Some(state) = current_para.as_mut() {
                        state.text.push('\n');
                    }
                } else if tag == b"w:tbl" && !is_empty {
                    table_count += 1;
                    table = Some(TableState {
                        index: table_count,
                        rows: Vec::new(),
                        current_row: None,
                        current_cell_paras: Vec::new(),
                        in_cell: false,
                    });
                } else if tag == b"w:tr" && !is_empty {
                    if let Some(tbl) = table.as_mut() {
                        tbl.current_row = Some(Vec::new());
                    }
                } else if tag == b"w:tc" && !is_empty {
                    if let Some(tbl) = table.as_mut() {
                        tbl.in_cell = true;
                        tbl.current_cell_paras = Vec::new();
                    }
                }
            }
            Event::Text(ref e) => {
                if in_text_run {
                    if let Some(state) = current_para.as_mut() {
                        state.text.push_str(&unescape_text(&reader, e.as_ref()));
                    }
                }
            }
            Event::End(ref e) => {
                let tag = e.name().as_ref().to_vec();
                if tag == b"w:t" {
                    in_text_run = false;
                } else if tag == b"w:p" {
                    if let Some(state) = current_para.take() {
                        let text = state.text.trim().to_string();
                        let in_table_cell = table.as_ref().map(|t| t.in_cell).unwrap_or(false);
                        if in_table_cell {
                            if let Some(tbl) = table.as_mut() {
                                if !text.is_empty() {
                                    tbl.current_cell_paras.push(text);
                                }
                            }
                        } else if !text.is_empty() {
                            para_index += 1;
                            let style = state.style.clone().unwrap_or_default();
                            if style.to_ascii_lowercase().starts_with("heading") {
                                section = text.clone();
                            }
                            let mut fields = HashMap::new();
                            fields.insert("text".to_string(), serde_json::Value::String(text));
                            fields.insert(
                                "paragraph_index".to_string(),
                                serde_json::Value::Number(para_index.into()),
                            );
                            fields.insert(
                                "section".to_string(),
                                serde_json::Value::String(section.clone()),
                            );
                            fields.insert("style".to_string(), serde_json::Value::String(style));
                            fields.insert(
                                "list_item".to_string(),
                                serde_json::Value::Bool(state.list_item),
                            );
                            records.push(NormalizedRecord {
                                source_id: source_id.clone(),
                                lineage_ref: format!("para_{}", para_index),
                                fields,
                            });
                        }
                    }
                } else if tag == b"w:tc" {
                    if let Some(tbl) = table.as_mut() {
                        let cell_text = tbl.current_cell_paras.join(" ");
                        if let Some(row) = tbl.current_row.as_mut() {
                            row.push(cell_text);
                        }
                        tbl.in_cell = false;
                    }
                } else if tag == b"w:tr" {
                    if let Some(tbl) = table.as_mut() {
                        if let Some(row) = tbl.current_row.take() {
                            if row.iter().any(|c| !c.trim().is_empty()) {
                                tbl.rows.push(row);
                            }
                        }
                    }
                } else if tag == b"w:tbl" {
                    if let Some(tbl) = table.take() {
                        for (row_idx, row) in tbl.rows.iter().enumerate() {
                            let mut fields = HashMap::new();
                            fields.insert(
                                "row_text".to_string(),
                                serde_json::Value::String(row.join(" | ")),
                            );
                            fields.insert(
                                "cells".to_string(),
                                serde_json::Value::Array(
                                    row.iter()
                                        .map(|c| serde_json::Value::String(c.clone()))
                                        .collect(),
                                ),
                            );
                            fields.insert(
                                "table_index".to_string(),
                                serde_json::Value::Number((tbl.index).into()),
                            );
                            fields.insert(
                                "row_index".to_string(),
                                serde_json::Value::Number((row_idx + 1).into()),
                            );
                            fields.insert(
                                "section".to_string(),
                                serde_json::Value::String(section.clone()),
                            );
                            records.push(NormalizedRecord {
                                source_id: source_id.clone(),
                                lineage_ref: format!("table_{}_row_{}", tbl.index, row_idx + 1),
                                fields,
                            });
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
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
            "paragraph_index".to_string(),
            "section".to_string(),
            "style".to_string(),
            "row_text".to_string(),
            "cells".to_string(),
        ],
        sample_records,
    };

    Ok((records, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_docx_xml() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Project Plan</w:t></w:r></w:p>
    <w:p><w:r><w:t>First paragraph about graphs.</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr/></w:pPr><w:r><w:t>List item one</w:t></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Role</w:t></w:r></w:p></w:tc></w:tr>
      <w:tr><w:tc><w:p><w:r><w:t>Ada</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Engineer</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
  </w:body>
</w:document>"#
            .to_string()
    }

    fn test_docx_bytes() -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(test_docx_xml().as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn test_docx_paragraphs_tables_and_sections() {
        let bytes = test_docx_bytes();
        let (records, profile) = parse_docx_content(SourceId::new(), "plan.docx", &bytes).unwrap();
        assert_eq!(records.len(), 3 + 2, "3 paras + 2 table rows");

        let paras: Vec<_> = records
            .iter()
            .filter(|r| r.lineage_ref.starts_with("para_"))
            .collect();
        assert_eq!(paras.len(), 3);
        assert_eq!(paras[0].fields["style"], "Heading1");
        // Later paragraphs inherit the heading as their section.
        assert_eq!(paras[1].fields["section"], "Project Plan");
        assert_eq!(paras[2].fields["list_item"], true);

        let rows: Vec<_> = records
            .iter()
            .filter(|r| r.lineage_ref.starts_with("table_"))
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].lineage_ref, "table_1_row_1");
        assert_eq!(rows[1].fields["row_text"], "Ada | Engineer");
        assert_eq!(profile.total_records, 5);
    }

    #[test]
    fn test_non_zip_bytes_are_rejected_clearly() {
        let err = parse_docx_content(SourceId::new(), "fake.docx", b"not a zip").unwrap_err();
        match err {
            IngestError::ParseError { filename, message } => {
                assert_eq!(filename, "fake.docx");
                assert!(message.contains("zip"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }
}
