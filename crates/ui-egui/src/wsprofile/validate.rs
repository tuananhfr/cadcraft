//! Structural limits and payload rules checked on raw profile text before it is trusted.

use serde_json::Value;

pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_DEPTH: usize = 32;
pub const MAX_ARRAY: usize = 500;
pub const MAX_STRING: usize = 1024;
/// Stops a hostile document from producing an unbounded report.
const MAX_ISSUES: usize = 64;

const EXECUTABLE_EXTENSIONS: [&str; 11] = [".lsp", ".fas", ".vlx", ".dvb", ".arx", ".dll", ".exe", ".js", ".ps1", ".bat", ".cmd"];
const EXECUTABLE_KEYS: [&str; 7] = ["script", "lisp", "exec", "eval", "code", "onload", "startup"];
const SECRET_KEY_PARTS: [&str; 6] = ["token", "password", "secret", "cookie", "authorization", "apikey"];

/// `Local` is a profile this app wrote; `Import` is one that came from outside and must not carry code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Local,
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// RFC 6901 JSON Pointer; empty for the document as a whole.
    pub pointer: String,
    pub reason: String,
}

fn issue(pointer: &str, reason: impl Into<String>) -> Issue {
    Issue { pointer: pointer.to_string(), reason: reason.into() }
}

/// Checks `raw` against the limits (and, for `Import`, the payload rules) and returns the parsed document.
pub fn validate(raw: &str, mode: Mode) -> Result<Value, Vec<Issue>> {
    if raw.len() > MAX_BYTES {
        return Err(vec![issue("", format!("too large: {} bytes, the limit is {MAX_BYTES}", raw.len()))]);
    }
    // Lexical scan first: the parser's own recursion limit is far above ours and must not be what rejects the input.
    if nesting_exceeds_limit(raw) {
        return Err(vec![issue("", format!("too deep: more than {MAX_DEPTH} nested levels"))]);
    }
    let value: Value = serde_json::from_str(raw).map_err(|error| vec![issue("", format!("invalid JSON: {error}"))])?;
    let mut issues = Vec::new();
    if !value.is_object() {
        issues.push(issue("", "a profile must be a JSON object"));
    }
    walk(&value, "", mode, &mut issues);
    if issues.is_empty() { Ok(value) } else { Err(issues) }
}

fn nesting_exceeds_limit(raw: &str) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for byte in raw.bytes() {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

fn escape_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn walk(value: &Value, pointer: &str, mode: Mode, issues: &mut Vec<Issue>) {
    if issues.len() >= MAX_ISSUES {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_pointer = format!("{pointer}/{}", escape_token(key));
                check_key(key, &child_pointer, mode, issues);
                walk(child, &child_pointer, mode, issues);
            }
        }
        Value::Array(items) => {
            if items.len() > MAX_ARRAY {
                issues.push(issue(pointer, format!("array too long: {} elements, the limit is {MAX_ARRAY}", items.len())));
            }
            for (index, child) in items.iter().enumerate() {
                walk(child, &format!("{pointer}/{index}"), mode, issues);
            }
        }
        Value::String(text) => check_string(text, pointer, mode, issues),
        _ => {}
    }
}

fn check_key(key: &str, pointer: &str, mode: Mode, issues: &mut Vec<Issue>) {
    if key.chars().count() > MAX_STRING {
        issues.push(issue(pointer, format!("string too long: key over {MAX_STRING} characters")));
    }
    let squashed: String = key.chars().filter(|c| !matches!(c, '_' | '-' | ' ')).flat_map(char::to_lowercase).collect();
    if SECRET_KEY_PARTS.iter().any(|part| squashed.contains(part)) {
        issues.push(issue(pointer, format!("secret-like key `{key}` is not stored in a profile")));
    }
    if mode == Mode::Import && EXECUTABLE_KEYS.contains(&squashed.as_str()) {
        issues.push(issue(pointer, format!("key `{key}` could start code and is not allowed in an imported profile")));
    }
}

fn check_string(text: &str, pointer: &str, mode: Mode, issues: &mut Vec<Issue>) {
    if text.chars().count() > MAX_STRING {
        issues.push(issue(pointer, format!("string too long: over {MAX_STRING} characters")));
    }
    if mode != Mode::Import {
        return;
    }
    if has_executable_extension(text) {
        issues.push(issue(pointer, "file extension of an executable or script is not allowed in an imported profile"));
    }
    if is_absolute_path(text) {
        issues.push(issue(pointer, "absolute, UNC or file: paths are not allowed in an imported profile"));
    }
}

/// True when `.ext` ends a word, so `settings.json` does not trip on `.js`.
fn has_executable_extension(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    EXECUTABLE_EXTENSIONS.iter().any(|extension| {
        lower
            .match_indices(extension)
            .any(|(at, found)| lower.get(at + found.len()..).and_then(|rest| rest.chars().next()).is_none_or(|next| !next.is_alphanumeric()))
    })
}

fn is_absolute_path(text: &str) -> bool {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\');
    drive || text.starts_with(['/', '\\']) || text.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
}
