// The transport contract returns the complete stale revision inline for three-way merging.
#![allow(clippy::large_enum_variant, clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::conflict::{FieldConflict, Resolution, conflicts};
use super::merge::{diff_patch, try_apply_patch};
use super::schema::{OwnerType, ProfileStatus, WorkspaceLayout, WorkspaceProfile};
use super::store::{MAX_USER_PROFILES, ProfileSet};
use super::{SaveStatus, startup};
use crate::CadApp;

const BASES_KEY: &str = "workspace_sync_bases";
const MAX_BASES: usize = 32;
const SERVER_FIELDS: &[&str] =
    &["etag", "revision", "created_at", "updated_at", "profile_id", "tenant_id", "owner_type", "owner_id", "schema_version", "status"];

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteError {
    Offline,
    Stale { server: WorkspaceProfile },
    Forbidden(String),
    NotFound,
}

/// Calls must return promptly: a network implementation must bridge to an asynchronous worker.
pub trait ProfileRemote {
    fn get(&mut self, id: &str) -> Result<WorkspaceProfile, RemoteError>;
    fn patch(&mut self, id: &str, if_match: &str, patch: &Value) -> Result<WorkspaceProfile, RemoteError>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryRemote {
    pub profiles: BTreeMap<String, WorkspaceProfile>,
    pub online: bool,
    pub forbidden: BTreeSet<String>,
}

impl ProfileRemote for MemoryRemote {
    fn get(&mut self, id: &str) -> Result<WorkspaceProfile, RemoteError> {
        if !self.online {
            return Err(RemoteError::Offline);
        }
        if self.forbidden.contains(id) {
            return Err(RemoteError::Forbidden("access denied".into()));
        }
        let profile = self.profiles.get_mut(id).ok_or(RemoteError::NotFound)?;
        if profile.etag.as_deref().is_none_or(str::is_empty) {
            profile.etag = Some(format!("\"{}:{}\"", profile.profile_id, profile.revision));
        }
        Ok(profile.clone())
    }

