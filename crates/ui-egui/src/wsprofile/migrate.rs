//! Brings stored workspace profiles up to the current schema. Works on raw JSON so fields this
//! build does not know survive, and hands back the untouched original so an upgrade can be undone.

use std::fmt;

use cadcraft_engine::cmd::registry::RENAMED;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::schema::{OwnerType, PanelMode, SCHEMA_VERSION, WorkspaceLayout, WorkspaceProfile};

#[derive(Debug)]
pub struct Migrated {
    pub value: Value,
    /// Names of the steps that changed the document, in order; empty when it was already current.
    pub steps: Vec<&'static str>,
    /// The input exactly as received; set only when a step ran.
    pub backup: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MigrateError {
    NotAnObject,
    /// A newer major version than this build understands; startup falls back to SIMPLE.
    NewerMajor(String),
    Invalid(String),
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => f.write_str("a workspace profile must be a JSON object"),
            Self::NewerMajor(version) => write!(f, "profile schema {version} is newer than this build supports"),
            Self::Invalid(reason) => write!(f, "invalid workspace profile: {reason}"),
        }
    }
}

impl std::error::Error for MigrateError {}

/// One schema upgrade. `apply` edits the document in place; the runner sets `schema_version` to `to`.
pub struct Step {
    pub name: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub apply: fn(&mut Value) -> Result<(), MigrateError>,
}

/// Version-to-version upgrades for the live schema. Empty while 1.0.0 is the first version.
pub const STEPS: &[Step] = &[];

/// `(to_version, one-line change)`, oldest first; the last entry is `SCHEMA_VERSION`.
pub const CHANGELOG: &[(&str, &str)] = &[("1.0.0", "First profile schema; legacy workspace.json v1 files convert to a user profile over SIMPLE")];

const LEGACY_STEP: &str = "legacy_v1";
const RENAME_STEP: &str = "rename_command_ids";
/// A fixed instant keeps conversion pure; the store stamps real times when it saves.
const LEGACY_TIMESTAMP: &str = "1970-01-01T00:00:00Z";
/// `WorkspaceLayout` field names; a legacy key that collides with one cannot sit in `extra` under its own name.
const LAYOUT_KEYS: &[&str] =
    &["ribbon", "toolbars", "panels", "dock_tree", "command_line", "quick_access", "status_bar", "shortcuts", "aliases", "favorites", "toolsets"];
const LEGACY_PALETTE_PANELS: &[&str] = &["layers", "properties"];
const MAX_HISTORY_LINES: u64 = 12;

/// Upgrades `v` to the current schema, keeping every field it does not recognise.
pub fn migrate(v: Value) -> Result<Migrated, MigrateError> {
    migrate_inner(v, STEPS, RENAMED)
}

/// `migrate` with an injected step table, so a test can run multi-version upgrades without touching `SCHEMA_VERSION`.
#[doc(hidden)]
pub fn migrate_with(v: Value, steps: &[Step]) -> Result<Migrated, MigrateError> {
    migrate_inner(v, steps, RENAMED)
}

fn migrate_inner(v: Value, steps: &[Step], renamed: &[(&str, &str)]) -> Result<Migrated, MigrateError> {
    let Value::Object(root) = &v else { return Err(MigrateError::NotAnObject) };
    let legacy = is_legacy_v1(root)?;
    let original = v.clone();
    let mut value = v;
    let mut applied: Vec<&'static str> = Vec::new();

    if legacy {
        value = from_legacy_v1(&value);
        applied.push(LEGACY_STEP);
    }

    let version = schema_version(&value)?;
    let (major, ..) = parse_version(&version).ok_or_else(|| MigrateError::Invalid(format!("unreadable schema_version {version:?}")))?;
    let (current_major, ..) = parse_version(SCHEMA_VERSION).unwrap_or((1, 0, 0));
    if major > current_major {
        return Err(MigrateError::NewerMajor(version));
    }

    let mut current = version;
    // Each pass consumes one step, so a cyclic table cannot loop.
    for _ in 0..steps.len() {
        let Some(step) = steps.iter().find(|step| step.from == current) else { break };
        (step.apply)(&mut value)?;
        if let Some(root) = value.as_object_mut() {
            root.insert("schema_version".into(), Value::String(step.to.into()));
        }
        current = step.to.to_string();
        applied.push(step.name);
    }
    if parse_version(&current).is_none_or(|(major, ..)| major < current_major) {
        return Err(MigrateError::Invalid(format!("no migration path from schema {current}")));
    }

    if rename_command_ids(&mut value, renamed) {
        applied.push(RENAME_STEP);
    }

    // The result is what the app will load; refuse here rather than let a later parse fail on stored data.
    <WorkspaceProfile as Deserialize>::deserialize(&value).map_err(|error| MigrateError::Invalid(error.to_string()))?;

    let backup = (!applied.is_empty()).then_some(original);
    Ok(Migrated { value, steps: applied, backup })
}

