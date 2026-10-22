//! Workspace profiles: the serialisable description of a user's layout (ribbon, tool bars, panels,
//! shortcuts), its validation, the built-in presets, and the state that keeps the running interface
//! and the stored profile in step (layers, autosave, layout undo).

pub mod apply;
pub mod autosave;
pub mod canonical;
pub mod commands;
pub mod conflict;
pub mod device;
pub mod history;
pub mod merge;
pub mod migrate;
pub mod picker;
pub mod presets;
pub mod schema;
pub mod screen;
pub mod startup;
pub mod store;
pub mod sync;
pub mod tab_conflict;
pub mod validate;
pub mod wire;

use serde_json::{Map, Value, json};

use crate::{CadApp, FileKind};
use apply::{ApplyReport, Fingerprint};
use autosave::Autosave;
use device::{DeviceState, split_geometry};
use history::LayoutHistory;
use merge::{Effective, Layer, Level, apply_patch_locked, diff_patch, effective};
use presets::{PRESET_IDS, SAFE_PRESET, preset};
use schema::{SCHEMA_VERSION, WorkspaceLayout, WorkspaceProfile};
use startup::{FALLBACK_MESSAGE, Fallback, NOT_KEPT_MESSAGE, POLICY_MESSAGE};
use store::{ProfileSet, ProfileStore};

pub const WORKSPACE_FILE: FileKind = FileKind { label: "Workspace (JSON)", extensions: &["json"] };

pub const LOCKED_MESSAGE: &str = "Locked by company policy";
/// First wait before a failed commit is tried again; it doubles up to [`MAX_RETRY_SECS`].
const RETRY_SECS: f64 = 5.0;
const MAX_RETRY_SECS: f64 = 60.0;

#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    SavedLocally,
    Unsaved,
    Failed(String),
    Syncing,
    Synced,
    Offline,
    Conflict,
    TabConflict,
}

impl SaveStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SavedLocally => "saved_locally",
            Self::Unsaved => "unsaved",
            Self::Failed(_) => "failed",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
            Self::Offline => "offline",
            Self::Conflict => "conflict",
            Self::TabConflict => "tab_conflict",
        }
    }

    /// The status bar's words for it.
    pub fn label(&self) -> &'static str {
        match self {
            Self::SavedLocally => "Saved locally",
            Self::Unsaved => "Unsaved changes",
            Self::Failed(_) => "Could not save workspace",
            Self::Syncing => "Syncing",
            Self::Synced => "Synced",
            Self::Offline => "Offline",
            Self::Conflict => "Sync conflict",
            Self::TabConflict => "Changed in another tab",
        }
    }
}

/// One entry of the workspace picker.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub preset: bool,
}

pub struct WsState {
    /// The working profile: the user layer over `user.base_id` (no session layer in it).
    pub user: WorkspaceProfile,
    pub device: DeviceState,
    pub layers: Vec<Layer>,
    pub effective: Effective,
    pub history: LayoutHistory,
    pub autosave: Autosave,
    pub status: SaveStatus,
    /// Revision of the stored set this app last read or wrote.
    pub known_revision: u64,
    pub sync: sync::SyncState,
    /// `None`: nothing is persisted, as before workspace profiles.
    pub store: Option<Box<dyn ProfileStore>>,
    pub fallback: Option<Fallback>,
    /// The preset id or saved profile id the user chose.
    pub active: String,
    /// Saved profiles (Save As).
    pub profiles: Vec<WorkspaceProfile>,
    /// A notice for the user (fallback, policy refusal); also shown in the status bar.
    pub message: Option<String>,
    /// What the last apply could not place.
    pub report: ApplyReport,
    /// The content rect ratios are taken against; follows the window each frame.
    pub area: egui::Rect,
    /// Unknown fields of the stored set, written back unchanged.
    pub set_extra: Map<String, Value>,
    /// The attached store held nothing: this is the first run, and the picker is still to be offered.
    pub first_run: bool,
    /// The open picker is the first-run one, so closing it unchosen picks SIMPLE.
    pub(crate) first_run_picker_open: bool,
    /// A layout was replaced while a panel may be mid-drag; the docking surface drops that drag.
    pub(crate) cancel_drag: bool,
    /// A tool bar grip is being dragged; a replaced layout drops it (like [`Self::cancel_drag`] for panels).
    pub toolbar_drag: crate::toolbars::DragPhase,
    pub(crate) applied: WorkspaceLayout,
    pub(crate) synced: Option<Fingerprint>,
    /// A damaged set startup could not copy aside; commits wait until the copy succeeds.
    unkept_damage: Option<Vec<u8>>,
    unkept_backup: Option<String>,
    pub suspended: bool,
    failed_commits: u32,
    now: f64,
    drag_before: Option<WorkspaceLayout>,
    released: bool,
    shown_scale: Option<f32>,
    area_known: bool,
}

