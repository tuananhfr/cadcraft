use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::schema::{
    EffectiveProfile, MAX_PROFILE_BYTES, MAX_PROFILE_DEPTH, MAX_PROFILE_LOCKS, PRINT_PROFILE_SCHEMA, PrintSettings, ProfileDoc, ProfileLevel,
    ProfileLocked, platform_defaults,
};
use crate::hash::sha256_hex;
use crate::sheetplan::codes::{PROFILE_LEVEL_SKIPPED, issue};
use crate::sheetplan::ids::canonical_json;
use crate::sheetplan::queue::MAX_RETRY_LIMIT;
use crate::sheetplan::{PreflightIssue, Severity};

const MAX_PROFILE_DOCS: usize = 256;

pub fn effective(docs: &[ProfileDoc]) -> EffectiveProfile {
    effective_with_issues(docs).0
}

pub fn effective_with_issues(docs: &[ProfileDoc]) -> (EffectiveProfile, Vec<PreflightIssue>) {
    let mut settings = platform_defaults();
    let mut value = serde_json::to_value(&settings).unwrap_or_default();
    let mut locked = BTreeMap::new();
    let mut origin = BTreeMap::new();
    if let Some(fields) = value.as_object() {
        for key in fields.keys() {
            origin.insert(format!("/{}", escape_token(key)), ProfileLevel::Platform);
        }
    }
    let mut issues = Vec::new();
    let mut ordered: Vec<_> = docs.iter().take(MAX_PROFILE_DOCS).collect();
    ordered.sort_by_key(|doc| doc.level);
    for doc in ordered {
        let result = validate_doc(doc).and_then(|()| {
            let mut next = value.clone();
            let mut next_origin = origin.clone();
            if let Some(fields) = doc.settings.as_object() {
                for (key, patch) in fields {
                    let path = format!("/{}", escape_token(key));
                    let Some(map) = next.as_object_mut() else { continue };
                    if patch.is_null() {
                        if overlaps_lock(&path, &locked) {
                            continue;
                        }
                        map.remove(key);
                    } else {
                        let slot = map.entry(key.clone()).or_insert(Value::Null);
                        if !merge_at(slot, patch, &path, &locked) {
                            continue;
                        }
                    }
                    next_origin.insert(path, doc.level);
                }
            }
            let next_settings = PrintSettings::deserialize(&next).map_err(|error| error.to_string())?;
            validate_settings(&next_settings)?;
            let normalized = serde_json::to_value(&next_settings).map_err(|error| error.to_string())?;
            // Unknown keys must not affect the version or leak into the next level's settings.
            next_origin.retain(|path, _| normalized.pointer(path).is_some());
            if canonical_json(&normalized).len() > MAX_PROFILE_BYTES {
                return Err("effective settings exceed 256 KiB".into());
            }
            Ok((next_settings, normalized, next_origin))
        });
        match result {
            Ok((next_settings, next, next_origin)) => {
                settings = next_settings;
                value = next;
                origin = next_origin;
                for pointer in &doc.locks {
                    locked.entry(pointer.clone()).or_insert(doc.level);
                }
            }
            Err(reason) => {
                let reason: String = reason.chars().take(256).collect();
                issues.push(issue(Severity::Info, PROFILE_LEVEL_SKIPPED, format!("Skipped {} print profile: {reason}", doc.level), None));
            }
        }
    }
    if docs.len() > MAX_PROFILE_DOCS {
        issues.push(issue(
            Severity::Info,
            PROFILE_LEVEL_SKIPPED,
            "Print profile document count exceeds 256; remaining documents were skipped.",
            None,
        ));
    }
    let version = sha256_hex(canonical_json(&value).as_bytes()).chars().take(16).collect();
    (EffectiveProfile { settings, locked, origin, version }, issues)
}

pub fn check_change(eff: &EffectiveProfile, pointer: &str) -> Result<(), ProfileLocked> {
    for (lock, level) in &eff.locked {
        if at_or_below(pointer, lock) || at_or_below(lock, pointer) {
            return Err(ProfileLocked { pointer: lock.clone(), level: *level });
        }
    }
    Ok(())
}

