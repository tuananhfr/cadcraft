//! The `ui.workspace.*` and `ui.layout.*` commands.

use serde_json::{Value, json};

use super::history::Entry;
use super::merge::{DELETE_KEY, changed_pointers, check, diff_patch, patch_for_pointer, strip_nulls, try_apply_patch};
use super::migrate::migrate;
use super::presets::preset_json;
use super::schema::{OwnerType, ProfileStatus, WorkspaceProfile};
use super::store::MAX_USER_PROFILES;
use super::validate::{Issue, Mode, validate};
use super::{LOCKED_MESSAGE, SaveStatus, observe, rfc3339, startup};
use crate::CadApp;

pub(super) const MAX_NAME: usize = 100;

pub fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let result = match id {
        "ui.workspace.select" => select(app, params),
        "ui.workspace.reset" => reset(app),
        "ui.workspace.saveas" => save_as(app, params),
        "ui.workspace.export" => export(app, params),
        "ui.workspace.import" => import(app, params),
        "ui.workspace.restore" => restore_revision(app, params),
        "ui.workspace.status" => Ok(app.workspace_status()),
        "ui.workspace.set" => set_value(app, params),
        "ui.layout.undo" => step_history(app, true),
        "ui.layout.redo" => step_history(app, false),
        _ => return None,
    };
    Some(result)
}

/// Stores a switch or another whole-workspace change at once (D8); a failure is in `ws.status`.
fn commit_now(app: &mut CadApp) -> bool {
    app.ws.store.is_some() && app.commit_workspace_now().is_ok()
}

fn issues_text(issues: &[Issue]) -> String {
    issues
        .iter()
        .take(5)
        .map(|issue| if issue.pointer.is_empty() { issue.reason.clone() } else { format!("{}: {}", issue.pointer, issue.reason) })
        .collect::<Vec<_>>()
        .join("; ")
}

fn select(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    // The menu form (no parameters) asks the person; a script has to name the workspace.
    if params.is_null() {
        app.ws.first_run_picker_open = false;
        app.ui.dialog = Some(super::picker::PICKER_DIALOG.into());
        return Ok(Value::Null);
    }
    let ids = || app.ws.choices().iter().map(|choice| choice.id.clone()).collect::<Vec<_>>().join(", ");
    let Some(id) = params.get("id").and_then(Value::as_str).map(str::trim) else { return Err(format!("id is required: {}", ids())) };
    let next = if preset_json(id).is_some() {
        startup::fresh_user(id, &app.ws.policy())
    } else if let Some(saved) = app.ws.profiles.iter().find(|profile| profile.profile_id == id) {
        saved.clone()
    } else {
        return Err(format!("Unknown workspace: {}", id.chars().take(MAX_NAME).collect::<String>()));
    };
    observe(app, true);
    // The layer being left is kept as a snapshot before it is replaced.
    if app.ws.autosave.pending() {
        commit_now(app);
    }
    let revision = app.ws.user.revision.max(next.revision);
    let next = WorkspaceProfile { revision, ..next };
    // One undo step that also gives back which workspace this was, so a later commit cannot write
    // the old layout over another workspace.
    if app.ws.active != id || app.ws.user != next {
        app.ws.history.record_switch(app.ws.active.clone(), app.ws.user.clone());
    }
    app.ws.user = next;
    app.ws.active = id.to_string();
    app.ws.first_run_picker_open = false;
    if app.ui.dialog.as_deref() == Some(super::picker::PICKER_DIALOG) {
        app.ui.dialog = None;
    }
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    let saved = commit_now(app);
    Ok(json!({"active": app.ws.active, "base_id": app.ws.user.base_id, "saved": saved}))
}

fn reset(app: &mut CadApp) -> Result<Value, String> {
    observe(app, true);
    let below = app.ws.below_user();
    if let Some(base) = app.ws.user.base_id.as_deref().and_then(super::presets::preset) {
        app.ws.user.ui_scale = base.ui_scale;
        app.ws.user.theme = base.theme;
        app.ws.user.locale = base.locale;
    }
    if below != app.ws.user.layout {
        app.ws.history.record(std::mem::replace(&mut app.ws.user.layout, below));
    }
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    Ok(json!({"active": app.ws.active, "base_id": app.ws.user.base_id}))
}

fn new_profile_id(app: &CadApp, name: &str) -> String {
    use std::hash::{Hash, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for _ in 0..64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        crate::now_ms().to_bits().hash(&mut hasher);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed).hash(&mut hasher);
        name.hash(&mut hasher);
        let id = format!("user.{:016x}", hasher.finish());
        if !app.ws.profiles.iter().any(|profile| profile.profile_id == id) {
            return id;
        }
    }
    format!("user.{}", app.ws.profiles.len())
}