impl Default for WsState {
    fn default() -> Self {
        let user = startup::fresh_user(SAFE_PRESET, &[]);
        let device = DeviceState::default();
        let layers = startup::layers_for(&user, &[], &device);
        let effective = effective(&layers);
        Self {
            applied: effective.layout.clone(),
            user,
            device,
            layers,
            effective,
            history: LayoutHistory::default(),
            autosave: Autosave::default(),
            status: SaveStatus::Unsaved,
            known_revision: 0,
            sync: sync::SyncState::default(),
            store: None,
            fallback: None,
            active: SAFE_PRESET.into(),
            profiles: Vec::new(),
            message: None,
            report: ApplyReport::default(),
            area: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)),
            set_extra: Map::new(),
            first_run: false,
            first_run_picker_open: false,
            cancel_drag: false,
            toolbar_drag: crate::toolbars::DragPhase::Idle,
            synced: None,
            unkept_damage: None,
            unkept_backup: None,
            suspended: false,
            failed_commits: 0,
            now: 0.0,
            drag_before: None,
            released: false,
            shown_scale: None,
            area_known: false,
        }
    }
}

impl WsState {
    /// Presets first, in shipping order, then the saved profiles.
    pub fn choices(&self) -> Vec<Choice> {
        let presets = PRESET_IDS.iter().map(|id| Choice { id: (*id).into(), name: preset(id).map(|p| p.name).unwrap_or_default(), preset: true });
        let saved = self.profiles.iter().map(|profile| Choice { id: profile.profile_id.clone(), name: profile.name.clone(), preset: false });
        presets.chain(saved).collect()
    }

    fn policy(&self) -> Vec<Layer> {
        self.layers.iter().filter(|layer| !matches!(layer.level, Level::Platform | Level::User | Level::Device | Level::Session)).cloned().collect()
    }

    /// The layout the user layer starts from for the current base.
    pub fn below_user(&self) -> WorkspaceLayout {
        startup::below_user(self.user.base_id.as_deref(), &self.policy())
    }

    /// Recomputes the layers and the effective layout after `user` changed.
    pub fn rebuild(&mut self) {
        let session: Vec<Layer> = self.layers.iter().filter(|layer| layer.level == Level::Session).cloned().collect();
        let mut layers = startup::layers_for(&self.user, &self.policy(), &self.device);
        layers.extend(session);
        self.layers = layers;
        self.effective = effective(&self.layers);
    }

    fn local_layout(&self) -> WorkspaceLayout {
        effective(&startup::layers_for(&self.user, &self.policy(), &self.device)).layout
    }

    fn keep_layout(&mut self, layout: WorkspaceLayout) {
        let (user, _) = split_geometry(diff_patch(&self.local_layout(), &layout));
        self.user.layout = merge::apply_patch(&self.user.layout, &user);
        self.device.layout_patch = split_geometry(diff_patch(&self.user.layout, &layout)).1;
    }

    /// Whether a change is not stored yet; a host checks this before it exits.
    pub fn needs_flush(&self) -> bool {
        !self.suspended && (self.autosave.pending() || matches!(self.status, SaveStatus::Failed(_) | SaveStatus::TabConflict))
    }

    fn mark_changed(&mut self) {
        if self.suspended || self.status == SaveStatus::TabConflict {
            return;
        }
        self.autosave.changed(self.now);
        self.status = SaveStatus::Unsaved;
    }
}

