use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// SHA-256 (lowercase hex) of the canonical JSON: sorted keys, numbers rounded to 1e-6.
pub fn input_hash<T: Serialize>(v: &T) -> String {
    let canonical = match serde_json::to_value(v) {
        Ok(value) => {
            let mut out = String::new();
            write_canonical(&value, &mut out);
            out
        }
        // Unserialisable input still hashes deterministically instead of panicking.
        Err(e) => format!("!serialize-error:{e}"),
    };
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&canonical_number(n)),
        Value::String(s) => out.push_str(&Value::String(s.clone()).to_string()),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                if let Some(item) = m.get(k) {
                    write_canonical(item, out);
                }
            }
            out.push('}');
        }
    }
}

fn canonical_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let x = n.as_f64().unwrap_or(0.0);
    let r = (x * 1e6).round() / 1e6;
    if r == 0.0 {
        return "0".to_string();
    }
    // f64 Display never uses exponent notation and drops ".0" for integral values.
    format!("{r}")
}
