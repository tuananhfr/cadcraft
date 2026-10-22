//! The two save states the status bar keeps apart: the drawing (saved, downloaded, autosaved) and
//! the workspace layout (stored on this device). Both are read from state the app already keeps.

use serde_json::Value;

use super::startup::Fallback;
use super::{SaveStatus, WsState, rfc3339};
use crate::CadApp;

/// Where the workspace layout stands with the profile store.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutSave {
    Clean,
    Dirty,
    SavingLocal,
    SavedLocal,
    // Cloud sync is deferred, so nothing enters these three yet.
    SyncPending,
    Syncing,
    Synced,
    Retry(String),
    Conflict,
    LastValidSnapshot(u64),
    SafeMode,
    Suspended,
}

impl LayoutSave {
    pub fn from_ws(ws: &WsState) -> Self {
        if ws.safe_mode {
            return Self::SafeMode;
        }
        if ws.suspended {
            return Self::Suspended;
        }
        match &ws.status {
            SaveStatus::TabConflict | SaveStatus::Conflict => return Self::Conflict,
            SaveStatus::Failed(error) => return Self::Retry(error.clone()),
            _ => {}
        }
        let restored = match &ws.fallback {
            Some(Fallback::RestoredSnapshot { revision, .. }) if !ws.edited => Some(*revision),
            _ => None,
        };
        if let Some(revision) = restored {
            return Self::LastValidSnapshot(revision);
        }
        if ws.autosave.pending() {
            return Self::Dirty;
        }
        if ws.writing {
            return Self::SavingLocal;
        }
        // A synced or offline set is saved on this device whatever the server holds.
        let saved = matches!(ws.status, SaveStatus::SavedLocally | SaveStatus::Syncing | SaveStatus::Synced | SaveStatus::Offline);
        // A first run shows a preset, which needs no storing until the user changes it.
        let first_run = matches!(ws.fallback, Some(Fallback::NoProfile));
        if ws.edited {
            if saved { Self::SavedLocal } else { Self::Dirty }
        } else if saved || first_run {
            Self::Clean
        } else {
            Self::Dirty
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::SavingLocal => "saving_local",
            Self::SavedLocal => "saved_local",
            Self::SyncPending => "sync_pending",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
            Self::Retry(_) => "retry",
            Self::Conflict => "conflict",
            Self::LastValidSnapshot(_) => "last_valid_snapshot",
            Self::SafeMode => "safe_mode",
            Self::Suspended => "suspended",
        }
    }

    /// The full sentence, for the status bar's tooltip and automation.
    pub fn label(&self) -> String {
        match self {
            Self::Clean | Self::SavedLocal => crate::tl!("Layout saved on this device (not synced)").into(),
            Self::Dirty => crate::tl!("Layout changed").into(),
            Self::SavingLocal => crate::tl!("Saving layout…").into(),
            Self::SyncPending | Self::Syncing => crate::tl!("Syncing").into(),
            Self::Synced => crate::tl!("Synced").into(),
            Self::Retry(_) => crate::tl!("Could not save layout; retrying").into(),
            Self::Conflict => crate::tl!("Changed in another tab").into(),
            Self::LastValidSnapshot(revision) => crate::tf!("Layout restored from revision {n}", n = revision),
            Self::SafeMode => crate::tl!("Safe mode: layout changes are not saved").into(),
            Self::Suspended => crate::tl!("Restored the 1.0.0 workspace. Changes are not saved until restart.").into(),
        }
    }

    /// The few words the status bar shows.
    pub fn short(&self) -> String {
        match self {
            Self::Clean | Self::SavedLocal => crate::tl!("Layout saved").into(),
            Self::Retry(_) => crate::tl!("Layout not saved").into(),
            Self::LastValidSnapshot(_) => crate::tl!("Layout restored").into(),
            Self::SafeMode => crate::tl!("Safe mode").into(),
            Self::Suspended => crate::tl!("Saving suspended").into(),
            _ => self.label(),
        }
    }
}

/// Where the open drawings stand with their files.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawingSave {
    Clean,
    Saved,
    Downloaded,
    Dirty,
    SavingLocal,
    AutosavedLocal { at_ms: f64 },
}

impl DrawingSave {
    pub fn from_app(app: &CadApp) -> Self {
        if app.has_unsaved_drawings() {
            return app.browser_drawing_save().unwrap_or(Self::Dirty);
        }
        let Ok(state) = app.session.state() else { return Self::Clean };
        if app.ui.downloaded.contains(&state.uid) {
            Self::Downloaded
        } else if state.path.is_some() {
            Self::Saved
        } else {
            Self::Clean
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Saved => "saved",
            Self::Downloaded => "downloaded",
            Self::Dirty => "dirty",
            Self::SavingLocal => "saving_local",
            Self::AutosavedLocal { .. } => "autosaved_local",
        }
    }

    /// The full sentence, for the status bar's tooltip and automation; empty for [`Self::Clean`],
    /// which the status bar does not show.
    pub fn label(&self) -> String {
        match self {
            Self::Clean => String::new(),
            Self::Saved => crate::tl!("Drawing saved").into(),
            Self::Downloaded => crate::tl!("Drawing downloaded").into(),
            Self::Dirty => crate::tl!("Drawing has unsaved changes").into(),
            Self::SavingLocal => crate::tl!("Saving drawing…").into(),
            Self::AutosavedLocal { at_ms } => crate::tf!("Drawing autosaved in this browser {time}", time = clock(*at_ms)),
        }
    }

    /// The few words the status bar shows.
    pub fn short(&self) -> String {
        match self {
            Self::AutosavedLocal { .. } => crate::tl!("Drawing autosaved").into(),
            _ => self.label(),
        }
    }
}

/// `HH:MM UTC`; there is no time zone database to show local time with.
fn clock(unix_ms: f64) -> String {
    let stamp = rfc3339(unix_ms);
    format!("{} UTC", stamp.get(11..16).unwrap_or_default())
}

impl CadApp {
    /// Notes how the active drawing was last saved, from a save command's result: a browser
    /// download (the web build's results name one) or a file.
    pub fn record_drawing_save(&mut self, result: &Value) {
        let Ok(state) = self.session.state() else { return };
        // Exports (PDF, SVG) report a path too, but the drawing keeps its own file.
        if state.path.as_deref() != result.get("path").and_then(Value::as_str) {
            return;
        }
        let uid = state.uid;
        if result.get("download").is_some() {
            self.ui.downloaded.insert(uid);
        } else {
            self.ui.downloaded.remove(&uid);
        }
    }
}