/// Folds interface changes made since the last look into the user layer. `record` adds a layout
/// undo step; a drag passes false and records once when it ends. Returns whether anything changed.
pub(crate) fn observe(app: &mut CadApp, record: bool) -> bool {
    let now = Fingerprint::of(app);
    if app.ws.synced.as_ref() == Some(&now) {
        return false;
    }
    let first_look = app.ws.synced.is_none();
    let captured = apply::capture(app, app.ws.area);
    // A profile that names a theme or language keeps the user's later choice; otherwise the
    // preferences keep it, as before profiles.
    let mut prefs_changed = false;
    if let Some(base) = &app.ws.synced {
        if base.theme != now.theme && app.ws.user.theme.is_some() {
            app.ws.user.theme = Some(now.theme.to_string());
            prefs_changed = true;
        }
        if base.language != now.language && app.ws.user.locale.is_some() {
            app.ws.user.locale = Some(now.language.to_string());
            prefs_changed = true;
        }
    }
    app.ws.synced = Some(now);
    let before = std::mem::replace(&mut app.ws.applied, captured.clone());
    if first_look {
        // An app that never applied a profile: what it shows is the baseline, not an edit.
        app.ws.keep_layout(captured);
        app.ws.rebuild();
        return false;
    }
    if before == captured {
        if prefs_changed {
            app.ws.mark_changed();
        }
        return prefs_changed;
    }
    let delta = diff_patch(&before, &captured);
    let local = app.ws.local_layout();
    let open = apply_patch_locked(&local, &delta, &Default::default());
    let mut allowed = apply_patch_locked(&local, &delta, &app.ws.effective.locked);
    // The user's layer may already hold a value a lock hides, so an unchanged layer does not mean
    // the edit went through.
    let refused: Vec<String> =
        merge::changed_pointers(&delta).into_iter().filter(|pointer| merge::check(&app.ws.effective, pointer).is_err()).collect();
    let blocked = open != allowed || !refused.is_empty();
    let panels_refused = open.panels != allowed.panels || refused.iter().any(|pointer| pointer == "/panels" || pointer.starts_with("/panels/"));
    // The tree holds every panel's placement, including a refused move or a session's placement;
    // the panel list alone then describes what the user may keep.
    let session_places_panels = app.ws.layers.iter().any(|layer| layer.level == Level::Session && layer.patch.get("panels").is_some());
    if allowed.dock_tree != local.dock_tree && (panels_refused || session_places_panels) && merge::check(&app.ws.effective, "/dock_tree").is_ok() {
        allowed.dock_tree = None;
    }
    if allowed != local {
        if record {
            app.ws.history.record(local);
        }
        app.ws.keep_layout(allowed);
        app.ws.mark_changed();
    }
    app.ws.rebuild();
    if blocked {
        app.ws.message = Some(LOCKED_MESSAGE.into());
        app.set_status(LOCKED_MESSAGE);
    }
    // Policy refusals and session overrides put the interface back to what the layers say. Without
    // either the layers reproduce the capture, and re-applying would only move floating panels.
    let overridden = blocked || app.ws.layers.iter().any(|layer| layer.level == Level::Session);
    if overridden && app.ws.effective.layout != captured {
        app.apply_workspace();
    }
    true
}

/// Changes the tool bar list through the user layer, so policy locks, layout undo and autosave apply
/// as for any other layout edit. Returns whether anything changed.
pub(crate) fn edit_toolbars(app: &mut CadApp, change: impl FnOnce(&mut Vec<schema::ToolbarState>) -> Result<(), String>) -> Result<bool, String> {
    observe(app, true);
    let before = app.ws.local_layout();
    let mut next = before.clone();
    change(&mut next.toolbars)?;
    if next == before {
        return Ok(false);
    }
    let delta = diff_patch(&before, &next);
    if merge::changed_pointers(&delta).iter().any(|pointer| merge::check(&app.ws.effective, pointer).is_err()) {
        return Err(LOCKED_MESSAGE.into());
    }
    let profile = WorkspaceProfile { layout: next, ..app.ws.user.clone() };
    startup::check_profile(&profile, &app.ws.policy()).map_err(|reason| format!("The tool bars cannot be stored: {reason}"))?;
    app.ws.history.record(before);
    app.ws.keep_layout(profile.layout);
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    Ok(true)
}