    fn patch(&mut self, id: &str, if_match: &str, patch: &Value) -> Result<WorkspaceProfile, RemoteError> {
        let server = self.get(id)?;
        if server.etag.as_deref().is_none_or(|etag| etag != if_match) {
            return Err(RemoteError::Stale { server });
        }
        let mut next = apply_profile_patch(&server, patch).map_err(RemoteError::Forbidden)?;
        next.revision = server.revision.checked_add(1).ok_or_else(|| RemoteError::Forbidden("revision exhausted".into()))?;
        next.etag = Some(format!("\"{}:{}\"", next.profile_id, next.revision));
        startup::check_profile(&next, &[]).map_err(RemoteError::Forbidden)?;
        self.profiles.insert(id.to_string(), next.clone());
        Ok(next)
    }
}

#[derive(Clone, Debug)]
pub struct PendingConflict {
    pub base: WorkspaceProfile,
    pub mine: WorkspaceProfile,
    pub server: WorkspaceProfile,
    pub fields: Vec<FieldConflict>,
}

#[derive(Default)]
pub struct SyncState {
    remote: Option<Box<dyn ProfileRemote>>,
    bases: BTreeMap<String, WorkspaceProfile>,
    blocked: BTreeSet<String>,
    busy: bool,
    pub pending: Option<PendingConflict>,
}

// Metadata uses the same merge engine as layouts, inside an opaque object so layout defaults
// cannot turn a metadata deletion into a default value.
fn metadata(profile: &WorkspaceProfile) -> Result<WorkspaceLayout, String> {
    let mut value = serde_json::to_value(profile).map_err(|error| error.to_string())?;
    let fields = value.as_object_mut().ok_or("a profile must be an object")?;
    fields.remove("layout");
    for key in SERVER_FIELDS {
        fields.remove(*key);
    }
    let mut wrapper = WorkspaceLayout::default();
    wrapper.extra.insert("profile_fields".into(), value);
    Ok(wrapper)
}

fn profile_patch(base: &WorkspaceProfile, mine: &WorkspaceProfile) -> Result<Value, String> {
    let delta = diff_patch(&metadata(base)?, &metadata(mine)?);
    let mut patch = delta.get("profile_fields").and_then(Value::as_object).cloned().unwrap_or_default();
    let layout = diff_patch(&base.layout, &mine.layout);
    if layout.as_object().is_some_and(|fields| !fields.is_empty()) {
        patch.insert("layout".into(), layout);
    }
    Ok(Value::Object(patch))
}

fn apply_profile_patch(profile: &WorkspaceProfile, patch: &Value) -> Result<WorkspaceProfile, String> {
    let mut fields = patch.as_object().cloned().ok_or("a profile patch must be an object")?;
    if fields.keys().any(|key| SERVER_FIELDS.contains(&key.as_str())) {
        return Err("a patch cannot change server-owned fields".into());
    }
    let layout = match fields.remove("layout") {
        Some(patch) => try_apply_patch(&profile.layout, &patch)?,
        None => profile.layout.clone(),
    };
    let wrapper = try_apply_patch(&metadata(profile)?, &json!({"profile_fields": fields}))?;
    let mut value = serde_json::to_value(profile).map_err(|error| error.to_string())?;
    let root = value.as_object_mut().ok_or("a profile must be an object")?;
    let old = metadata(profile)?;
    if let Some(fields) = old.extra.get("profile_fields").and_then(Value::as_object) {
        for key in fields.keys() {
            root.remove(key);
        }
    }
    if let Some(fields) = wrapper.extra.get("profile_fields").and_then(Value::as_object) {
        root.extend(fields.clone());
    }
    root.insert("layout".into(), serde_json::to_value(layout).map_err(|error| error.to_string())?);
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn field_conflicts(base: &WorkspaceProfile, mine: &WorkspaceProfile, server: &WorkspaceProfile) -> Result<Vec<FieldConflict>, String> {
    let mut fields = conflicts(
        &serde_json::to_value(&base.layout).map_err(|error| error.to_string())?,
        &serde_json::to_value(&mine.layout).map_err(|error| error.to_string())?,
        &serde_json::to_value(&server.layout).map_err(|error| error.to_string())?,
    );
    let meta = |profile| metadata(profile).map(|mut layout| layout.extra.remove("profile_fields").unwrap_or(Value::Null));
    fields.extend(conflicts(&meta(base)?, &meta(mine)?, &meta(server)?));
    Ok(fields)
}

impl CadApp {
    pub fn attach_remote(&mut self, remote: Box<dyn ProfileRemote>) {
        let mut bases = BTreeMap::new();
        if let Some(saved) = self.ws.set_extra.get(BASES_KEY).and_then(Value::as_object) {
            for (id, value) in saved.iter().take(MAX_BASES) {
                if let Ok(profile) = serde_json::from_value::<WorkspaceProfile>(value.clone())
                    && profile.profile_id == *id
                    && startup::check_profile(&profile, &self.ws.policy()).is_ok()
                {
                    bases.insert(id.clone(), profile);
                }
            }
        }
        if bases.len() >= MAX_BASES
            && !bases.contains_key(&self.ws.user.profile_id)
            && let Some(id) = bases.keys().next().cloned()
        {
            bases.remove(&id);
        }
        bases.entry(self.ws.user.profile_id.clone()).or_insert_with(|| self.ws.user.clone());
        self.ws.sync = SyncState { remote: Some(remote), bases, ..SyncState::default() };
        // Persist before any edits so a crash between a local commit and its sync hook keeps the base.
        if self.ws.store.is_some()
            && !self.ws.needs_flush()
            && let Err(reason) = self.persist_sync_bases()
        {
            self.sync_failed(reason);
        }
    }

    /// Called after a successful local commit; no store, uncommitted edits, or no remote is a no-op.
    pub fn sync_now(&mut self) {
        if self.ws.sync.busy || self.ws.store.is_none() || self.ws.needs_flush() {
            return;
        }
        let Some(mut remote) = self.ws.sync.remote.take() else { return };
        self.ws.sync.busy = true;
        let id = self.ws.user.profile_id.clone();
        if self.ws.sync.blocked.contains(&id) {
            self.ws.status = SaveStatus::Offline;
        } else if self.ws.sync.pending.as_ref().is_some_and(|pending| pending.mine.profile_id == id) {
            self.ws.status = SaveStatus::Conflict;
        } else {
            self.ws.sync.pending = None;
            if let Err(reason) = self.sync_with(remote.as_mut()) {
                self.sync_failed(reason);
            }
        }
        self.ws.sync.busy = false;
        self.ws.sync.remote = Some(remote);
    }

    fn sync_with(&mut self, remote: &mut dyn ProfileRemote) -> Result<(), String> {
        let mine = self.ws.user.clone();
        let base = self.ws.sync.bases.get(&mine.profile_id).cloned();
        // The base is written before attempting the remote, including failed offline attempts.
        self.persist_sync_bases()?;
        self.ws.status = SaveStatus::Syncing;
        let Some(base) = base else {
            let server = remote.get(&mine.profile_id).map_err(|error| self.remote_error(error))?;
            return self.unknown_base(mine, server);
        };
        let Some(etag) = base.etag.as_deref() else {
            let server = remote.get(&mine.profile_id).map_err(|error| self.remote_error(error))?;
            if !profile_patch(&base, &server)?.as_object().is_some_and(Map::is_empty) {
                return self.unknown_base(mine, server);
            }
            return self.merge_server(remote, base, mine, server);
        };
        let patch = profile_patch(&base, &mine)?;
        if patch.as_object().is_some_and(Map::is_empty) {
            let server = remote.get(&mine.profile_id).map_err(|error| self.remote_error(error))?;
            return self.merge_server(remote, base, mine, server);
        }
        match remote.patch(&mine.profile_id, etag, &patch) {
            Ok(server) => self.accept_server(server, None),
            Err(RemoteError::Stale { server }) => self.merge_server(remote, base, mine, server),
            Err(error) => Err(self.remote_error(error)),
        }
    }

    fn unknown_base(&mut self, mine: WorkspaceProfile, server: WorkspaceProfile) -> Result<(), String> {
        self.check_server(&server)?;
        // Without an acknowledged base, differences cannot safely be attributed to either side.
        if profile_patch(&mine, &server)?.as_object().is_some_and(Map::is_empty) {
            return self.accept_server(server, None);
        }
        let fields = vec![FieldConflict {
            pointer: String::new(),
            mine: serde_json::to_value(&mine).map_err(|error| error.to_string())?,
            theirs: serde_json::to_value(&server).map_err(|error| error.to_string())?,
        }];
        self.ws.sync.pending = Some(PendingConflict { base: server.clone(), mine, server, fields });
        self.ws.status = SaveStatus::Conflict;
        Ok(())
    }

    fn merge_server(
        &mut self,
        remote: &mut dyn ProfileRemote,
        base: WorkspaceProfile,
        mine: WorkspaceProfile,
        server: WorkspaceProfile,
    ) -> Result<(), String> {
        self.check_server(&server)?;
        let fields = field_conflicts(&base, &mine, &server)?;
        if !fields.is_empty() {
            self.ws.sync.pending = Some(PendingConflict { base, mine, server, fields });
            self.ws.status = SaveStatus::Conflict;
            return Ok(());
        }
        let merged = apply_profile_patch(&server, &profile_patch(&base, &mine)?)?;
        let patch = profile_patch(&server, &merged)?;
        if patch.as_object().is_some_and(Map::is_empty) {
            return self.accept_server(server, None);
        }
        let etag = server.etag.as_deref().ok_or("the remote profile has no ETag")?;
        match remote.patch(&mine.profile_id, etag, &patch) {
            Ok(server) => self.accept_server(server, None),
            Err(RemoteError::Stale { server }) => {
                self.check_server(&server)?;
                let fields = field_conflicts(&base, &mine, &server)?;
                if fields.is_empty() {
                    return Err(crate::tl!("The remote workspace changed; sync again to retry").into());
                }
                self.ws.sync.pending = Some(PendingConflict { base, mine, server, fields });
                self.ws.status = SaveStatus::Conflict;
                Ok(())
            }
            Err(error) => Err(self.remote_error(error)),
        }
    }

    fn check_server(&self, server: &WorkspaceProfile) -> Result<(), String> {
        if server.profile_id != self.ws.user.profile_id || server.etag.as_deref().is_none_or(str::is_empty) {
            return Err("the remote returned an invalid profile identity or ETag".into());
        }
        startup::check_profile(server, &self.ws.policy())
    }

    fn persist_sync_bases(&mut self) -> Result<(), String> {
        let bases = serde_json::to_value(&self.ws.sync.bases).map_err(|error| error.to_string())?;
        if self.ws.set_extra.get(BASES_KEY) == Some(&bases) {
            return Ok(());
        }
        // A damaged set that could not be kept must not be overwritten; commit_workspace_now keeps it first.
        if self.ws.unkept_damage.is_some() {
            return Err("the damaged workspace has not been kept yet".into());
        }
        let mut extra = self.ws.set_extra.clone();
        extra.insert(BASES_KEY.into(), bases);
        let set = ProfileSet {
            active: self.ws.active.clone(),
            user: Some(self.ws.user.clone()),
            device: Some(self.ws.device.clone()),
            profiles: self.ws.profiles.clone(),
            extra: extra.clone(),
            ..ProfileSet::default()
        };
        let text = startup::set_text(&set, &self.ws.policy())?;
        startup::parse_set_with(&text, &self.ws.policy()).map_err(|error| startup::describe(&error))?;
        self.ws.store.as_mut().ok_or("no local storage is attached")?.write_current(&text)?;
        self.ws.set_extra = extra;
        Ok(())
    }

    fn accept_server(&mut self, server: WorkspaceProfile, copy: Option<WorkspaceProfile>) -> Result<(), String> {
        self.check_server(&server)?;
        if copy.is_none()
            && self.ws.sync.bases.get(&server.profile_id) == Some(&server)
            && self.ws.user.etag == server.etag
            && profile_patch(&server, &self.ws.user)?.as_object().is_some_and(Map::is_empty)
        {
            self.ws.sync.pending = None;
            self.ws.message = None;
            self.ws.status = SaveStatus::Synced;
            return Ok(());
        }
        let old_user = self.ws.user.clone();
        let old_profiles = self.ws.profiles.clone();
        let old_extra = self.ws.set_extra.clone();
        let old_bases = self.ws.sync.bases.clone();
        let mut bases = self.ws.sync.bases.clone();
        if bases.len() >= MAX_BASES
            && !bases.contains_key(&server.profile_id)
            && let Some(id) = bases.keys().next().cloned()
        {
            bases.remove(&id);
        }
        bases.insert(server.profile_id.clone(), server.clone());
        let saved_bases = serde_json::to_value(&bases).map_err(|error| error.to_string())?;
        if let Some(copy) = copy {
            self.ws.profiles.push(copy);
        }
        self.ws.sync.bases = bases;
        self.ws.set_extra.insert(BASES_KEY.into(), saved_bases);
        self.ws.user = WorkspaceProfile { revision: old_user.revision.max(server.revision), ..server };
        self.ws.rebuild();
        self.apply_workspace();
        if let Err(error) = self.commit_workspace_now() {
            self.ws.user = old_user;
            self.ws.profiles = old_profiles;
            self.ws.set_extra = old_extra;
            self.ws.sync.bases = old_bases;
            self.ws.rebuild();
            self.apply_workspace();
            return Err(error);
        }
        self.ws.sync.pending = None;
        self.ws.message = None;
        self.ws.status = SaveStatus::Synced;
        Ok(())
    }

    fn remote_error(&mut self, error: RemoteError) -> String {
        match error {
            RemoteError::Offline => crate::tl!("Workspace sync is offline").into(),
            RemoteError::Forbidden(reason) => {
                self.ws.sync.blocked.insert(self.ws.user.profile_id.clone());
                crate::tf!("Workspace sync was forbidden: {reason}", reason = reason)
            }
            RemoteError::NotFound => crate::tl!("The remote workspace was not found").into(),
            RemoteError::Stale { .. } => crate::tl!("The remote workspace changed; sync again to retry").into(),
        }
    }

    fn sync_failed(&mut self, reason: String) {
        if !matches!(self.ws.status, SaveStatus::Failed(_)) {
            self.ws.status = SaveStatus::Offline;
        }
        let message = crate::tf!("Workspace sync failed: {reason}", reason = reason);
        self.ws.message = Some(message.clone());
        self.set_status(message);
    }

    pub fn resolve_conflict(&mut self, resolution: Resolution) {
        if self.ws.sync.busy {
            return;
        }
        let Some(pending) = self.ws.sync.pending.clone() else { return };
        if pending.mine.profile_id != self.ws.user.profile_id {
            return;
        }
        let Some(mut remote) = self.ws.sync.remote.take() else { return };
        self.ws.sync.busy = true;
        let result = (|| {
            if self.ws.needs_flush() {
                self.commit_workspace_now()?;
            }
            match resolution {
                Resolution::UseServer => {
                    let server = remote.get(&self.ws.user.profile_id).map_err(|error| self.remote_error(error))?;
                    self.accept_server(server, None)
                }
                Resolution::KeepBoth => {
                    if self.ws.profiles.len() >= MAX_USER_PROFILES {
                        return Err("the saved workspace limit was reached; no copy was discarded".into());
                    }
                    let mine = &self.ws.user;
                    let prefix: String = mine.profile_id.chars().take(900).collect();
                    let id = (1..=MAX_USER_PROFILES + 1)
                        .map(|number| format!("{prefix}.my-copy-{number}"))
                        .find(|id| *id != mine.profile_id && !self.ws.profiles.iter().any(|profile| profile.profile_id == *id))
                        .ok_or("a unique workspace copy id could not be allocated")?;
                    let copy = WorkspaceProfile {
                        profile_id: id,
                        name: crate::tf!("{name} (my copy)", name = mine.name),
                        etag: None,
                        revision: 1,
                        owner_type: OwnerType::User,
                        status: ProfileStatus::Draft,
                        ..mine.clone()
                    };
                    startup::check_profile(&copy, &self.ws.policy())?;
                    let server = remote.get(&self.ws.user.profile_id).map_err(|error| self.remote_error(error))?;
                    self.accept_server(server, Some(copy))
                }
                Resolution::KeepMine => {
                    let mine = self.ws.user.clone();
                    let merged = apply_profile_patch(&pending.server, &profile_patch(&pending.base, &mine)?)?;
                    let patch = profile_patch(&pending.server, &merged)?;
                    let etag = pending.server.etag.as_deref().ok_or("the remote profile has no ETag")?;
                    match remote.patch(&mine.profile_id, etag, &patch) {
                        Ok(server) => self.accept_server(server, None),
                        Err(RemoteError::Stale { server }) => self.merge_server(remote.as_mut(), pending.base, mine, server),
                        Err(error) => Err(self.remote_error(error)),
                    }
                }
            }
        })();
        if let Err(reason) = result {
            self.sync_failed(reason);
        }
        self.ws.sync.busy = false;
        self.ws.sync.remote = Some(remote);
    }
}
