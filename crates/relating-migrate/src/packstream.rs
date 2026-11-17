use crate::error::MigrateError;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct BoltNode {
    pub id: i64,
    pub labels: Vec<String>,
    pub props: BTreeMap<String, BoltValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoltRelationship {
    pub id: i64,
    pub start: i64,
    pub end: i64,
    pub rel_type: String,
    pub props: BTreeMap<String, BoltValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoltValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<BoltValue>),
    Map(BTreeMap<String, BoltValue>),
    Node(BoltNode),
    Relationship(BoltRelationship),
    Struct { tag: u8, fields: Vec<BoltValue> },
}

impl BoltValue {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            BoltValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            BoltValue::String(s) => Some(s),
            _ => None,
        }
    }
}

pub fn json_to_bolt(value: &serde_json::Value) -> BoltValue {
    match value {
        serde_json::Value::Null => BoltValue::Null,
        serde_json::Value::Bool(b) => BoltValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                BoltValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                BoltValue::Float(f)
            } else {
                BoltValue::Null
            }
        }
        serde_json::Value::String(s) => BoltValue::String(s.clone()),
        serde_json::Value::Array(items) => {
            BoltValue::List(items.iter().map(json_to_bolt).collect())
        }
        serde_json::Value::Object(map) => BoltValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_bolt(v)))
                .collect(),
        ),
    }
}

pub fn bolt_to_json(value: &BoltValue) -> serde_json::Value {
    match value {
        BoltValue::Null => serde_json::Value::Null,
        BoltValue::Bool(b) => serde_json::Value::Bool(*b),
        BoltValue::Int(n) => serde_json::Value::Number((*n).into()),
        BoltValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        BoltValue::String(s) => serde_json::Value::String(s.clone()),
        BoltValue::Bytes(bytes) => {
            serde_json::Value::String(bytes.iter().map(|b| format!("{:02x}", b)).collect())
        }
        BoltValue::List(items) => {
            serde_json::Value::Array(items.iter().map(bolt_to_json).collect())
        }
        BoltValue::Map(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), bolt_to_json(v)))
                .collect(),
        ),
        BoltValue::Node(node) => serde_json::json!({
            "_element": "node",
            "id": node.id,
            "labels": node.labels,
            "properties": serde_json::Value::Object(
                node.props.iter().map(|(k, v)| (k.clone(), bolt_to_json(v))).collect()
            ),
        }),
        BoltValue::Relationship(rel) => serde_json::json!({
            "_element": "relationship",
            "id": rel.id,
            "start": rel.start,
            "end": rel.end,
            "type": rel.rel_type,
            "properties": serde_json::Value::Object(
                rel.props.iter().map(|(k, v)| (k.clone(), bolt_to_json(v))).collect()
            ),
        }),
        BoltValue::Struct { tag, fields } => serde_json::json!({
            "_element": "struct",
            "tag": tag,
            "fields": fields.iter().map(bolt_to_json).collect::<Vec<_>>(),
        }),
    }
}

