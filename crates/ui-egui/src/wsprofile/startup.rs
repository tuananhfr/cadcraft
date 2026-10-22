//! Reading the stored workspace at startup (spec §7): last good set → validate → migrate → policy
//! layers. Damaged sets restore the last good snapshot before SIMPLE; damaged bytes are kept.

use serde_json::{Map, Value};

use super::canonical::{CHECKSUM_KEY, ChecksumCheck, verify_checksum, with_checksum};
use super::device::{DeviceState, split_geometry};
use super::merge::{Layer, Level, diff_patch, effective, try_apply_patch};
use super::migrate::{MigrateError, migrate};
use super::presets::{SAFE_PRESET, preset, preset_json};
use super::schema::{OwnerType, ProfileStatus, WorkspaceLayout, WorkspaceProfile};
use super::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES, MAX_USER_PROFILES, ProfileSet, ProfileStore};
use super::validate::{Issue, Mode, validate};
use super::wire::{from_wire, to_wire};

pub const FALLBACK_MESSAGE: &str = "Workspace could not be read; using SIMPLE. The damaged copy was kept.";
pub const RESTORED_MESSAGE: &str = "Workspace could not be read; restored the last good copy (revision {revision}). The damaged copy was kept.";
pub const NOT_KEPT_MESSAGE: &str =
    "Workspace could not be read; using SIMPLE. The damaged copy could not be kept, so it is not overwritten until it can be.";
pub const POLICY_MESSAGE: &str = "The workspace policy could not be read and was ignored";
/// How many snapshots startup tries before giving up on them.
const MAX_SNAPSHOT_TRIES: usize = KEEP_SNAPSHOTS;
/// Policy files are small and few; more layers than this are ignored.
const MAX_POLICY_LAYERS: usize = 8;
/// A stored profile keeps its own layer over its base and policy here instead of a whole
/// `layout`, so later preset and policy changes still reach it.
const LAYOUT_PATCH_KEY: &str = "layout_patch";

