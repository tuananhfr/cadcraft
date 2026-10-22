//! Profile layers and how they combine: merge patches applied from Platform to Session, with policy
//! locks that later levels cannot override.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::schema::WorkspaceLayout;

/// Who a layer belongs to, from the most general to the most specific (spec §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Platform,
    Product,
    Tenant,
    Company,
    Discipline,
    Project,
    User,
    Device,
    Session,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub level: Level,
    /// Merge patch over the `WorkspaceLayout` JSON; see [`effective`] for the keyed arrays.
    #[serde(default)]
    pub patch: Value,
    /// JSON Pointers (keyed-array elements addressed by id) that later levels cannot change.
    #[serde(default)]
    pub locks: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effective {
    pub layout: WorkspaceLayout,
    /// Locked pointer to the level that locked it.
    pub locked: BTreeMap<String, Level>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyBlock {
    /// The lock that refuses the change.
    pub pointer: String,
    pub level: Level,
}

/// Arrays whose elements are matched by their `id` instead of their index.
pub const KEYED_ARRAYS: [&str; 3] = ["/toolbars", "/panels", "/ribbon/tabs"];
/// In a keyed-array element patch, `"$delete": true` removes that element.
pub const DELETE_KEY: &str = "$delete";
/// Bounds work on hostile policy files and patches.
const MAX_LOCKS: usize = 256;
const MAX_DEPTH: usize = 64;
const MAX_KEYED_ELEMENTS: usize = 500;

/// Merges `layers` in level order (stable for equal levels) over an empty layout.
///
/// Each patch is an RFC 7396 merge patch over the `WorkspaceLayout` JSON, except for the arrays in
/// [`KEYED_ARRAYS`] (`toolbars`, `panels`, `ribbon.tabs`): their patch is a list of element
/// patches merged into the element with the same `id` (appended when new, removed by
/// `"$delete": true`). When an element patch list names every resulting element, the result
/// also takes the list's order. A lock is the pointer of a field or of a keyed element
/// (`/toolbars/draw`); once a layer locks it, patches from later levels never change it or
/// anything under it. A layer whose result no longer reads as a layout is skipped whole.
pub fn effective(layers: &[Layer]) -> Effective {
    let mut ordered: Vec<&Layer> = layers.iter().collect();
    ordered.sort_by_key(|layer| layer.level);
    let mut value = Value::Object(Map::new());
    let mut locked: BTreeMap<String, Level> = BTreeMap::new();
    for layer in ordered {
        if layer.patch.is_object() {
            let locks: Vec<&str> = locked.iter().filter(|(_, level)| **level < layer.level).map(|(pointer, _)| pointer.as_str()).collect();
            let mut next = value.clone();
            merge_at(&mut next, &layer.patch, "", &locks, 0);
            if WorkspaceLayout::deserialize(&next).is_ok() {
                value = next;
            }
        }
        for lock in &layer.locks {
            if locked.len() >= MAX_LOCKS {
                break;
            }
            if lock.is_empty() || lock.starts_with('/') {
                locked.entry(lock.clone()).or_insert(layer.level);
            }
        }
    }
    let mut layout = WorkspaceLayout::deserialize(&value).unwrap_or_default();
    crate::shortcuts::remove_reserved(&mut layout.shortcuts);
    Effective { layout, locked }
}

/// Whether the user may change `pointer`: a lock on it, above it, or below it (a replacement would
/// carry the locked child along) refuses it.
pub fn check(eff: &Effective, pointer: &str) -> Result<(), PolicyBlock> {
    match eff.locked.iter().find(|(lock, _)| {
        let lock = [lock.as_str()];
        is_locked(pointer, &lock) || has_lock_below(pointer, &lock)
    }) {
        Some((lock, level)) => Err(PolicyBlock { pointer: lock.clone(), level: *level }),
        None => Ok(()),
    }
}

/// The smallest patch that turns `base` into `now` under [`effective`]'s rules.
pub fn diff_patch(base: &WorkspaceLayout, now: &WorkspaceLayout) -> Value {
    let (Ok(base), Ok(now)) = (serde_json::to_value(base), serde_json::to_value(now)) else { return Value::Object(Map::new()) };
    diff_value(&base, &now, "", 0).filter(Value::is_object).unwrap_or_else(|| Value::Object(Map::new()))
}

/// `layout` with `patch` merged in; a patch that would break the layout leaves it unchanged.
pub fn apply_patch(layout: &WorkspaceLayout, patch: &Value) -> WorkspaceLayout {
    apply_patch_locked(layout, patch, &BTreeMap::new())
}

/// Like [`apply_patch`], but parts of `patch` under any of `locked` are left out.
pub fn apply_patch_locked(layout: &WorkspaceLayout, patch: &Value, locked: &BTreeMap<String, Level>) -> WorkspaceLayout {
    let locks: Vec<&str> = locked.keys().map(String::as_str).collect();
    merged(layout, patch, &locks).unwrap_or_else(|_| layout.clone())
}

/// `layout` with `patch` merged in, or why the result is not a layout.
pub fn try_apply_patch(layout: &WorkspaceLayout, patch: &Value) -> Result<WorkspaceLayout, String> {
    merged(layout, patch, &[])
}

fn merged(layout: &WorkspaceLayout, patch: &Value, locks: &[&str]) -> Result<WorkspaceLayout, String> {
    if !patch.is_object() {
        return Err("a layout patch is a JSON object".into());
    }
    let mut value = serde_json::to_value(layout).map_err(|error| error.to_string())?;
    merge_at(&mut value, patch, "", locks, 0);
    WorkspaceLayout::deserialize(&value).map_err(|error| error.to_string())
}

/// The pointers a patch touches, keyed elements addressed by id; for showing what an import changes.
pub fn changed_pointers(patch: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_pointers(patch, "", &mut out, 0);
    out
}

/// Builds the patch that sets `pointer` (keyed elements addressed by id) to `value`; `null` removes it.
pub fn patch_for_pointer(pointer: &str, value: Value) -> Result<Value, String> {
    let tokens = split_pointer(pointer)?;
    if tokens.is_empty() {
        return Err("the pointer must name a field, not the whole layout".into());
    }
    let mut inner = value;
    for index in (0..tokens.len()).rev() {
        let token = tokens.get(index).cloned().unwrap_or_default();
        let parent: String = tokens.iter().take(index).map(|t| format!("/{}", escape_token(t))).collect();
        inner = if KEYED_ARRAYS.contains(&parent.as_str()) {
            let mut element = match inner {
                Value::Null => {
                    let mut removal = Map::new();
                    removal.insert(DELETE_KEY.into(), Value::Bool(true));
                    removal
                }
                Value::Object(fields) => fields,
                _ => return Err(format!("an element of {parent} is an object")),
            };
            element.insert("id".into(), Value::String(token));
            Value::Array(vec![Value::Object(element)])
        } else {
            let mut fields = Map::new();
            fields.insert(token, inner);
            Value::Object(fields)
        };
    }
    Ok(inner)
}

/// Drops `null` members from objects inside `value`. A merge patch cannot carry a null member, so
/// opaque values (the dock tree, unknown fields) are kept without them to compare equal after a merge.
pub fn strip_nulls(value: &mut Value) {
    let mut stack = vec![value];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 100_000 {
            return;
        }
        match node {
            Value::Object(fields) => {
                fields.retain(|_, member| !member.is_null());
                stack.extend(fields.values_mut());
            }
            Value::Array(items) => stack.extend(items.iter_mut()),
            _ => {}
        }
    }
}

