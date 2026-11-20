use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkConfig {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chars: 1200,
            overlap_chars: 150,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkCandidate {
    pub ordinal: u32,
    pub content: String,
    pub content_hash: String,
    pub metadata: serde_json::Value,
}

pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn split_paragraphs(text: &str) -> Vec<&str> {
    text.split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Split free text into overlapping chunks. Paragraph boundaries are preferred
/// so chunks keep coherent context; oversized paragraphs are hard-split.
pub fn chunk_text(text: &str, section: Option<&str>, config: &ChunkConfig) -> Vec<ChunkCandidate> {
    let paragraphs = split_paragraphs(text);
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut merged: Vec<String> = Vec::new();
    let mut current = String::new();
    for para in paragraphs {
        if para.len() > config.max_chars {
            if !current.is_empty() {
                merged.push(std::mem::take(&mut current));
            }
            let mut rest = para;
            while rest.len() > config.max_chars {
                let mut split_at = config.max_chars.min(rest.len());
                while split_at > 0 && !rest.is_char_boundary(split_at) {
                    split_at -= 1;
                }
                if split_at == 0 {
                    split_at = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                }
                merged.push(rest[..split_at].trim().to_string());
                rest = rest[split_at..].trim_start();
            }
            if !rest.is_empty() {
                current.push_str(rest);
            }
            continue;
        }
        if !current.is_empty() && current.len() + 2 + para.len() > config.max_chars {
            merged.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    if !current.is_empty() {
        merged.push(current);
    }

    let mut out = Vec::with_capacity(merged.len());
    let mut carry_suffix = String::new();
    for (ordinal, piece) in merged.into_iter().enumerate() {
        let mut content = String::new();
        if !carry_suffix.is_empty() && !out.is_empty() {
            content.push_str(&carry_suffix);
            content.push_str("\n\n");
        }
        content.push_str(&piece);

        carry_suffix = tail_chars(&piece, config.overlap_chars);

        let metadata = serde_json::json!({
            "kind": "prose",
            "section": section,
            "char_length": content.len(),
        });
        out.push(ChunkCandidate {
            ordinal: ordinal as u32,
            content_hash: hash_content(&content),
            content,
            metadata,
        });
    }
    out
}

fn tail_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

/// Render one structured record as a chunk, preserving field names and the
/// record lineage so table rows keep cell-level provenance.
pub fn chunk_record(
    ordinal: u32,
    lineage_ref: &str,
    fields: &std::collections::HashMap<String, serde_json::Value>,
) -> ChunkCandidate {
    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort();

    let mut lines = Vec::with_capacity(keys.len());
    for key in &keys {
        let rendered = match fields.get(key.as_str()) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        lines.push(format!("{}: {}", key, rendered));
    }
    let content = lines.join("\n");

    let metadata = serde_json::json!({
        "kind": "record",
        "lineage_ref": lineage_ref,
        "field_names": keys,
    });
    ChunkCandidate {
        ordinal,
        content_hash: hash_content(&content),
        content,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_chunk_text_short_single_chunk() {
        let text = "First paragraph.\n\nSecond paragraph.";
        let chunks = chunk_text(text, None, &ChunkConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].ordinal, 0);
        assert!(chunks[0].content.contains("First paragraph."));
        assert!(chunks[0].content.contains("Second paragraph."));
        assert_eq!(chunks[0].metadata["kind"], "prose");
    }

    #[test]
    fn test_chunk_text_respects_max_chars_with_overlap() {
        let para = "abcdefghij ".repeat(60);
        let text = format!("{}\n\n{}\n\n{}", para, para, para);
        let config = ChunkConfig {
            max_chars: 500,
            overlap_chars: 50,
        };
        let chunks = chunk_text(&text, Some("intro"), &config);
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(
                c.content.len() <= 500 + 50 + 2,
                "chunk too long: {}",
                c.content.len()
            );
            assert_eq!(c.metadata["section"], "intro");
        }
        // Ordinals are sequential and hashes are stable
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.ordinal, i as u32);
            assert_eq!(c.content_hash, hash_content(&c.content));
        }
    }

    #[test]
    fn test_chunk_text_empty_returns_none() {
        let chunks = chunk_text("   \n\n  ", None, &ChunkConfig::default());
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_record_preserves_fields_and_lineage() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), serde_json::json!("Ada"));
        fields.insert("age".to_string(), serde_json::json!(36));
        let candidate = chunk_record(7, "row-3", &fields);
        assert_eq!(candidate.ordinal, 7);
        assert!(candidate.content.contains("name: Ada"));
        assert!(candidate.content.contains("age: 36"));
        assert_eq!(candidate.metadata["kind"], "record");
        assert_eq!(candidate.metadata["lineage_ref"], "row-3");
    }
}