fn encode_int(n: i64, out: &mut Vec<u8>) {
    if (-16..=127).contains(&n) {
        out.push(n as u8);
    } else if (-128..=127).contains(&n) {
        out.push(0xC8);
        out.push(n as u8);
    } else if (-32768..=32767).contains(&n) {
        out.push(0xC9);
        out.extend_from_slice(&(n as i16).to_be_bytes());
    } else if (-2147483648..=2147483647).contains(&n) {
        out.push(0xCA);
        out.extend_from_slice(&(n as i32).to_be_bytes());
    } else {
        out.push(0xCB);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn encode_len_tiny(prefix: u8, len: usize, out: &mut Vec<u8>) -> Result<(), MigrateError> {
    if len < 16 {
        out.push(prefix | len as u8);
        Ok(())
    } else if len <= u8::MAX as usize {
        out.push(prefix + 0x50);
        out.push(len as u8);
        Ok(())
    } else if len <= u16::MAX as usize {
        out.push(prefix + 0x51);
        out.extend_from_slice(&(len as u16).to_be_bytes());
        Ok(())
    } else if len <= u32::MAX as usize {
        out.push(prefix + 0x52);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        Ok(())
    } else {
        Err(MigrateError::Protocol(
            "Value too large to encode".to_string(),
        ))
    }
}

fn encode_struct_header(
    field_count: usize,
    tag: u8,
    out: &mut Vec<u8>,
) -> Result<(), MigrateError> {
    if field_count < 16 {
        out.push(0xB0 | field_count as u8);
    } else if field_count <= u8::MAX as usize {
        out.push(0xDC);
        out.push(field_count as u8);
    } else if field_count <= u16::MAX as usize {
        out.push(0xDD);
        out.extend_from_slice(&(field_count as u16).to_be_bytes());
    } else {
        return Err(MigrateError::Protocol(
            "Struct too large to encode".to_string(),
        ));
    }
    out.push(tag);
    Ok(())
}

pub fn encode_value(value: &BoltValue, out: &mut Vec<u8>) -> Result<(), MigrateError> {
    match value {
        BoltValue::Null => out.push(0xC0),
        BoltValue::Bool(false) => out.push(0xC2),
        BoltValue::Bool(true) => out.push(0xC3),
        BoltValue::Float(f) => {
            out.push(0xC1);
            out.extend_from_slice(&f.to_be_bytes());
        }
        BoltValue::Int(n) => encode_int(*n, out),
        BoltValue::String(s) => {
            encode_len_tiny(0x80, s.len(), out)?;
            out.extend_from_slice(s.as_bytes());
        }
        BoltValue::Bytes(bytes) => {
            // PackStream has no tiny-bytes form: always 0xCC/0xCD/0xCE.
            let len = bytes.len();
            if len <= u8::MAX as usize {
                out.push(0xCC);
                out.push(len as u8);
            } else if len <= u16::MAX as usize {
                out.push(0xCD);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            } else if len <= u32::MAX as usize {
                out.push(0xCE);
                out.extend_from_slice(&(len as u32).to_be_bytes());
            } else {
                return Err(MigrateError::Protocol(
                    "Bytes too large to encode".to_string(),
                ));
            }
            out.extend_from_slice(bytes);
        }
        BoltValue::List(items) => {
            encode_len_tiny(0x90, items.len(), out)?;
            for item in items {
                encode_value(item, out)?;
            }
        }
        BoltValue::Map(map) => {
            encode_len_tiny(0xA0, map.len(), out)?;
            for (k, v) in map {
                encode_value(&BoltValue::String(k.clone()), out)?;
                encode_value(v, out)?;
            }
        }
        BoltValue::Node(node) => {
            encode_struct_header(3, 0x4E, out)?;
            encode_value(&BoltValue::Int(node.id), out)?;
            encode_value(
                &BoltValue::List(
                    node.labels
                        .iter()
                        .map(|l| BoltValue::String(l.clone()))
                        .collect(),
                ),
                out,
            )?;
            encode_value(&BoltValue::Map(node.props.clone()), out)?;
        }
        BoltValue::Relationship(rel) => {
            encode_struct_header(5, 0x52, out)?;
            encode_value(&BoltValue::Int(rel.id), out)?;
            encode_value(&BoltValue::Int(rel.start), out)?;
            encode_value(&BoltValue::Int(rel.end), out)?;
            encode_value(&BoltValue::String(rel.rel_type.clone()), out)?;
            encode_value(&BoltValue::Map(rel.props.clone()), out)?;
        }
        BoltValue::Struct { tag, fields } => {
            encode_struct_header(fields.len(), *tag, out)?;
            for field in fields {
                encode_value(field, out)?;
            }
        }
    }
    Ok(())
}

pub fn encode_message(tag: u8, fields: &[BoltValue]) -> Result<Vec<u8>, MigrateError> {
    let mut out = Vec::new();
    encode_struct_header(fields.len(), tag, &mut out)?;
    for field in fields {
        encode_value(field, &mut out)?;
    }
    Ok(out)
}

struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_byte(&mut self) -> Result<u8, MigrateError> {
        if self.pos >= self.buf.len() {
            return Err(MigrateError::Protocol("Truncated Bolt message".to_string()));
        }
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], MigrateError> {
        if self.pos + n > self.buf.len() {
            return Err(MigrateError::Protocol("Truncated Bolt message".to_string()));
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_u16(&mut self) -> Result<usize, MigrateError> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]) as usize)
    }

    fn read_u32(&mut self) -> Result<usize, MigrateError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    fn decode_value(&mut self) -> Result<BoltValue, MigrateError> {
        let marker = self.read_byte()?;
        match marker {
            0xC0 => Ok(BoltValue::Null),
            0xC2 => Ok(BoltValue::Bool(false)),
            0xC3 => Ok(BoltValue::Bool(true)),
            0xC1 => {
                let b = self.read_bytes(8)?;
                let arr: [u8; 8] = b
                    .try_into()
                    .map_err(|_| MigrateError::Protocol("Invalid float encoding".to_string()))?;
                Ok(BoltValue::Float(f64::from_be_bytes(arr)))
            }
            0xC8 => Ok(BoltValue::Int(self.read_byte()? as i8 as i64)),
            0xC9 => {
                let b = self.read_bytes(2)?;
                Ok(BoltValue::Int(i16::from_be_bytes([b[0], b[1]]) as i64))
            }
            0xCA => {
                let b = self.read_bytes(4)?;
                Ok(BoltValue::Int(
                    i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64
                ))
            }
            0xCB => {
                let b = self.read_bytes(8)?;
                let arr: [u8; 8] = b
                    .try_into()
                    .map_err(|_| MigrateError::Protocol("Invalid int encoding".to_string()))?;
                Ok(BoltValue::Int(i64::from_be_bytes(arr)))
            }
            0x00..=0x7F => Ok(BoltValue::Int(marker as i64)),
            0xF0..=0xFF => Ok(BoltValue::Int(marker as i8 as i64)),
            0x80..=0x8F => self.decode_string((marker & 0x0F) as usize),
            0xD0 => {
                let len = self.read_byte()? as usize;
                self.decode_string(len)
            }
            0xD1 => {
                let len = self.read_u16()?;
                self.decode_string(len)
            }
            0xD2 => {
                let len = self.read_u32()?;
                self.decode_string(len)
            }
            0x90..=0x9F => self.decode_list((marker & 0x0F) as usize),
            0xD4 => {
                let len = self.read_byte()? as usize;
                self.decode_list(len)
            }
            0xD5 => {
                let len = self.read_u16()?;
                self.decode_list(len)
            }
            0xD6 => {
                let len = self.read_u32()?;
                self.decode_list(len)
            }
            0xA0..=0xAF => self.decode_map((marker & 0x0F) as usize),
            0xD8 => {
                let len = self.read_byte()? as usize;
                self.decode_map(len)
            }
            0xD9 => {
                let len = self.read_u16()?;
                self.decode_map(len)
            }
            0xDA => {
                let len = self.read_u32()?;
                self.decode_map(len)
            }
            0xCC => {
                let len = self.read_byte()? as usize;
                Ok(BoltValue::Bytes(self.read_bytes(len)?.to_vec()))
            }
            0xCD => {
                let len = self.read_u16()?;
                Ok(BoltValue::Bytes(self.read_bytes(len)?.to_vec()))
            }
            0xCE => {
                let len = self.read_u32()?;
                Ok(BoltValue::Bytes(self.read_bytes(len)?.to_vec()))
            }
            0xB0..=0xBF => self.decode_struct((marker & 0x0F) as usize),
            0xDC => {
                let len = self.read_byte()? as usize;
                self.decode_struct(len)
            }
            0xDD => {
                let len = self.read_u16()?;
                self.decode_struct(len)
            }
            _ => Err(MigrateError::Protocol(format!(
                "Unknown PackStream marker {:#04x}",
                marker
            ))),
        }
    }

    fn decode_string(&mut self, len: usize) -> Result<BoltValue, MigrateError> {
        let bytes = self.read_bytes(len)?;
        let s = std::str::from_utf8(bytes)
            .map_err(|_| MigrateError::Protocol("Invalid UTF-8 in string".to_string()))?;
        Ok(BoltValue::String(s.to_string()))
    }

    fn decode_list(&mut self, len: usize) -> Result<BoltValue, MigrateError> {
        let mut items = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            items.push(self.decode_value()?);
        }
        Ok(BoltValue::List(items))
    }

    fn decode_map(&mut self, len: usize) -> Result<BoltValue, MigrateError> {
        let mut map = BTreeMap::new();
        for _ in 0..len {
            let key = self.decode_value()?;
            let value = self.decode_value()?;
            match key {
                BoltValue::String(k) => {
                    map.insert(k, value);
                }
                _ => {
                    return Err(MigrateError::Protocol(
                        "Map key is not a string".to_string(),
                    ))
                }
            }
        }
        Ok(BoltValue::Map(map))
    }

    fn decode_struct(&mut self, field_count: usize) -> Result<BoltValue, MigrateError> {
        let tag = self.read_byte()?;
        let mut fields = Vec::with_capacity(field_count.min(64));
        for _ in 0..field_count {
            fields.push(self.decode_value()?);
        }
        match (tag, fields.as_slice()) {
            (0x4E, [BoltValue::Int(id), BoltValue::List(labels), BoltValue::Map(props)]) => {
                let mut label_strs = Vec::with_capacity(labels.len());
                for label in labels {
                    match label {
                        BoltValue::String(s) => label_strs.push(s.clone()),
                        _ => {
                            return Err(MigrateError::Protocol(
                                "Node label is not a string".to_string(),
                            ))
                        }
                    }
                }
                Ok(BoltValue::Node(BoltNode {
                    id: *id,
                    labels: label_strs,
                    props: props.clone(),
                }))
            }
            (
                0x52,
                [BoltValue::Int(id), BoltValue::Int(start), BoltValue::Int(end), BoltValue::String(rel_type), BoltValue::Map(props)],
            ) => Ok(BoltValue::Relationship(BoltRelationship {
                id: *id,
                start: *start,
                end: *end,
                rel_type: rel_type.clone(),
                props: props.clone(),
            })),
            _ => Ok(BoltValue::Struct { tag, fields }),
        }
    }
}