fn is_legacy_v1(root: &Map<String, Value>) -> Result<bool, MigrateError> {
    if root.contains_key("schema_version") {
        return Ok(false);
    }
    match root.get("version") {
        None => Ok(false),
        Some(version) if version.as_u64() == Some(1) => {
            if root.get("workspace").is_some_and(Value::is_object) {
                Ok(true)
            } else {
                Err(MigrateError::Invalid("legacy workspace file has no workspace object".into()))
            }
        }
        Some(_) => Err(MigrateError::Invalid("unsupported legacy workspace version".into())),
    }
}

fn schema_version(value: &Value) -> Result<String, MigrateError> {
    match value.get("schema_version") {
        Some(Value::String(version)) => Ok(version.clone()),
        Some(_) => Err(MigrateError::Invalid("schema_version must be a string".into())),
        None => Err(MigrateError::Invalid("missing schema_version".into())),
    }
}

/// Plain `major.minor.patch`; anything else (pre-release tags, signs, extra parts) is not a version we wrote.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let number = |part: &str| (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())).then(|| part.parse::<u64>().ok()).flatten();
    let mut parts = text.split('.');
    let version = (number(parts.next()?)?, number(parts.next()?)?, number(parts.next()?)?);
    parts.next().is_none().then_some(version)
}

/// Converts the `{"version":1,"workspace":{..}}` file of the first workspace feature into a user profile on top of SIMPLE.
pub fn from_legacy_v1(workspace_json: &Value) -> Value {
    let mut layout = WorkspaceLayout::default();
    if let Some(fields) = workspace_json.get("workspace").and_then(Value::as_object) {
        for (key, value) in fields {
            if !apply_legacy_field(&mut layout, key, value) {
                let name = if LAYOUT_KEYS.contains(&key.as_str()) { format!("legacy_{key}") } else { key.clone() };
                layout.extra.insert(name, value.clone());
            }
        }
    }
    let profile = WorkspaceProfile {
        profile_id: "user.legacy-v1".into(),
        owner_type: OwnerType::User,
        name: "Imported workspace".into(),
        base_id: Some("SIMPLE".into()),
        created_at: LEGACY_TIMESTAMP.into(),
        updated_at: LEGACY_TIMESTAMP.into(),
        layout,
        ..WorkspaceProfile::default()
    };
    serde_json::to_value(profile).unwrap_or(Value::Null)
}

/// Maps one legacy key onto its closest layout field; `false` leaves the key for `extra`.
fn apply_legacy_field(layout: &mut WorkspaceLayout, key: &str, value: &Value) -> bool {
    match (key, value) {
        ("docking", docking) => {
            layout.dock_tree = (!docking.is_null()).then(|| docking.clone());
            true
        }
        ("showToolbar", Value::Bool(visible)) => match layout.toolbars.iter_mut().find(|toolbar| toolbar.id == "standard") {
            Some(toolbar) => {
                toolbar.visible = *visible;
                true
            }
            None => false,
        },
        ("showToolsets", Value::Bool(visible)) => {
            layout.toolsets.visible = *visible;
            true
        }
        ("showPalettes", Value::Bool(visible)) => {
            for panel in layout.panels.iter_mut().filter(|panel| LEGACY_PALETTE_PANELS.contains(&panel.id.as_str())) {
                panel.visible = *visible;
                if !*visible {
                    panel.mode = PanelMode::Hidden;
                }
            }
            true
        }
        ("showStatusBar", Value::Bool(visible)) => {
            layout.status_bar.visible = *visible;
            true
        }
        ("showCommandLine", Value::Bool(visible)) => {
            layout.command_line.visible = *visible;
            true
        }
        ("historyLines", Value::Number(lines)) => match lines.as_u64().filter(|lines| *lines <= MAX_HISTORY_LINES) {
            Some(lines) => {
                layout.command_line.lines = lines as u32;
                true
            }
            None => false,
        },
        ("toolsetTab", Value::String(tab)) => {
            layout.toolsets.tab = tab.clone();
            true
        }
        ("collapsedGroups", Value::Array(groups)) => {
            let names: Vec<String> = groups.iter().filter_map(|group| group.as_str().map(str::to_string)).collect();
            if names.len() != groups.len() {
                return false;
            }
            layout.toolsets.collapsed_groups = names;
            true
        }
        _ => false,
    }
}

