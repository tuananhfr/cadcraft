//! Reading the stored workspace at startup (spec §7): last good set → validate → migrate → policy
//! layers. Anything unreadable falls back to SIMPLE and the damaged text is kept as a snapshot.

use serde_json::{Map, Value};

use super::merge::{Layer, Level, diff_patch, effective};
use super::migrate::{MigrateError, migrate};
use super::presets::{SAFE_PRESET, preset, preset_json};
use super::schema::{OwnerType, ProfileStatus, WorkspaceLayout, WorkspaceProfile};
use super::store::{CORRUPT_SNAPSHOT, MAX_SET_BYTES, MAX_USER_PROFILES, ProfileSet, ProfileStore};
use super::validate::{Issue, Mode, validate};

pub const FALLBACK_MESSAGE: &str = "Workspace could not be read; using SIMPLE. The damaged copy was kept.";
/// Policy files are small and few; more layers than this are ignored.
const MAX_POLICY_LAYERS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum Fallback {
    /// Nothing stored yet (first run).
    NoProfile,
    Corrupt(String),
    NewerMajor(String),
    Invalid(Vec<Issue>),
}

#[derive(Clone, Debug)]
pub struct Restored {
    pub user: WorkspaceProfile,
    pub layers: Vec<Layer>,
    pub fallback: Option<Fallback>,
    /// The rest of the stored set (saved profiles, active choice); default after a fallback.
    pub set: ProfileSet,
}

/// Loads the stored set, or SIMPLE when there is none or it cannot be used.
pub fn restore(store: &mut dyn ProfileStore) -> Restored {
    let policy = store.read_policy().map(|raw| policy_layers(&raw)).unwrap_or_default();
    let Some(raw) = store.read_current() else { return fallback_to_safe(policy, Fallback::NoProfile) };
    match parse_set(&raw) {
        Ok(set) => {
            let user = working_profile(&set, &policy);
            let layers = layers_for(&user, &policy);
            Restored { user, layers, fallback: None, set }
        }
        Err(fallback) => {
            // The current text stays where it is too until the next commit, so a failed copy loses nothing yet.
            if let Err(error) = store.push_snapshot(CORRUPT_SNAPSHOT, &raw) {
                log::warn!("workspace: could not keep the damaged profile set: {error}");
            }
            fallback_to_safe(policy, fallback)
        }
    }
}

fn fallback_to_safe(policy: Vec<Layer>, fallback: Fallback) -> Restored {
    let user = fresh_user(SAFE_PRESET, &policy);
    let layers = layers_for(&user, &policy);
    Restored { user, layers, fallback: Some(fallback), set: ProfileSet::default() }
}

/// Reads stored set text (or a bare profile, as hosts hand over migrated legacy files).
pub fn parse_set(raw: &str) -> Result<ProfileSet, Fallback> {
    if raw.len() > MAX_SET_BYTES {
        return Err(Fallback::Corrupt(format!("the stored workspace is larger than {MAX_SET_BYTES} bytes")));
    }
    let value: Value = serde_json::from_str(raw).map_err(|error| Fallback::Corrupt(error.to_string()))?;
    let Value::Object(mut root) = value else { return Err(Fallback::Corrupt("the stored workspace is not a JSON object".into())) };
    let is_set = ["active", "profiles", "user"].iter().any(|key| root.contains_key(*key));
    if !is_set {
        let user = load_profile(Value::Object(root))?;
        let active = user.base_id.clone().filter(|id| preset_json(id).is_some()).unwrap_or_else(|| SAFE_PRESET.into());
        return Ok(ProfileSet { active, user: Some(user), ..ProfileSet::default() });
    }
    if let Some(version) = root.get("schema_version").and_then(Value::as_str)
        && newer_major(version)
    {
        return Err(Fallback::NewerMajor(version.to_string()));
    }
    let active = match root.remove("active") {
        Some(Value::String(active)) => active,
        None => SAFE_PRESET.into(),
        Some(_) => return Err(Fallback::Invalid(vec![issue("/active", "must be a string")])),
    };
    let profiles = match root.remove("profiles") {
        Some(Value::Array(items)) => items.into_iter().take(MAX_USER_PROFILES).map(load_profile).collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
        Some(_) => return Err(Fallback::Invalid(vec![issue("/profiles", "must be a list")])),
    };
    let user = match root.remove("user") {
        Some(Value::Null) | None => None,
        Some(value) => Some(load_profile(value)?),
    };
    root.remove("schema_version");
    Ok(ProfileSet { active, profiles, user, extra: root, ..ProfileSet::default() })
}

