use serde_json::Value;

use super::types::Sheet;

pub use crate::hash::sha256_hex;

pub fn canonical_json(v: &Value) -> String {
    enum Frame<'a> {
        Value(&'a Value),
        Array(std::slice::Iter<'a, Value>, bool),
        Object(std::vec::IntoIter<(&'a String, &'a Value)>, bool),
    }

    let mut output = String::new();
    let mut stack = vec![Frame::Value(v)];
    // Requests may contain nested settings; an explicit stack avoids call-stack exhaustion.
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Value(Value::Array(values)) => {
                output.push('[');
                stack.push(Frame::Array(values.iter(), true));
            }
            Frame::Value(Value::Object(values)) => {
                output.push('{');
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by_key(|(key, _)| *key);
                stack.push(Frame::Object(entries.into_iter(), true));
            }
            Frame::Value(value) => output.push_str(&value.to_string()),
            Frame::Array(mut values, first) => {
                if let Some(value) = values.next() {
                    if !first {
                        output.push(',');
                    }
                    stack.push(Frame::Array(values, false));
                    stack.push(Frame::Value(value));
                } else {
                    output.push(']');
                }
            }
            Frame::Object(mut entries, first) => {
                if let Some((key, value)) = entries.next() {
                    if !first {
                        output.push(',');
                    }
                    output.push_str(&Value::String(key.clone()).to_string());
                    output.push(':');
                    stack.push(Frame::Object(entries, false));
                    stack.push(Frame::Value(value));
                } else {
                    output.push('}');
                }
            }
        }
    }
    output
}

pub fn job_id(source_hash: &str, request: &Value) -> String {
    let mut input = source_hash.to_string();
    input.push_str(&canonical_json(request));
    sha256_hex(input.as_bytes()).chars().take(16).collect()
}

pub fn sheet_key(s: &Sheet) -> String {
    format!("{}@{}", s.sheet_id, s.order)
}