/// Applies the engine's id renames to every place a profile names a command. Returns whether anything changed.
fn rename_command_ids(root: &mut Value, renamed: &[(&str, &str)]) -> bool {
    if renamed.is_empty() {
        return false;
    }
    // Spec-style profiles keep shortcuts and the like at the root, the model keeps them in `layout`.
    let mut changed = rename_in_container(root, renamed);
    if let Some(layout) = root.get_mut("layout") {
        changed |= rename_in_container(layout, renamed);
    }
    changed
}

fn rename_in_container(container: &mut Value, renamed: &[(&str, &str)]) -> bool {
    let mut changed = false;
    if let Some(toolbars) = container.get_mut("toolbars").and_then(Value::as_array_mut) {
        for toolbar in toolbars {
            changed |= toolbar.get_mut("commands").is_some_and(|list| rename_in_list(list, renamed));
        }
    }
    let tabs = container.get_mut("ribbon").and_then(|ribbon| ribbon.get_mut("tabs")).and_then(Value::as_array_mut);
    for tab in tabs.into_iter().flatten() {
        let groups = tab.get_mut("groups").and_then(Value::as_array_mut);
        for group in groups.into_iter().flatten() {
            changed |= group.get_mut("commands").is_some_and(|list| rename_in_list(list, renamed));
        }
    }
    for key in ["quick_access", "favorites"] {
        changed |= container.get_mut(key).is_some_and(|list| rename_in_list(list, renamed));
    }
    changed |= container.get_mut("toolsets").and_then(|toolsets| toolsets.get_mut("hidden_tools")).is_some_and(|list| rename_in_list(list, renamed));
    changed |= container.get_mut("shortcuts").is_some_and(|shortcuts| rename_keys(shortcuts, renamed));
    changed |= container.get_mut("aliases").is_some_and(|aliases| rename_values(aliases, renamed));
    changed
}

/// Follows rename chains (a to b to c); the hop bound keeps a cyclic table finite.
fn newest<'a>(id: &'a str, renamed: &[(&'a str, &'a str)]) -> &'a str {
    let mut current = id;
    for _ in 0..=renamed.len() {
        match renamed.iter().find(|(old, _)| *old == current) {
            Some((_, new)) => current = new,
            None => break,
        }
    }
    current
}

fn rename_in_list(list: &mut Value, renamed: &[(&str, &str)]) -> bool {
    let Some(items) = list.as_array_mut() else { return false };
    let mut changed = false;
    for item in items {
        if let Value::String(id) = item {
            let new = newest(id, renamed).to_string();
            if new != *id {
                *id = new;
                changed = true;
            }
        }
    }
    changed
}

fn rename_values(map: &mut Value, renamed: &[(&str, &str)]) -> bool {
    let Some(entries) = map.as_object_mut() else { return false };
    let mut changed = false;
    for value in entries.values_mut() {
        changed |= match value {
            Value::String(id) => {
                let new = newest(id, renamed).to_string();
                let differs = new != *id;
                *id = new;
                differs
            }
            _ => false,
        };
    }
    changed
}