fn save_as(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    if params.is_null() {
        app.ui.dialog = Some(super::picker::SAVE_AS_DIALOG.into());
        return Ok(Value::Null);
    }
    let name = params.get("name").and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).ok_or("name is required")?;
    if name.chars().count() > MAX_NAME {
        return Err(format!("A workspace name has at most {MAX_NAME} characters"));
    }
    if app.ws.profiles.len() >= MAX_USER_PROFILES {
        return Err(format!("At most {MAX_USER_PROFILES} saved workspaces; delete one first"));
    }
    if app.ws.profiles.iter().any(|profile| profile.name.to_lowercase() == name.to_lowercase()) {
        return Err(format!("A workspace named {name} already exists"));
    }
    observe(app, true);
    let now = rfc3339(crate::now_ms());
    let profile = WorkspaceProfile {
        profile_id: new_profile_id(app, name),
        owner_type: OwnerType::User,
        status: ProfileStatus::Draft,
        name: name.to_string(),
        created_at: now.clone(),
        updated_at: now,
        ..app.ws.user.clone()
    };
    app.ws.profiles.push(profile.clone());
    app.ws.active = profile.profile_id.clone();
    app.ws.user = profile;
    app.ws.mark_changed();
    let saved = commit_now(app);
    Ok(json!({"profile_id": app.ws.active, "name": name, "saved": saved}))
}

/// The working profile as a file to share: no host-specific identity, nothing a strict import refuses.
fn export_text(app: &CadApp) -> Result<String, String> {
    let profile = WorkspaceProfile { etag: None, tenant_id: None, owner_id: None, ..app.ws.user.clone() };
    let text = serde_json::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    validate(&text, Mode::Import).map_err(|issues| format!("This workspace cannot be exported: {}", issues_text(&issues)))?;
    Ok(text)
}

fn export(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    observe(app, true);
    let text = export_text(app)?;
    let path = match params.get("path").and_then(Value::as_str) {
        Some(path) => Some(path.to_string()),
        None if params.is_null() => app.services.pick_save.as_ref().and_then(|pick| pick("Workspace.json")),
        None => None,
    };
    if let Some(path) = &path {
        write_file(path, &text)?;
    }
    Ok(json!({"text": text, "path": path}))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|error| format!("Could not write {path}: {error}"))
}

#[cfg(target_arch = "wasm32")]
fn write_file(_path: &str, _text: &str) -> Result<(), String> {
    Err("Writing a workspace file is not available in the browser; use the exported text".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| format!("Could not read {path}: {error}"))?;
    let mut text = String::new();
    // One byte over the limit is enough for validation to report the file as too large.
    file.take(super::validate::MAX_BYTES as u64 + 1).read_to_string(&mut text).map_err(|error| format!("Could not read {path}: {error}"))?;
    Ok(text)
}

#[cfg(target_arch = "wasm32")]
fn read_file(_path: &str) -> Result<String, String> {
    Err("Reading a workspace file by path is not available in the browser; pass its text".into())
}

fn import(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let text = match (params.get("text").and_then(Value::as_str), params.get("path").and_then(Value::as_str)) {
        (Some(text), _) => text.to_string(),
        (None, Some(path)) => read_file(path)?,
        (None, None) if params.is_null() => match app.services.pick_open.as_ref().and_then(|pick| pick()) {
            Some(path) => read_file(&path)?,
            None => return Ok(Value::Null),
        },
        (None, None) => return Err("text or path is required".into()),
    };
    let value = validate(&text, Mode::Import).map_err(|issues| format!("This workspace cannot be imported: {}", issues_text(&issues)))?;
    let migrated = migrate(value).map_err(|error| format!("This workspace cannot be imported: {error}"))?;
    let mut profile: WorkspaceProfile =
        serde_json::from_value(migrated.value).map_err(|error| format!("This workspace cannot be imported: {error}"))?;
    if let Some(tree) = &mut profile.layout.dock_tree {
        strip_nulls(tree);
    }
    observe(app, true);
    let mut changes = changed_pointers(&diff_patch(&app.ws.user.layout, &profile.layout));
    if profile.ui_scale != app.ws.user.ui_scale {
        changes.push("/ui_scale".into());
    }
    if profile.theme != app.ws.user.theme {
        changes.push("/theme".into());
    }
    if profile.locale != app.ws.user.locale {
        changes.push("/locale".into());
    }
    if params.get("dry_run").and_then(Value::as_bool) == Some(true) {
        return Ok(json!({"dry_run": true, "changes": changes}));
    }
    app.ws.history.record(app.ws.user.layout.clone());
    if let Some(base) = profile.base_id.as_deref().filter(|base| preset_json(base).is_some()) {
        if preset_json(&app.ws.active).is_some() {
            app.ws.active = base.to_string();
        }
        app.ws.user.base_id = Some(base.to_string());
    }
    app.ws.user.layout = profile.layout;
    app.ws.user.ui_scale = profile.ui_scale;
    app.ws.user.theme = profile.theme;
    app.ws.user.locale = profile.locale;
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    let saved = commit_now(app);
    Ok(json!({"changes": changes, "revision": app.ws.user.revision, "saved": saved}))
}