pub(crate) fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn split_pointer(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let rest = pointer.strip_prefix('/').ok_or("a JSON Pointer starts with /")?;
    if rest.split('/').count() > MAX_DEPTH {
        return Err("the pointer is too deep".into());
    }
    Ok(rest.split('/').map(|token| token.replace("~1", "/").replace("~0", "~")).collect())
}

/// `path` is a lock or lies under one.
fn is_locked(path: &str, locks: &[&str]) -> bool {
    locks.iter().any(|lock| path.strip_prefix(lock).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')))
}

/// Some lock lies strictly under `path`.
fn has_lock_below(path: &str, locks: &[&str]) -> bool {
    locks.iter().any(|lock| lock.strip_prefix(path).is_some_and(|rest| rest.starts_with('/')))
}

fn item_id(item: &Value) -> Option<&str> {
    item.get("id").and_then(Value::as_str)
}

fn merge_at(target: &mut Value, patch: &Value, path: &str, locks: &[&str], depth: usize) {
    if depth > MAX_DEPTH || is_locked(path, locks) {
        return;
    }
    if let Value::Array(elements) = patch
        && KEYED_ARRAYS.contains(&path)
    {
        merge_keyed(target, elements, path, locks, depth);
        return;
    }
    let Value::Object(fields) = patch else {
        if !has_lock_below(path, locks) {
            *target = patch.clone();
        }
        return;
    };
    if !target.is_object() {
        if has_lock_below(path, locks) {
            return;
        }
        *target = Value::Object(Map::new());
    }
    let Value::Object(map) = target else { return };
    for (key, child_patch) in fields {
        let child = format!("{path}/{}", escape_token(key));
        if child_patch.is_null() {
            if !is_locked(&child, locks) && !has_lock_below(&child, locks) {
                map.remove(key);
            }
            continue;
        }
        let existed = map.contains_key(key);
        let slot = map.entry(key.clone()).or_insert(Value::Null);
        merge_at(slot, child_patch, &child, locks, depth + 1);
        // A refused merge must not leave a placeholder behind.
        if !existed && map.get(key).is_some_and(Value::is_null) {
            map.remove(key);
        }
    }
}

fn merge_keyed(target: &mut Value, elements: &[Value], path: &str, locks: &[&str], depth: usize) {
    if !target.is_array() {
        if has_lock_below(path, locks) {
            return;
        }
        *target = Value::Array(Vec::new());
    }
    let Value::Array(items) = target else { return };
    let mut order: Vec<&str> = Vec::new();
    for element in elements {
        let Some(id) = item_id(element) else { continue };
        order.push(id);
        let child = format!("{path}/{}", escape_token(id));
        if is_locked(&child, locks) {
            continue;
        }
        let position = items.iter().position(|item| item_id(item) == Some(id));
        if element.get(DELETE_KEY).and_then(Value::as_bool) == Some(true) {
            if let Some(at) = position
                && !has_lock_below(&child, locks)
            {
                items.remove(at);
            }
            continue;
        }
        let mut element_patch = element.clone();
        if let Some(fields) = element_patch.as_object_mut() {
            fields.remove(DELETE_KEY);
        }
        if let Some(item) = position.and_then(|at| items.get_mut(at)) {
            merge_at(item, &element_patch, &child, locks, depth + 1);
        } else if items.len() < MAX_KEYED_ELEMENTS {
            let mut item = Value::Null;
            merge_at(&mut item, &element_patch, &child, locks, depth + 1);
            if !item.is_null() {
                items.push(item);
            }
        }
    }
    let ranks: Option<Vec<usize>> = items.iter().map(|item| item_id(item).and_then(|id| order.iter().position(|named| *named == id))).collect();
    if let Some(ranks) = ranks {
        let mut ranked: Vec<(usize, Value)> = ranks.into_iter().zip(items.drain(..)).collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        items.extend(ranked.into_iter().map(|(_, item)| item));
    }
}

fn diff_value(base: &Value, now: &Value, path: &str, depth: usize) -> Option<Value> {
    if base == now {
        return None;
    }
    if depth > MAX_DEPTH {
        return Some(now.clone());
    }
    match (base, now) {
        (Value::Array(old), Value::Array(new)) if KEYED_ARRAYS.contains(&path) => Some(diff_keyed(old, new, path, depth)),
        (Value::Object(old), Value::Object(new)) => {
            let mut patch = Map::new();
            for key in old.keys().filter(|key| !new.contains_key(*key)) {
                patch.insert(key.clone(), Value::Null);
            }
            for (key, value) in new {
                let child = format!("{path}/{}", escape_token(key));
                match old.get(key) {
                    Some(previous) => {
                        if let Some(change) = diff_value(previous, value, &child, depth + 1) {
                            patch.insert(key.clone(), change);
                        }
                    }
                    None => {
                        patch.insert(key.clone(), value.clone());
                    }
                }
            }
            Some(Value::Object(patch))
        }
        _ => Some(now.clone()),
    }
}

fn diff_keyed(old: &[Value], new: &[Value], path: &str, depth: usize) -> Value {
    let new_ids: Vec<&str> = new.iter().filter_map(item_id).collect();
    let mut patches: Vec<(String, Value)> = Vec::new();
    for item in new {
        let Some(id) = item_id(item) else { continue };
        let child = format!("{path}/{}", escape_token(id));
        match old.iter().find(|previous| item_id(previous) == Some(id)) {
            None => patches.push((id.to_string(), item.clone())),
            Some(previous) => match diff_value(previous, item, &child, depth + 1) {
                Some(Value::Object(mut change)) => {
                    change.insert("id".into(), Value::String(id.to_string()));
                    patches.push((id.to_string(), Value::Object(change)));
                }
                Some(_) => patches.push((id.to_string(), item.clone())),
                None => {}
            },
        }
    }
    let removed: Vec<&str> = old.iter().filter_map(item_id).filter(|id| !new_ids.contains(id)).collect();
    // Order the merge would produce without an explicit order: survivors in old order, then additions.
    let mut merged: Vec<&str> = old.iter().filter_map(item_id).filter(|id| new_ids.contains(id)).collect();
    merged.extend(new_ids.iter().copied().filter(|id| !old.iter().any(|previous| item_id(previous) == Some(*id))));
    let mut out: Vec<Value> = if merged == new_ids {
        patches.into_iter().map(|(_, patch)| patch).collect()
    } else {
        new_ids
            .iter()
            .map(|id| {
                patches.iter().find(|(patched, _)| patched == id).map(|(_, patch)| patch.clone()).unwrap_or_else(|| {
                    let mut named = Map::new();
                    named.insert("id".into(), Value::String((*id).to_string()));
                    Value::Object(named)
                })
            })
            .collect()
    };
    for id in removed {
        let mut removal = Map::new();
        removal.insert("id".into(), Value::String(id.to_string()));
        removal.insert(DELETE_KEY.into(), Value::Bool(true));
        out.push(Value::Object(removal));
    }
    Value::Array(out)
}

fn collect_pointers(patch: &Value, path: &str, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH || out.len() >= MAX_KEYED_ELEMENTS {
        return;
    }
    match patch {
        Value::Object(fields) if !path.is_empty() || !fields.is_empty() => {
            if fields.is_empty() {
                out.push(path.to_string());
            }
            for (key, value) in fields {
                collect_pointers(value, &format!("{path}/{}", escape_token(key)), out, depth + 1);
            }
        }
        Value::Array(elements) if KEYED_ARRAYS.contains(&path) => {
            for element in elements {
                let Some(id) = item_id(element) else { continue };
                let child = format!("{path}/{}", escape_token(id));
                let fields = element.as_object().map(|fields| fields.iter().filter(|(key, _)| *key != "id").count()).unwrap_or(0);
                if element.get(DELETE_KEY).is_some() || fields == 0 {
                    out.push(child);
                } else if let Some(map) = element.as_object() {
                    for (key, value) in map.iter().filter(|(key, _)| *key != "id") {
                        collect_pointers(value, &format!("{child}/{}", escape_token(key)), out, depth + 1);
                    }
                }
            }
        }
        _ if !path.is_empty() => out.push(path.to_string()),
        _ => {}
    }
}
