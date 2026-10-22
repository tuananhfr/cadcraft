use std::collections::BTreeMap;

use cadcraft_engine::hash::sha256_hex;
use serde_json::Value;

pub const CHECKSUM_KEY: &str = "checksum";

pub fn canonical_json(v: &Value) -> String {
    let mut text = String::new();
    write_json(v, &mut text);
    text
}

fn write_json(v: &Value, text: &mut String) {
    match v {
        Value::Object(fields) => {
            // Sort explicitly because serde_json's preserve_order feature can change Map iteration.
            let sorted: BTreeMap<_, _> = fields.iter().collect();
            text.push('{');
            for (index, (key, value)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&Value::String(key.clone()).to_string());
                text.push(':');
                write_json(value, text);
            }
            text.push('}');
        }
        Value::Array(items) => {
            text.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                write_json(item, text);
            }
            text.push(']');
        }
        _ => text.push_str(&v.to_string()),
    }
}

pub fn checksum_of(v: &Value) -> String {
    let mut payload = v.clone();
    if let Some(fields) = payload.as_object_mut() {
        fields.remove(CHECKSUM_KEY);
    }
    format!("sha256:{}", sha256_hex(canonical_json(&payload).as_bytes()))
}

pub fn with_checksum(mut v: Value) -> Value {
    if v.is_object() {
        let checksum = checksum_of(&v);
        if let Some(fields) = v.as_object_mut() {
            fields.insert(CHECKSUM_KEY.into(), Value::String(checksum));
        }
    }
    v
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChecksumCheck {
    Valid,
    Missing,
    Mismatch { stored: String, actual: String },
}

pub fn verify_checksum(v: &Value) -> ChecksumCheck {
    let Some(stored) = v.get(CHECKSUM_KEY) else { return ChecksumCheck::Missing };
    let actual = checksum_of(v);
    match stored.as_str() {
        Some(stored) if stored == actual => ChecksumCheck::Valid,
        Some(stored) => ChecksumCheck::Mismatch { stored: stored.into(), actual },
        None => ChecksumCheck::Mismatch { stored: stored.to_string(), actual },
    }
}