/// Profile-only edits use the same policy, history and autosave path as docked controls.
pub(crate) fn edit_layout(app: &mut CadApp, change: impl FnOnce(&mut WorkspaceLayout) -> Result<(), String>) -> Result<bool, String> {
    observe(app, true);
    let before = app.ws.local_layout();
    let mut next = before.clone();
    change(&mut next)?;
    if next == before {
        return Ok(false);
    }
    let delta = diff_patch(&before, &next);
    if merge::changed_pointers(&delta).iter().any(|pointer| merge::check(&app.ws.effective, pointer).is_err()) {
        return Err(LOCKED_MESSAGE.into());
    }
    let profile = WorkspaceProfile { layout: next, ..app.ws.user.clone() };
    startup::check_profile(&profile, &app.ws.policy()).map_err(|reason| crate::tf!("The workspace cannot be stored: {reason}", reason = reason))?;
    app.ws.history.record(before);
    app.ws.keep_layout(profile.layout);
    app.ws.rebuild();
    app.apply_workspace();
    app.ws.mark_changed();
    Ok(true)
}

pub(crate) fn apply_aliases(app: &CadApp) {
    let aliases: Vec<_> = app.ws.effective.layout.aliases.iter().map(|(alias, command)| (alias.clone(), command.clone())).collect();
    cadcraft_engine::cmd::file::pgp::set_aliases(&aliases);
}

/// Alias commands mutate the engine store first; a refused profile edit restores the active map.
pub(crate) fn sync_aliases(app: &mut CadApp, before: &[(String, String)]) -> Result<(), String> {
    let aliases = cadcraft_engine::cmd::file::pgp::aliases();
    if aliases == before {
        return Ok(());
    }
    let after: std::collections::BTreeMap<_, _> = aliases.into_iter().collect();
    let before: std::collections::BTreeMap<_, _> = before.iter().cloned().collect();
    let result = edit_layout(app, |layout| {
        for alias in before.keys().chain(after.keys()) {
            if before.get(alias) != after.get(alias) {
                let key = layout.aliases.keys().find(|held| held.eq_ignore_ascii_case(alias)).cloned().unwrap_or_else(|| alias.clone());
                layout.aliases.retain(|held, _| !held.eq_ignore_ascii_case(alias));
                if let Some(command) = after.get(alias) {
                    layout.aliases.insert(key, command.clone());
                }
            }
        }
        Ok(())
    });
    if let Err(error) = result {
        apply_aliases(app);
        return Err(error);
    }
    Ok(())
}

/// Opens the picker once on a first run, unless the host asked to skip it (UX01).
fn offer_picker(app: &mut CadApp) {
    if !app.ws.first_run {
        return;
    }
    if app.skip_first_run_picker {
        app.ws.first_run = false;
        return;
    }
    // Whatever the host already put on screen stays; the picker comes on a later frame.
    if app.ui.dialog.is_some() {
        return;
    }
    app.ws.first_run = false;
    app.ws.first_run_picker_open = true;
    app.ui.dialog = Some(picker::PICKER_DIALOG.into());
}

/// Per-frame workspace upkeep: follows the window, records drags once they end, autosaves.
pub fn tick(app: &mut CadApp, ctx: &egui::Context) {
    if app.ws.store.is_none() {
        return;
    }
    offer_picker(app);
    let (time, down, released) = ctx.input(|input| (input.time, input.pointer.any_down(), input.pointer.any_released()));
    app.ws.now = time;
    let area = ctx.content_rect();
    if area.is_finite() && area.width() > 0.0 && area.height() > 0.0 && area != app.ws.area {
        let first = !app.ws.area_known;
        app.ws.area = area;
        // The store is attached before the window has a size; lay the profile out for the real one
        // unless the user already changed something.
        if first && app.ws.synced.as_ref() == Some(&Fingerprint::of(app)) {
            app.apply_workspace();
        }
    }
    let area = app.ws.area;
    app.ws.device.screen = Some(screen::ScreenProfile {
        work_area: [area.min.x, area.min.y, area.width(), area.height()],
        pixel_ratio: ctx.native_pixels_per_point().filter(|ratio| ratio.is_finite() && *ratio > 0.0).unwrap_or(1.0),
        ..app.ws.device.screen.clone().unwrap_or_default()
    });
    app.ws.area_known = true;
    if down {
        if app.ws.drag_before.is_none() && app.ws.synced.as_ref() != Some(&Fingerprint::of(app)) {
            app.ws.drag_before = Some(app.ws.local_layout());
        }
        observe(app, false);
    } else if let Some(before) = app.ws.drag_before.take() {
        observe(app, false);
        if app.ws.local_layout() != before {
            app.ws.history.record(before);
            app.ws.mark_changed();
            app.ws.autosave.interaction_ended();
        }
    } else {
        // Docking applies a drop during the frame of the release; it shows up here a frame later.
        let after_release = app.ws.released || released;
        if observe(app, true) && after_release {
            app.ws.autosave.interaction_ended();
        }
    }
    app.ws.released = released;
    let external = app.ws.store.as_mut().is_some_and(|store| store.take_external_change());
    if external
        && !app.ws.suspended
        && !app.ws.needs_flush()
        && let Err(error) = app.adopt_stored_workspace()
    {
        app.set_status(error);
    }
    // Another dialog replacing the picker abandons it; storing resumes.
    if app.ui.dialog.as_deref() != Some(picker::PICKER_DIALOG) {
        app.ws.first_run_picker_open = false;
    }
    // The debounce does not store while the picker waits, so a layout settling under it is not written as a choice.
    let storing = !app.ws.first_run_picker_open && !app.ws.suspended;
    if !down && storing && app.ws.autosave.due(time) {
        let _ = app.commit_workspace_now();
    }
    if storing && app.ws.autosave.pending() {
        let wait = std::time::Duration::try_from_secs_f64(app.ws.autosave.wait_secs()).unwrap_or(std::time::Duration::from_millis(750));
        ctx.request_repaint_after(wait);
    }
}