fn restore_revision(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let revision = params.get("revision").and_then(Value::as_u64).ok_or("revision is required")?;
    let raw =
        app.ws.store.as_ref().and_then(|store| store.read_snapshot(revision)).ok_or_else(|| format!("No saved workspace revision {revision}"))?;
    let set = startup::parse_set_with(&raw, &app.ws.policy())
        .map_err(|fallback| format!("Revision {revision} cannot be read: {}", startup::describe(&fallback)))?;
    observe(app, true);
    let restored = match set.user.clone() {
        Some(user) => user,
        None => match set.profiles.iter().find(|profile| profile.profile_id == set.active) {
            Some(saved) => saved.clone(),
            None => startup::fresh_user(&set.active, &app.ws.policy()),
        },
    };
    app.ws.history.record(app.ws.user.layout.clone());
    let current = app.ws.user.revision;
    app.ws.user = WorkspaceProfile { revision: current, ..restored };
    app.ws.active = set.active;
    // Workspaces saved since that revision stay; the restore is about the layout, not the list.
    for profile in set.profiles {
        if app.ws.profiles.len() < MAX_USER_PROFILES && !app.ws.profiles.iter().any(|saved| saved.profile_id == profile.profile_id) {
            app.ws.profiles.push(profile);
        }
    }
    app.ws.set_extra = set.extra;
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    if !commit_now(app) {
        return Err(match &app.ws.status {
            SaveStatus::Failed(error) => format!("Revision {revision} was restored but could not be saved: {error}"),
            _ => "No workspace storage is attached".into(),
        });
    }
    Ok(json!({"revision": app.ws.user.revision, "from": revision}))
}

fn set_value(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let pointer = params.get("pointer").and_then(Value::as_str).ok_or("pointer is required")?;
    let value = params.get("value").cloned().ok_or("value is required")?;
    observe(app, true);
    let mut next = app.ws.user.clone();
    match pointer {
        "/ui_scale" => {
            let scale = value.as_f64().filter(|scale| scale.is_finite()).ok_or("ui_scale must be a number")?;
            next.ui_scale = (scale as f32).clamp(0.75, 2.0);
        }
        "/theme" => {
            next.theme =
                optional_choice(&value, |theme| crate::theme::ThemePref::parse(theme).is_some(), "theme must be system, light, dark or null")?
        }
        "/locale" => {
            next.locale = optional_choice(&value, |code| crate::i18n::Preference::parse(code).is_some(), "locale must be a language code or null")?
        }
        _ => {
            check(&app.ws.effective, pointer).map_err(|_| LOCKED_MESSAGE.to_string())?;
            if value.get(DELETE_KEY).is_some() {
                return Err(format!("{DELETE_KEY} is not a value"));
            }
            let patch = patch_for_pointer(pointer, value)?;
            next.layout = try_apply_patch(&app.ws.user.layout, &patch).map_err(|error| format!("{pointer}: {error}"))?;
            if next.layout == app.ws.user.layout {
                return Ok(json!({"pointer": pointer, "changed": false}));
            }
        }
    }
    startup::check_profile(&next, &app.ws.policy()).map_err(|reason| format!("{pointer} cannot be stored: {reason}"))?;
    if next.layout != app.ws.user.layout {
        app.ws.history.record(app.ws.user.layout.clone());
    }
    app.ws.user = next;
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    Ok(json!({"pointer": pointer, "changed": true}))
}

fn optional_choice(value: &Value, valid: impl Fn(&str) -> bool, error: &str) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) if valid(text) => Ok(Some(text.clone())),
        _ => Err(error.into()),
    }
}

fn step_history(app: &mut CadApp, undo: bool) -> Result<Value, String> {
    observe(app, true);
    let pending = if undo { app.ws.history.peek_undo() } else { app.ws.history.peek_redo() };
    let now = match pending {
        Some(Entry::Switch { .. }) => Entry::Switch { active: app.ws.active.clone(), user: Box::new(app.ws.user.clone()) },
        _ => Entry::Layout(app.ws.user.layout.clone()),
    };
    let step = if undo { app.ws.history.undo(now) } else { app.ws.history.redo(now) };
    match step {
        None => return Err(if undo { "No layout change to undo".into() } else { "No layout change to redo".into() }),
        Some(Entry::Layout(layout)) => app.ws.user.layout = layout,
        Some(Entry::Switch { active, user }) => {
            // Revisions only grow, so the restored workspace cannot be older than a stored one.
            let revision = app.ws.user.revision.max(user.revision);
            app.ws.user = WorkspaceProfile { revision, ..*user };
            app.ws.active = active;
        }
    }
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    Ok(json!({"undo": app.ws.history.undo_len(), "redo": app.ws.history.redo_len()}))
}
