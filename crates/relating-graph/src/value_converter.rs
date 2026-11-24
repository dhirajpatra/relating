use relate::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn relate_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::List(list) => serde_json::Value::Array(list.iter().map(relate_to_json).collect()),
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map.iter() {
                obj.insert(k.clone(), relate_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        Value::Node(node) => {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "_type".to_string(),
                serde_json::Value::String("node".to_string()),
            );
            obj.insert("id".to_string(), serde_json::Value::Number(node.id.into()));
            obj.insert(
                "labels".to_string(),
                serde_json::Value::Array(
                    node.labels
                        .iter()
                        .map(|l| serde_json::Value::String(l.to_string()))
                        .collect(),
                ),
            );
            let mut props = serde_json::Map::new();
            for (k, v) in node.properties.iter() {
                props.insert(k.clone(), relate_to_json(v));
            }
            obj.insert("properties".to_string(), serde_json::Value::Object(props));
            serde_json::Value::Object(obj)
        }
        Value::Relationship(rel) => {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "_type".to_string(),
                serde_json::Value::String("relationship".to_string()),
            );
            obj.insert("id".to_string(), serde_json::Value::Number(rel.id.into()));
            obj.insert(
                "rel_type".to_string(),
                serde_json::Value::String(rel.rel_type.to_string()),
            );
            obj.insert(
                "start".to_string(),
                serde_json::Value::Number(rel.start.into()),
            );
            obj.insert("end".to_string(), serde_json::Value::Number(rel.end.into()));
            let mut props = serde_json::Map::new();
            for (k, v) in rel.properties.iter() {
                props.insert(k.clone(), relate_to_json(v));
            }
            obj.insert("properties".to_string(), serde_json::Value::Object(props));
            serde_json::Value::Object(obj)
        }
        Value::Vector { values, .. } => {
            serde_json::Value::Array(values.iter().map(relate_to_json).collect())
        }
        other => serde_json::Value::String(format!("{:?}", other)),
    }
}

pub fn json_to_relate(val: serde_json::Value) -> Value {
    match val {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(Arc::from(s.as_str())),
        serde_json::Value::Array(arr) => {
            Value::List(Arc::new(arr.into_iter().map(json_to_relate).collect()))
        }
        serde_json::Value::Object(obj) => {
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_relate(v));
            }
            Value::Map(Arc::new(map))
        }
    }
}