/// RFC 3339 UTC for `unix_ms`, written out by hand to avoid a date dependency.
pub fn rfc3339(unix_ms: f64) -> String {
    let secs = if unix_ms.is_finite() && unix_ms > 0.0 { (unix_ms / 1000.0) as i64 } else { 0 };
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil-from-days (H. Hinnant), valid for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

impl CadApp {
    /// Takes over profile storage: restores the stored workspace (or SIMPLE) and shows it.
    pub fn attach_profile_store(&mut self, mut store: Box<dyn ProfileStore>) {
        let raw = store.read_current();
        let revision = raw.as_deref().and_then(startup::stored_revision).unwrap_or(0);
        let (restored, unkept) = startup::open(store.as_mut());
        self.ws.known_revision = revision.max(restored.user.revision);
        let damaged = matches!(restored.fallback, Some(Fallback::Corrupt(_) | Fallback::NewerMajor(_) | Fallback::Invalid(_)));
        let kept = unkept.is_none();
        self.ws.unkept_damage = unkept;
        self.ws.unkept_backup = match &restored.fallback {
            None => raw.filter(|raw| startup::needs_backup(raw)),
            Some(Fallback::RestoredSnapshot { revision, .. }) => store.read_snapshot(*revision).filter(|raw| startup::needs_backup(raw)),
            _ => None,
        };
        self.ws.failed_commits = 0;
        let mut notices: Vec<String> = Vec::new();
        if let Some(Fallback::RestoredSnapshot { revision, .. }) = &restored.fallback {
            notices.push(if kept {
                crate::tf!(
                    "Workspace could not be read; restored the last good copy (revision {revision}). The damaged copy was kept.",
                    revision = revision
                )
            } else {
                crate::tf!(
                    "Workspace could not be read; restored the last good copy (revision {revision}). The damaged copy could not be kept, so it is not overwritten until it can be.",
                    revision = revision
                )
            });
        } else if damaged {
            notices.push(if kept { FALLBACK_MESSAGE.into() } else { NOT_KEPT_MESSAGE.into() });
        }
        if let Some(problem) = store.read_policy().and_then(|raw| startup::policy_problem(&raw)) {
            log::warn!("workspace: policy: {problem}");
            notices.push(format!("{POLICY_MESSAGE}: {problem}"));
        }
        let session: Vec<Layer> = self.ws.layers.iter().filter(|layer| layer.level == Level::Session).cloned().collect();
        self.ws.store = Some(store);
        self.ws.device = restored.set.device.clone().unwrap_or_default();
        self.ws.device.screen = Some(screen::ScreenProfile {
            work_area: [self.ws.area.min.x, self.ws.area.min.y, self.ws.area.width(), self.ws.area.height()],
            ..self.ws.device.screen.clone().unwrap_or_default()
        });
        self.ws.user = restored.user;
        self.ws.layers = restored.layers;
        self.ws.layers.extend(session);
        self.ws.effective = effective(&self.ws.layers);
        self.ws.active = restored.set.active;
        self.ws.profiles = restored.set.profiles;
        self.ws.set_extra = restored.set.extra;
        self.ws.history.clear();
        self.ws.autosave.clear();
        if matches!(restored.fallback, Some(Fallback::RestoredSnapshot { .. })) {
            self.ws.autosave.changed(self.ws.now);
        }
        self.ws.status = if restored.fallback.is_none() { SaveStatus::SavedLocally } else { SaveStatus::Unsaved };
        self.ws.first_run = matches!(restored.fallback, Some(Fallback::NoProfile));
        self.ws.fallback = restored.fallback;
        self.apply_workspace();
        if self.ws.unkept_backup.is_some() && self.ws.fallback.is_none() && !self.ws.suspended {
            let _ = self.commit_workspace(false);
        }
        if !notices.is_empty() {
            let notice = notices.join(" ");
            self.ws.message = Some(notice.clone());
            self.set_status(notice);
        }
    }

    /// Shows the effective layout and the profile's theme and language (when it names them).
    pub fn apply_workspace(&mut self) {
        self.autohide = crate::autohide::State::default();
        if let Some(theme) = self.ws.user.theme.as_deref().and_then(crate::theme::ThemePref::parse) {
            self.ui.theme = theme;
        }
        if let Some(language) = self.ws.user.locale.as_deref().and_then(crate::i18n::Preference::parse) {
            self.ui.interface_language = language;
        }
        let layout = self.ws.effective.layout.clone();
        let area = self.ws.area;
        self.ws.report = apply::apply(self, &layout, area);
        apply_aliases(self);
        self.ws.cancel_drag = true;
        self.ws.toolbar_drag = crate::toolbars::DragPhase::Idle;
    }

    /// Replaces the session layer: changes for this run only (an embedding host's), never stored.
    pub fn set_session_layer(&mut self, patch: Value) {
        observe(self, true);
        self.ws.layers.retain(|layer| layer.level != Level::Session);
        if patch.as_object().is_some_and(|fields| !fields.is_empty()) {
            self.ws.layers.push(Layer { level: Level::Session, patch, locks: Vec::new() });
        }
        self.ws.effective = effective(&self.ws.layers);
        self.apply_workspace();
    }

    pub(super) fn adopt_stored_workspace(&mut self) -> Result<(), String> {
        if self.ws.suspended {
            return Err(commands::ROLLBACK_MESSAGE.into());
        }
        let store = self.ws.store.take().ok_or("No workspace storage is attached")?;
        self.attach_profile_store(store);
        self.ws.drag_before = None;
        self.ws.first_run = false;
        self.ws.first_run_picker_open = false;
        if self.ui.dialog.as_deref() == Some(tab_conflict::DIALOG) {
            self.ui.dialog = None;
        }
        if self.ws.fallback.is_none() || matches!(self.ws.fallback, Some(Fallback::NoProfile)) {
            self.ws.message = Some("Updated from another tab".into());
            self.set_status(crate::tl!("Updated from another tab"));
        }
        Ok(())
    }

    pub(super) fn check_workspace_revision(&mut self, interactive: bool) -> Result<(), String> {
        let raw = self.ws.store.as_ref().and_then(|store| store.read_current());
        let stored = raw.as_deref().and_then(startup::stored_revision).unwrap_or(0);
        if stored > self.ws.known_revision || self.ws.status == SaveStatus::TabConflict {
            self.ws.status = SaveStatus::TabConflict;
            self.ws.autosave.clear();
            if interactive {
                self.ui.dialog = Some(tab_conflict::DIALOG.into());
            }
            return Err("Workspace changed in another tab".into());
        }
        Ok(())
    }

    /// Stores the workspace now: next revision, a snapshot of it, then the current set.
    pub fn commit_workspace_now(&mut self) -> Result<(), String> {
        self.commit_workspace(true)
    }

    pub(super) fn commit_workspace(&mut self, interactive: bool) -> Result<(), String> {
        if self.ws.suspended {
            return Err(commands::ROLLBACK_MESSAGE.into());
        }
        observe(self, true);
        // localStorage cannot check and write atomically; IndexedDB transactions will close this race.
        self.check_workspace_revision(interactive)?;
        let policy = self.ws.policy();
        let Some(store) = self.ws.store.as_mut() else { return Err("No workspace storage is attached".into()) };
        let newest = store.snapshots().into_iter().max().unwrap_or(0);
        let mut user = self.ws.user.clone();
        user.revision = newest.max(user.revision).max(self.ws.known_revision).saturating_add(1);
        user.updated_at = rfc3339(crate::now_ms());
        if user.created_at.is_empty() {
            user.created_at = user.updated_at.clone();
        }
        let mut profiles = self.ws.profiles.clone();
        if let Some(saved) = profiles.iter_mut().find(|profile| profile.profile_id == self.ws.active) {
            *saved = user.clone();
        }
        let set = ProfileSet {
            schema_version: SCHEMA_VERSION,
            active: self.ws.active.clone(),
            profiles,
            user: Some(user.clone()),
            device: Some(self.ws.device.clone()),
            extra: self.ws.set_extra.clone(),
        };
        let unkept_damage = &mut self.ws.unkept_damage;
        let unkept_backup = &mut self.ws.unkept_backup;
        let written = startup::set_text(&set, &policy).and_then(|json| {
            // Never write a set the next startup would refuse: that would lose every setting.
            startup::parse_set_with(&json, &policy).map_err(|fallback| format!("it would not read back: {}", startup::describe(&fallback)))?;
            if let Some(damaged) = unkept_damage.as_deref() {
                startup::keep_copy(store.as_mut(), damaged)
                    .map_err(|error| format!("the damaged workspace could not be kept, so it is not replaced: {error}"))?;
                *unkept_damage = None;
            }
            if let Some(raw) = unkept_backup.as_deref() {
                store.keep_backup("1.0.0", raw)?;
                *unkept_backup = None;
            }
            store.push_snapshot(user.revision, &json)?;
            store.write_current(&json)
        });
        match written {
            Ok(()) => {
                self.ws.known_revision = user.revision;
                self.ws.user = user;
                self.ws.profiles = set.profiles;
                self.ws.status = SaveStatus::SavedLocally;
                self.ws.autosave.clear();
                self.ws.failed_commits = 0;
                self.sync_now();
                Ok(())
            }
            Err(error) => {
                // Storage that is full or blocked now may not be later: try again, less and less often.
                let delay = (RETRY_SECS * 2f64.powi(self.ws.failed_commits.min(4) as i32)).min(MAX_RETRY_SECS);
                self.ws.failed_commits = self.ws.failed_commits.saturating_add(1);
                self.ws.autosave.retry(self.ws.now, delay);
                self.ws.status = SaveStatus::Failed(error.clone());
                self.set_status(format!("Could not save workspace: {error}"));
                Err(error)
            }
        }
    }

    /// Saved state for `ui.workspace.status`.
    pub fn workspace_status(&self) -> Value {
        let ws = &self.ws;
        let fallback = ws.fallback.as_ref().map(|fallback| match fallback {
            Fallback::NoProfile => "no_profile",
            Fallback::Corrupt(_) => "corrupt",
            Fallback::NewerMajor(_) => "newer_major",
            Fallback::Invalid(_) => "invalid",
            Fallback::RestoredSnapshot { .. } => "restored_snapshot",
        });
        let restored_revision = match &ws.fallback {
            Some(Fallback::RestoredSnapshot { revision, .. }) => Some(*revision),
            _ => None,
        };
        let error = match &ws.status {
            SaveStatus::Failed(error) => Some(error.clone()),
            _ => None,
        };
        json!({
            "status": if ws.suspended { "suspended" } else { ws.status.as_str() },
            "label": if ws.suspended { "Saving suspended" } else { ws.status.label() },
            "error": error,
            "message": ws.message,
            "active": ws.active,
            "base_id": ws.user.base_id,
            "revision": ws.user.revision,
            "pending": ws.autosave.pending(),
            "persisted": ws.store.is_some(),
            "fallback": fallback,
            "restored_revision": restored_revision,
            "undo": ws.history.undo_len(),
            "redo": ws.history.redo_len(),
            "snapshots": ws.store.as_ref().map(|store| store.snapshots()).unwrap_or_default(),
            "workspaces": ws.choices().iter().map(|choice| json!({"id": choice.id, "name": choice.name, "preset": choice.preset})).collect::<Vec<_>>(),
            "unknown_commands": ws.report.unknown_commands,
            "unknown_panels": ws.report.unknown_panels,
        })
    }
}