fn issue(pointer: &str, reason: &str) -> Issue {
    Issue { pointer: pointer.into(), reason: reason.into() }
}

fn newer_major(version: &str) -> bool {
    let major = |text: &str| text.split('.').next().and_then(|part| part.parse::<u64>().ok());
    matches!((major(version), major(super::schema::SCHEMA_VERSION)), (Some(found), Some(ours)) if found > ours)
}

fn load_profile(value: Value) -> Result<WorkspaceProfile, Fallback> {
    let text = serde_json::to_string(&value).map_err(|error| Fallback::Corrupt(error.to_string()))?;
    validate(&text, Mode::Local).map_err(Fallback::Invalid)?;
    let migrated = migrate(value).map_err(|error| match error {
        MigrateError::NewerMajor(version) => Fallback::NewerMajor(version),
        other => Fallback::Invalid(vec![issue("", &other.to_string())]),
    })?;
    serde_json::from_value(migrated.value).map_err(|error| Fallback::Invalid(vec![issue("", &error.to_string())]))
}

/// Layers from a policy file; an unreadable file contributes none. Policy may not speak for the
/// platform, the user or the session.
pub fn policy_layers(raw: &str) -> Vec<Layer> {
    if let Err(issues) = validate(raw, Mode::Local) {
        log::warn!("workspace: policy ignored: {issues:?}");
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(raw) else { return Vec::new() };
    let Some(items) = value.get("layers").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| serde_json::from_value::<Layer>(item.clone()).ok())
        .filter(|layer| !matches!(layer.level, Level::Platform | Level::User | Level::Session))
        .take(MAX_POLICY_LAYERS)
        .collect()
}

/// The preset layout a base id names; SIMPLE for an unknown or missing one.
pub fn base_layout(base_id: Option<&str>) -> WorkspaceLayout {
    base_id.and_then(preset).or_else(|| preset(SAFE_PRESET)).map(|profile| profile.layout).unwrap_or_default()
}

fn platform_layer(base_id: Option<&str>) -> Layer {
    Layer {
        level: Level::Platform,
        patch: serde_json::to_value(base_layout(base_id)).unwrap_or_else(|_| Value::Object(Map::new())),
        locks: Vec::new(),
    }
}

/// The layout the user layer starts from: the base preset with policy applied.
pub fn below_user(base_id: Option<&str>, policy: &[Layer]) -> WorkspaceLayout {
    let mut layers = vec![platform_layer(base_id)];
    layers.extend(policy.iter().filter(|layer| layer.level < Level::User).cloned());
    effective(&layers).layout
}

/// Platform, policy and the user's own layer for `user`.
pub fn layers_for(user: &WorkspaceProfile, policy: &[Layer]) -> Vec<Layer> {
    let base_id = user.base_id.as_deref();
    let mut layers = vec![platform_layer(base_id)];
    layers.extend(policy.iter().filter(|layer| layer.level < Level::User).cloned());
    let below = effective(&layers).layout;
    layers.push(Layer { level: Level::User, patch: diff_patch(&below, &user.layout), locks: Vec::new() });
    layers
}

/// A working profile with an empty user layer over the preset `id`.
pub fn fresh_user(id: &str, policy: &[Layer]) -> WorkspaceProfile {
    let base = preset(id).or_else(|| preset(SAFE_PRESET)).unwrap_or_default();
    let base_id = preset_json(id).map_or(SAFE_PRESET, |_| id).to_string();
    WorkspaceProfile {
        profile_id: format!("user.{base_id}"),
        owner_type: OwnerType::User,
        status: ProfileStatus::Draft,
        name: base.name.clone(),
        revision: 0,
        layout: below_user(Some(&base_id), policy),
        base_id: Some(base_id),
        ..base
    }
}

fn working_profile(set: &ProfileSet, policy: &[Layer]) -> WorkspaceProfile {
    if let Some(user) = &set.user {
        return user.clone();
    }
    match set.profiles.iter().find(|profile| profile.profile_id == set.active) {
        Some(saved) => saved.clone(),
        None => fresh_user(&set.active, policy),
    }
}