#[derive(Clone, Debug, PartialEq)]
pub enum Fallback {
    /// Nothing stored yet (first run).
    NoProfile,
    Corrupt(String),
    NewerMajor(String),
    Invalid(Vec<Issue>),
    /// `current` could not be used; snapshot `revision` was, after `reason`.
    RestoredSnapshot {
        revision: u64,
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct Restored {
    pub user: WorkspaceProfile,
    pub layers: Vec<Layer>,
    pub fallback: Option<Fallback>,
    /// The rest of the stored set (saved profiles, active choice); default when no set can be used.
    pub set: ProfileSet,
}

/// Loads the stored set, the last valid snapshot after damage, or SIMPLE when neither can be used.
pub fn restore(store: &mut dyn ProfileStore) -> Restored {
    open(store).0
}

/// [`restore`], plus the damaged bytes when no copy of them could be kept.
pub(crate) fn open(store: &mut dyn ProfileStore) -> (Restored, Option<Vec<u8>>) {
    let policy = store.read_policy().map(|raw| policy_layers(&raw)).unwrap_or_default();
    let raw = match store.read_current() {
        Some(raw) => raw,
        None => match store.read_current_bytes() {
            None => return (fallback_to_safe(policy, Fallback::NoProfile), None),
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => {
                    let bytes = error.into_bytes();
                    let unkept = keep_copy(store, &bytes).err().map(|_| bytes);
                    let fallback = Fallback::Corrupt("the stored workspace is not UTF-8 text".into());
                    return (snapshot_or_safe(store, policy, fallback), unkept);
                }
            },
        },
    };
    match parse_set_with(&raw, &policy) {
        Ok(set) => {
            let user = working_profile(&set, &policy);
            let layers = layers_for(&user, &policy, &set.device.clone().unwrap_or_default());
            (Restored { user, layers, fallback: None, set }, None)
        }
        Err(fallback) => {
            let unkept = keep_copy(store, raw.as_bytes()).err().map(|_| raw.into_bytes());
            (snapshot_or_safe(store, policy, fallback), unkept)
        }
    }
}

fn snapshot_or_safe(store: &dyn ProfileStore, policy: Vec<Layer>, fallback: Fallback) -> Restored {
    if matches!(fallback, Fallback::Corrupt(_) | Fallback::Invalid(_)) {
        let mut revisions = store.snapshots();
        revisions.sort_unstable_by(|a, b| b.cmp(a));
        revisions.dedup();
        for revision in revisions.into_iter().filter(|revision| *revision != CORRUPT_SNAPSHOT).take(MAX_SNAPSHOT_TRIES) {
            let Some(raw) = store.read_snapshot(revision) else { continue };
            let Ok(set) = parse_set_with(&raw, &policy) else { continue };
            let user = working_profile(&set, &policy);
            let layers = layers_for(&user, &policy, &set.device.clone().unwrap_or_default());
            let fallback = Some(Fallback::RestoredSnapshot { revision, reason: describe(&fallback) });
            return Restored { user, layers, fallback, set };
        }
    }
    fallback_to_safe(policy, fallback)
}

/// Keeps a damaged set: beside the snapshots where the store can, else as snapshot
/// [`CORRUPT_SNAPSHOT`]. The current text stays in place until a commit, which waits for this.
pub(crate) fn keep_copy(store: &mut dyn ProfileStore, raw: &[u8]) -> Result<String, String> {
    let kept = store.keep_damaged(raw).or_else(|unsupported| match std::str::from_utf8(raw) {
        Ok(text) => store.push_snapshot(CORRUPT_SNAPSHOT, text).map(|()| format!("snapshot {CORRUPT_SNAPSHOT}")),
        Err(_) => Err(unsupported),
    });
    if let Err(error) = &kept {
        log::warn!("workspace: could not keep the damaged profile set: {error}");
    }
    kept
}

fn fallback_to_safe(policy: Vec<Layer>, fallback: Fallback) -> Restored {
    let user = fresh_user(SAFE_PRESET, &policy);
    let layers = layers_for(&user, &policy, &DeviceState::default());
    Restored { user, layers, fallback: Some(fallback), set: ProfileSet::default() }
}

/// Reads the current set's revision without validating its workspace layout.
pub fn stored_revision(raw: &str) -> Option<u64> {
    if raw.len() > MAX_SET_BYTES {
        return None;
    }
    let value: Value = serde_json::from_str(raw).ok()?;
    value.pointer("/user/revision").and_then(Value::as_u64)
}

/// Reads stored set text (or a bare profile, as hosts hand over migrated legacy files), with no
/// policy under the stored layers.
pub fn parse_set(raw: &str) -> Result<ProfileSet, Fallback> {
    parse_set_with(raw, &[])
}

/// [`parse_set`] with the host's policy layers under each stored profile's own layer.
pub fn parse_set_with(raw: &str, policy: &[Layer]) -> Result<ProfileSet, Fallback> {
    if raw.len() > MAX_SET_BYTES {
        return Err(Fallback::Corrupt(format!("the stored workspace is larger than {MAX_SET_BYTES} bytes")));
    }
    let value: Value = serde_json::from_str(raw).map_err(|error| Fallback::Corrupt(error.to_string()))?;
    if matches!(verify_checksum(&value), ChecksumCheck::Mismatch { .. }) {
        return Err(Fallback::Corrupt("checksum mismatch".into()));
    }
    let value = from_wire(&value);
    let Value::Object(mut root) = value else { return Err(Fallback::Corrupt("the stored workspace is not a JSON object".into())) };
    root.remove(CHECKSUM_KEY);
    let is_set = ["active", "profiles", "user"].iter().any(|key| root.contains_key(*key));
    if !is_set {
        let value = Value::Object(root);
        let device = legacy_device(&value, policy)?;
        let user = load_profile(value, policy)?;
        let active = user.base_id.clone().filter(|id| preset_json(id).is_some()).unwrap_or_else(|| SAFE_PRESET.into());
        return Ok(ProfileSet { active, user: Some(user), device, ..ProfileSet::default() });
    }
    if let Some(version) = root.get("schema_version") {
        let major =
            super::schema::version_major(version).ok_or_else(|| Fallback::Invalid(vec![issue("/schema_version", "unreadable schema_version")]))?;
        if major > super::schema::SCHEMA_VERSION {
            return Err(Fallback::NewerMajor(version.as_str().map(str::to_string).unwrap_or_else(|| version.to_string())));
        }
    }
    let active = match root.remove("active") {
        Some(Value::String(active)) => active,
        None => SAFE_PRESET.into(),
        Some(_) => return Err(Fallback::Invalid(vec![issue("/active", "must be a string")])),
    };
    let device = match root.remove("device") {
        None | Some(Value::Null) => {
            let legacy = root.get("user").filter(|value| !value.is_null()).or_else(|| {
                root.get("profiles")
                    .and_then(Value::as_array)?
                    .iter()
                    .find(|profile| profile.get("profile_id").and_then(Value::as_str) == Some(active.as_str()))
            });
            legacy.map(|value| legacy_device(value, policy)).transpose()?.flatten()
        }
        Some(value) => {
            let mut device: DeviceState =
                serde_json::from_value(value).map_err(|error| Fallback::Invalid(vec![issue("/device", &error.to_string())]))?;
            if !device.layout_patch.is_object() && !device.layout_patch.is_null() {
                return Err(Fallback::Invalid(vec![issue("/device/layout_patch", "must be an object")]));
            }
            device.layout_patch = split_geometry(device.layout_patch).1;
            Some(device)
        }
    };
    let profiles = match root.remove("profiles") {
        Some(Value::Array(items)) => {
            items.into_iter().take(MAX_USER_PROFILES).map(|item| load_profile(item, policy)).collect::<Result<Vec<_>, _>>()?
        }
        None => Vec::new(),
        Some(_) => return Err(Fallback::Invalid(vec![issue("/profiles", "must be a list")])),
    };
    let user = match root.remove("user") {
        Some(Value::Null) | None => None,
        Some(value) => Some(load_profile(value, policy)?),
    };
    root.remove("schema_version");
    Ok(ProfileSet { active, profiles, user, device, extra: root, ..ProfileSet::default() })
}

fn legacy_device(value: &Value, policy: &[Layer]) -> Result<Option<DeviceState>, Fallback> {
    let mut resolved = value.clone();
    resolve_layout_patch(&mut resolved, policy)?;
    let text = serde_json::to_string(&resolved).map_err(|error| Fallback::Corrupt(error.to_string()))?;
    validate(&text, Mode::Local).map_err(Fallback::Invalid)?;
    let migrated = migrate(resolved.clone()).map_err(|error| match error {
        MigrateError::NewerMajor(version) => Fallback::NewerMajor(version),
        other => Fallback::Invalid(vec![issue("", &other.to_string())]),
    })?;
    if let Some(device) = migrated.value.get("device") {
        return serde_json::from_value(device.clone()).map(Some).map_err(|error| Fallback::Invalid(vec![issue("/device", &error.to_string())]));
    }
    let value = &resolved;
    let scale = value.get("ui_scale").or_else(|| value.get("uiScale"));
    let layout = value.get(LAYOUT_PATCH_KEY).or_else(|| value.get("layout")).unwrap_or(value);
    let geometry = split_geometry(layout.clone()).1;
    if scale.is_none() && geometry.as_object().is_none_or(Map::is_empty) {
        return Ok(None);
    }
    let mut device: DeviceState = serde_json::from_value(serde_json::json!({"ui_scale": scale.cloned().unwrap_or(Value::from(1.0))}))
        .map_err(|error| Fallback::Invalid(vec![issue("/ui_scale", &error.to_string())]))?;
    device.layout_patch = geometry;
    Ok(Some(device))
}

fn issue(pointer: &str, reason: &str) -> Issue {
    Issue { pointer: pointer.into(), reason: reason.into() }
}

/// Turns a stored layer back into a whole layout over today's base and policy. Done before
/// validation and migration, so both see the same full profile they always did.
fn resolve_layout_patch(value: &mut Value, policy: &[Layer]) -> Result<(), Fallback> {
    let Some(fields) = value.as_object_mut() else { return Ok(()) };
    let Some(patch) = fields.remove(LAYOUT_PATCH_KEY) else { return Ok(()) };
    if !patch.is_object() {
        return Err(Fallback::Invalid(vec![issue("/layout_patch", "must be an object")]));
    }
    let below = below_user(fields.get("base_id").and_then(Value::as_str), policy);
    let layout = try_apply_patch(&below, &patch).map_err(|error| Fallback::Invalid(vec![issue("/layout_patch", &error)]))?;
    let layout = serde_json::to_value(layout).map_err(|error| Fallback::Invalid(vec![issue("/layout_patch", &error.to_string())]))?;
    fields.insert("layout".into(), layout);
    Ok(())
}

fn load_profile(mut value: Value, policy: &[Layer]) -> Result<WorkspaceProfile, Fallback> {
    resolve_layout_patch(&mut value, policy)?;
    let text = serde_json::to_string(&value).map_err(|error| Fallback::Corrupt(error.to_string()))?;
    validate(&text, Mode::Local).map_err(Fallback::Invalid)?;
    let migrated = migrate(value).map_err(|error| match error {
        MigrateError::NewerMajor(version) => Fallback::NewerMajor(version),
        other => Fallback::Invalid(vec![issue("", &other.to_string())]),
    })?;
    let mut migrated_value = migrated.value;
    if let Some(fields) = migrated_value.as_object_mut() {
        fields.remove("device");
    }
    let mut profile: WorkspaceProfile =
        serde_json::from_value(migrated_value).map_err(|error| Fallback::Invalid(vec![issue("", &error.to_string())]))?;
    let below = below_user(profile.base_id.as_deref(), policy);
    let (user, _) = split_geometry(diff_patch(&below, &profile.layout));
    profile.layout = super::merge::apply_patch(&below, &user);
    Ok(profile)
}

/// `profile` as the set stores it: its own layer over its base and `policy` in place of the layout.
fn stored_profile(profile: &WorkspaceProfile, policy: &[Layer]) -> Result<Value, String> {
    let mut value = serde_json::to_value(profile).map_err(|error| error.to_string())?;
    let patch = split_geometry(diff_patch(&below_user(profile.base_id.as_deref(), policy), &profile.layout)).0;
    let fields = value.as_object_mut().ok_or("a profile must serialise to an object")?;
    fields.remove("layout");
    fields.insert(LAYOUT_PATCH_KEY.into(), patch);
    Ok(value)
}

/// The text a commit writes for `set`.
pub(crate) fn set_text(set: &ProfileSet, policy: &[Layer]) -> Result<String, String> {
    let mut value = serde_json::to_value(set).map_err(|error| error.to_string())?;
    let fields = value.as_object_mut().ok_or("a profile set must serialise to an object")?;
    if let Some(user) = &set.user {
        fields.insert("user".into(), stored_profile(user, policy)?);
    }
    let profiles = set.profiles.iter().map(|profile| stored_profile(profile, policy)).collect::<Result<Vec<_>, _>>()?;
    fields.insert("profiles".into(), Value::Array(profiles));
    serde_json::to_string(&with_checksum(to_wire(&value))).map_err(|error| error.to_string())
}

pub(crate) fn needs_backup(raw: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
    let value = from_wire(&value);
    value.get("schema_version").is_some_and(|v| super::schema::version_major(v) == Some(1))
        || value.get("version").and_then(Value::as_u64) == Some(1)
        || value.get("user").and_then(|user| user.get("schema_version")).is_some_and(|v| super::schema::version_major(v) == Some(1))
        || value.get("profiles").and_then(Value::as_array).is_some_and(|items| {
            items.iter().any(|profile| profile.get("schema_version").is_some_and(|v| super::schema::version_major(v) == Some(1)))
        })
}

/// Refuses a profile that startup would not read back.
pub(crate) fn check_profile(profile: &WorkspaceProfile, policy: &[Layer]) -> Result<(), String> {
    load_profile(stored_profile(profile, policy)?, policy).map(|_| ()).map_err(|fallback| describe(&fallback))
}

/// A short reason for the user and the status bar.
pub fn describe(fallback: &Fallback) -> String {
    match fallback {
        Fallback::NoProfile => "nothing is stored".into(),
        Fallback::Corrupt(reason) | Fallback::RestoredSnapshot { reason, .. } => reason.clone(),
        Fallback::NewerMajor(version) => format!("it was written for a newer schema {version}"),
        Fallback::Invalid(issues) => issues
            .iter()
            .take(5)
            .map(|issue| if issue.pointer.is_empty() { issue.reason.clone() } else { format!("{}: {}", issue.pointer, issue.reason) })
            .collect::<Vec<_>>()
            .join("; "),
    }
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
        .filter_map(|item| serde_json::from_value::<Layer>(from_wire(item)).ok())
        .filter(|layer| policy_level(layer.level))
        .take(MAX_POLICY_LAYERS)
        .collect()
}

fn policy_level(level: Level) -> bool {
    !matches!(level, Level::Platform | Level::User | Level::Device | Level::Session)
}

/// Why a policy file is ignored in whole or in part, for the user to see; `None` when all of it applies.
pub fn policy_problem(raw: &str) -> Option<String> {
    if let Err(issues) = validate(raw, Mode::Local) {
        return Some(describe(&Fallback::Invalid(issues)));
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    let Some(items) = value.get("layers").and_then(Value::as_array) else { return Some("it has no list of layers".into()) };
    if items.len() > MAX_POLICY_LAYERS {
        return Some(format!("only the first {MAX_POLICY_LAYERS} layers are used"));
    }
    items.iter().enumerate().find_map(|(index, item)| match serde_json::from_value::<Layer>(from_wire(item)) {
        Err(error) => Some(format!("/layers/{index}: {error}")),
        Ok(layer) if !policy_level(layer.level) => Some(format!("/layers/{index}: a policy cannot set the {:?} level", layer.level)),
        Ok(_) => None,
    })
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
pub fn layers_for(user: &WorkspaceProfile, policy: &[Layer], device: &DeviceState) -> Vec<Layer> {
    let base_id = user.base_id.as_deref();
    let mut layers = vec![platform_layer(base_id)];
    layers.extend(policy.iter().filter(|layer| layer.level < Level::User).cloned());
    let below = effective(&layers).layout;
    layers.push(Layer { level: Level::User, patch: split_geometry(diff_patch(&below, &user.layout)).0, locks: Vec::new() });
    layers.push(Layer { level: Level::Device, patch: split_geometry(device.layout_patch.clone()).1, locks: Vec::new() });
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