pub fn naming_tokens(naming: &str) -> Result<(), String> {
    if naming.len() > MAX_PROFILE_BYTES {
        return Err("naming exceeds 256 KiB".into());
    }
    let mut rest = naming;
    while let Some(index) = rest.find(['{', '}']) {
        let tail = rest.get(index..).ok_or("invalid naming token")?;
        let token = tail.strip_prefix('{').ok_or("unmatched closing brace in naming")?;
        let (token, remaining) = token.split_once('}').ok_or("unclosed naming token")?;
        if !matches!(token, "stem" | "sheet_id" | "order" | "part") {
            return Err("unknown naming token; allowed tokens are {stem}, {sheet_id}, {order}, {part}".into());
        }
        rest = remaining;
    }
    Ok(())
}

fn validate_settings(settings: &PrintSettings) -> Result<(), String> {
    if !usize::try_from(settings.pages_per_file).is_ok_and(|pages| (1..=cadcraft_doc::MAX_PLOT_PAGES).contains(&pages)) {
        return Err("pages_per_file is outside the PDF page limits".into());
    }
    if settings.retry_limit > MAX_RETRY_LIMIT {
        return Err("retry_limit exceeds the supported limit".into());
    }
    naming_tokens(&settings.naming)
}

pub(super) fn validate_doc(doc: &ProfileDoc) -> Result<(), String> {
    if doc.schema_version != PRINT_PROFILE_SCHEMA {
        return Err("unsupported print profile schema_version".into());
    }
    if let Some(project_id) = &doc.project_id {
        if doc.level != ProfileLevel::Drawing {
            return Err("only drawing print profiles may carry project_id".into());
        }
        super::store::validate_id(ProfileLevel::Project, project_id)?;
    }
    if doc.locks.len() > MAX_PROFILE_LOCKS {
        return Err("print profile exceeds 256 locks".into());
    }
    let mut remaining = MAX_PROFILE_BYTES;
    consume(&mut remaining, doc.id.len())?;
    for lock in &doc.locks {
        consume(&mut remaining, lock.len())?;
        if !valid_pointer(lock) {
            return Err("invalid print profile lock pointer".into());
        }
    }
    // Check depth and a byte budget before any recursive clone or serialization of caller-owned JSON.
    validate_value(&doc.settings, 1, &mut remaining)?;
    let bytes = serde_json::to_vec(doc).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("print profile exceeds 256 KiB".into());
    }
    if !doc.settings.is_null() && !doc.settings.is_object() {
        return Err("print profile settings must be an object".into());
    }
    Ok(())
}

fn consume(remaining: &mut usize, bytes: usize) -> Result<(), String> {
    *remaining = remaining.checked_sub(bytes).ok_or("print profile exceeds 256 KiB")?;
    Ok(())
}

pub(super) fn validate_value(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), String> {
    if depth > MAX_PROFILE_DEPTH {
        return Err("print profile JSON depth exceeds 32".into());
    }
    consume(remaining, 1)?;
    match value {
        Value::String(text) => consume(remaining, text.len())?,
        Value::Array(values) => {
            for child in values {
                validate_value(child, depth + 1, remaining)?;
            }
        }
        Value::Object(fields) => {
            for (key, child) in fields {
                consume(remaining, key.len())?;
                validate_value(child, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn valid_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    let Some(rest) = pointer.strip_prefix('/') else { return false };
    if rest.split('/').count() > MAX_PROFILE_DEPTH {
        return false;
    }
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn at_or_below(path: &str, parent: &str) -> bool {
    path.strip_prefix(parent).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn overlaps_lock(path: &str, locks: &BTreeMap<String, ProfileLevel>) -> bool {
    locks.keys().any(|lock| at_or_below(path, lock) || at_or_below(lock, path))
}

fn merge_at(target: &mut Value, patch: &Value, path: &str, locks: &BTreeMap<String, ProfileLevel>) -> bool {
    if locks.keys().any(|lock| at_or_below(path, lock)) {
        return false;
    }
    let Value::Object(fields) = patch else {
        if overlaps_lock(path, locks) {
            return false;
        }
        *target = patch.clone();
        return true;
    };
    if !target.is_object() {
        if overlaps_lock(path, locks) {
            return false;
        }
        *target = Value::Object(Map::new());
    }
    let Some(map) = target.as_object_mut() else { return false };
    let mut applied = false;
    for (key, patch) in fields {
        let child = format!("{path}/{}", escape_token(key));
        if patch.is_null() {
            if !overlaps_lock(&child, locks) {
                map.remove(key);
                applied = true;
            }
        } else {
            let slot = map.entry(key.clone()).or_insert(Value::Null);
            applied |= merge_at(slot, patch, &child, locks);
        }
    }
    applied
}