pub fn decode_message(buf: &[u8]) -> Result<(u8, Vec<BoltValue>), MigrateError> {
    let mut decoder = Decoder::new(buf);
    let marker = decoder.read_byte()?;
    let field_count = match marker {
        0xB0..=0xBF => (marker & 0x0F) as usize,
        0xDC => decoder.read_byte()? as usize,
        0xDD => decoder.read_u16()?,
        _ => {
            return Err(MigrateError::Protocol(format!(
                "Expected struct message, got marker {:#04x}",
                marker
            )))
        }
    };
    let tag = decoder.read_byte()?;
    let mut fields = Vec::with_capacity(field_count.min(64));
    for _ in 0..field_count {
        fields.push(decoder.decode_value()?);
    }
    Ok((tag, fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: &BoltValue) -> BoltValue {
        let mut buf = Vec::new();
        encode_value(value, &mut buf).unwrap();
        let mut decoder = Decoder::new(&buf);
        decoder.decode_value().unwrap()
    }

    #[test]
    fn test_round_trip_scalars() {
        assert_eq!(round_trip(&BoltValue::Null), BoltValue::Null);
        assert_eq!(round_trip(&BoltValue::Bool(true)), BoltValue::Bool(true));
        assert_eq!(round_trip(&BoltValue::Bool(false)), BoltValue::Bool(false));
        assert_eq!(round_trip(&BoltValue::Float(3.25)), BoltValue::Float(3.25));
        assert_eq!(
            round_trip(&BoltValue::String("hello".to_string())),
            BoltValue::String("hello".to_string())
        );
        assert_eq!(
            round_trip(&BoltValue::Bytes(vec![1, 2, 3])),
            BoltValue::Bytes(vec![1, 2, 3])
        );
    }

    #[test]
    fn test_round_trip_int_boundaries() {
        for n in [
            -17i64,
            -16,
            -1,
            0,
            1,
            127,
            128,
            255,
            -128,
            -129,
            32767,
            32768,
            -32768,
            -32769,
            2147483647,
            2147483648,
            i64::MIN,
            i64::MAX,
        ] {
            assert_eq!(round_trip(&BoltValue::Int(n)), BoltValue::Int(n), "n={}", n);
        }
    }

    #[test]
    fn test_tiny_int_single_byte() {
        let mut buf = Vec::new();
        encode_value(&BoltValue::Int(42), &mut buf).unwrap();
        assert_eq!(buf, vec![42]);
        let mut buf = Vec::new();
        encode_value(&BoltValue::Int(-5), &mut buf).unwrap();
        assert_eq!(buf, vec![0xFB]);
    }

    #[test]
    fn test_round_trip_nested_collections() {
        let mut inner = BTreeMap::new();
        inner.insert("a".to_string(), BoltValue::Int(1));
        inner.insert(
            "b".to_string(),
            BoltValue::List(vec![BoltValue::Null, BoltValue::Bool(true)]),
        );
        let value = BoltValue::Map(inner);
        assert_eq!(round_trip(&value), value);

        let long = "x".repeat(500);
        assert_eq!(
            round_trip(&BoltValue::String(long.clone())),
            BoltValue::String(long)
        );
    }

    #[test]
    fn test_decode_node_struct() {
        // struct(3, 'N') { id: 7, labels: ["Person"], props: {name: "Ada"} }
        let mut buf = vec![0xB3, 0x4E, 0x07, 0x91, 0x86];
        buf.extend_from_slice(b"Person");
        buf.extend_from_slice(&[0xA1, 0x84]);
        buf.extend_from_slice(b"name");
        buf.extend_from_slice(&[0x83]);
        buf.extend_from_slice(b"Ada");
        let mut decoder = Decoder::new(&buf);
        match decoder.decode_value().unwrap() {
            BoltValue::Node(node) => {
                assert_eq!(node.id, 7);
                assert_eq!(node.labels, vec!["Person".to_string()]);
                assert_eq!(
                    node.props.get("name"),
                    Some(&BoltValue::String("Ada".to_string()))
                );
            }
            other => panic!("expected node, got {:?}", other),
        }
    }

    #[test]
    fn test_message_encode_decode() {
        let fields = vec![
            BoltValue::String("RETURN 1".to_string()),
            BoltValue::Map(BTreeMap::new()),
        ];
        let bytes = encode_message(0x10, &fields).unwrap();
        let (tag, decoded) = decode_message(&bytes).unwrap();
        assert_eq!(tag, 0x10);
        assert_eq!(decoded, fields);
    }

    #[test]
    fn test_truncated_input_errors() {
        let mut decoder = Decoder::new(&[0xD1, 0x00]);
        assert!(decoder.decode_value().is_err());
        assert!(decode_message(&[0xB1]).is_err());
    }

    #[test]
    fn test_json_conversions() {
        let json = serde_json::json!({"name": "Ada", "age": 36, "tags": ["a", "b"]});
        let bolt = json_to_bolt(&json);
        let back = bolt_to_json(&bolt);
        assert_eq!(back, json);
    }
}