/// A key already present under the new name wins; the old entry stays so no binding is lost.
fn rename_keys(map: &mut Value, renamed: &[(&str, &str)]) -> bool {
    let Some(entries) = map.as_object_mut() else { return false };
    let moves: Vec<(String, String)> =
        entries.keys().filter_map(|key| Some((key.clone(), newest(key, renamed).to_string())).filter(|(old, new)| old != new)).collect();
    let mut changed = false;
    for (old, new) in moves {
        if entries.contains_key(&new) {
            continue;
        }
        if let Some(value) = entries.remove(&old) {
            entries.insert(new, value);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::wsprofile::schema::Dock;

    const RENAMES: &[(&str, &str)] = &[("oldline", "line"), ("line", "line2"), ("oldrect", "rectang")];

    fn profile_using_old_ids() -> Value {
        json!({
            "schema_version": "1.0.0",
            "layout": {
                "toolbars": [{"id": "draw", "commands": ["oldline", "circle", "oldrect"]}],
                "ribbon": {"tabs": [{"id": "home", "groups": [{"id": "g", "commands": ["oldrect"]}]}]},
                "quick_access": ["oldline"],
                "favorites": ["oldrect", "pan"],
                "toolsets": {"hidden_tools": ["oldline"]},
                "shortcuts": {"oldline": "L", "pan": "P"},
                "aliases": {"ln": "oldline", "keep": "circle"}
            }
        })
    }

    #[test]
    fn rename_pass_rewrites_every_command_reference() {
        let migrated = migrate_inner(profile_using_old_ids(), &[], RENAMES).unwrap();
        assert_eq!(migrated.steps, vec![RENAME_STEP]);
        assert_eq!(migrated.backup, Some(profile_using_old_ids()));
        let layout = &migrated.value["layout"];
        // oldline follows the chain oldline -> line -> line2.
        assert_eq!(layout["toolbars"][0]["commands"], json!(["line2", "circle", "rectang"]));
        assert_eq!(layout["ribbon"]["tabs"][0]["groups"][0]["commands"], json!(["rectang"]));
        assert_eq!(layout["quick_access"], json!(["line2"]));
        assert_eq!(layout["favorites"], json!(["rectang", "pan"]));
        assert_eq!(layout["toolsets"]["hidden_tools"], json!(["line2"]));
        assert_eq!(layout["shortcuts"], json!({"line2": "L", "pan": "P"}));
        assert_eq!(layout["aliases"], json!({"ln": "line2", "keep": "circle"}));
    }

    #[test]
    fn rename_pass_is_idempotent_and_keeps_colliding_bindings() {
        let first = migrate_inner(profile_using_old_ids(), &[], RENAMES).unwrap();
        let second = migrate_inner(first.value.clone(), &[], RENAMES).unwrap();
        assert_eq!(second.value, first.value);
        assert!(second.steps.is_empty());

        let colliding = json!({"schema_version": "1.0.0", "shortcuts": {"oldrect": "R", "rectang": "Q"}});
        let migrated = migrate_inner(colliding.clone(), &[], RENAMES).unwrap();
        assert_eq!(migrated.value, colliding);
        assert!(migrated.steps.is_empty());
    }

    #[test]
    fn cyclic_rename_table_terminates() {
        let cycle: &[(&str, &str)] = &[("a", "b"), ("b", "a")];
        let input = json!({"schema_version": "1.0.0", "layout": {"favorites": ["a"]}});
        assert!(migrate_inner(input, &[], cycle).is_ok());
    }

    #[test]
    fn legacy_keys_named_like_layout_fields_are_kept_under_a_prefix() {
        let input = json!({"version": 1, "workspace": {"toolbars": "custom", "ribbon": 3}});
        let value = from_legacy_v1(&input);
        assert_eq!(value["layout"]["legacy_toolbars"], json!("custom"));
        assert_eq!(value["layout"]["legacy_ribbon"], json!(3));
        assert!(value["layout"]["toolbars"].is_array());
        assert!(migrate(input).is_ok());
    }

    #[test]
    fn legacy_conversion_of_nothing_is_plain_simple_user_layer() {
        let value = from_legacy_v1(&json!(null));
        let profile: WorkspaceProfile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.layout, WorkspaceLayout::default());
        assert_eq!(profile.base_id.as_deref(), Some("SIMPLE"));
        assert_eq!(profile.layout.command_line.dock, Dock::Bottom);
    }

    #[test]
    fn version_parser_accepts_only_plain_triples() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        for bad in ["", "1", "1.2", "1.2.3.4", "1.2.x", "+1.0.0", "-1.0.0", "1.0.0-beta", " 1.0.0", "1..0", "99999999999999999999.0.0"] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
    }
}
