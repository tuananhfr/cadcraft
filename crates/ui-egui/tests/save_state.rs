//! The status bar's two save states: the drawing (saved, downloaded) and the workspace layout
//! (stored on this device), and the commit durability the layout state follows.

use std::collections::VecDeque;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::Drawing;
use cadcraft_ui_egui::wsprofile::SaveStatus;
use cadcraft_ui_egui::wsprofile::autosave::Autosave;
use cadcraft_ui_egui::wsprofile::save_state::{DrawingSave, LayoutSave};
use cadcraft_ui_egui::wsprofile::store::{Durability, MemoryStore, ProfileStore};
use cadcraft_ui_egui::{CadApp, Services};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Engine aliases are one store per process; tests that run engine commands hold this so a
/// parallel test's alias change is not taken for their own layout edit.
fn alias_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A store that writes at once but reports durability from a script; its last entry repeats.
struct Scripted {
    inner: MemoryStore,
    script: VecDeque<Durability>,
}

impl Scripted {
    fn new(script: impl IntoIterator<Item = Durability>) -> Self {
        Self { inner: MemoryStore::default(), script: script.into_iter().collect() }
    }
}

impl ProfileStore for Scripted {
    fn read_current(&self) -> Option<String> {
        self.inner.read_current()
    }
    fn write_current(&mut self, json: &str) -> Result<(), String> {
        self.inner.write_current(json)
    }
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        self.inner.push_snapshot(revision, json)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.inner.snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.inner.read_snapshot(revision)
    }
    fn durability(&mut self) -> Durability {
        if self.script.len() > 1 {
            return self.script.pop_front().unwrap_or(Durability::Committed);
        }
        self.script.front().cloned().unwrap_or(Durability::Committed)
    }
}

fn attached(store: impl ProfileStore + 'static) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app
}

fn frame(app: &mut CadApp, ctx: &egui::Context, time: f64) {
    let raw = egui::RawInput { time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| app.logic(ui.ctx()));
    out.textures_delta.clear();
}

fn layout(app: &CadApp) -> LayoutSave {
    LayoutSave::from_ws(&app.ws)
}

fn install_writer() {
    set_io(IoHooks { read: |_, _| Ok(Drawing::new_metric()), write: |_, _| Ok(b"0\nEOF\n".to_vec()), plot: None });
}

fn drawing_path(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cadcraft-save-state-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.dxf")).to_string_lossy().to_string();
    (dir, path)
}

#[test]
fn debounce_is_600_ms() {
    let mut autosave = Autosave::default();
    assert_eq!(autosave.debounce_secs, 0.6);
    autosave.changed(0.0);
    assert!(!autosave.due(0.59));
    assert!(autosave.due(0.6));
}

#[test]
fn layout_state_walks_clean_dirty_saving_saved() {
    let _aliases = alias_lock();
    let mut app = attached(Scripted::new([Durability::Pending, Durability::Committed]));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    let mut seen = vec![layout(&app)];
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    seen.push(layout(&app));
    app.commit_workspace_now().unwrap();
    seen.push(layout(&app));
    assert_eq!(app.workspace_status()["layout_save"], "saving_local");
    assert_ne!(app.ws.status, SaveStatus::SavedLocally, "an accepted write is not saved until it is durable");
    frame(&mut app, &ctx, 1.1);
    seen.push(layout(&app));
    let states: Vec<&str> = seen.iter().map(LayoutSave::as_str).collect();
    assert_eq!(states, ["clean", "dirty", "saving_local", "saved_local"]);
    assert!(!seen.iter().any(|state| matches!(state, LayoutSave::SyncPending | LayoutSave::Syncing | LayoutSave::Synced)));
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
    assert_eq!(app.workspace_status()["layout_save"], "saved_local");
}

#[test]
fn pending_write_is_not_reported_saved() {
    let _aliases = alias_lock();
    let mut app = attached(Scripted::new([Durability::Pending]));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    app.commit_workspace_now().unwrap();
    for time in [1.1, 2.0, 10.0, 60.0] {
        frame(&mut app, &ctx, time);
        let state = layout(&app);
        assert_eq!(state, LayoutSave::SavingLocal, "at {time}");
        assert!(!state.label().to_lowercase().contains("saved"), "{}", state.label());
        assert!(!state.short().to_lowercase().contains("saved"), "{}", state.short());
        assert_ne!(app.workspace_status()["status"], "saved_locally");
    }
}

#[test]
fn failed_durability_arms_retry() {
    let _aliases = alias_lock();
    let mut app = attached(Scripted::new([Durability::Failed("QuotaExceededError".into())]));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    let error = app.commit_workspace_now().unwrap_err();
    assert!(error.contains("QuotaExceededError"), "{error}");
    assert_eq!(layout(&app), LayoutSave::Retry("QuotaExceededError".into()));
    assert_eq!(layout(&app).label(), "Could not save layout; retrying");
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "{:?}", app.ws.status);
    assert!(app.ws.needs_flush(), "the change still has to reach storage");
    assert!(app.ws.autosave.pending(), "another attempt is armed");
    assert_eq!(app.ws.autosave.wait_secs(), 5.0);
    // The retry fails the same way and waits twice as long.
    frame(&mut app, &ctx, 6.1);
    assert_eq!(layout(&app), LayoutSave::Retry("QuotaExceededError".into()));
    assert_eq!(app.ws.autosave.wait_secs(), 10.0);
}

#[test]
fn conflict_durability_becomes_tab_conflict() {
    let _aliases = alias_lock();
    let mut app = attached(Scripted::new([Durability::Conflict { stored: 9 }]));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    let error = app.commit_workspace_now().unwrap_err();
    assert!(error.contains("changed in another tab"), "{error}");
    assert_eq!(app.ws.status, SaveStatus::TabConflict);
    assert_eq!(layout(&app), LayoutSave::Conflict);
    assert_eq!(app.ui.dialog.as_deref(), Some("workspace_tab_conflict"));
    assert!(!app.ws.autosave.pending());
    assert_eq!(app.workspace_status()["layout_save"], "conflict");
}

#[test]
fn drawing_and_layout_labels_are_separate() {
    let _aliases = alias_lock();
    install_writer();
    let (dir, path) = drawing_path("Separate");
    let mut app = attached(MemoryStore::default());
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Clean);
    assert_eq!(layout(&app), LayoutSave::Clean);

    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Dirty);
    assert_eq!(layout(&app), LayoutSave::Clean, "a drawing edit is no layout change");

    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    assert_eq!(layout(&app), LayoutSave::Dirty);
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Dirty, "a layout edit leaves the drawing as it was");

    app.run("saveas", json!({"path": path})).unwrap();
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Saved);
    assert_eq!(layout(&app), LayoutSave::Dirty, "saving the drawing does not store the layout");
    assert_eq!(DrawingSave::from_app(&app).label(), "Drawing saved");
    assert_eq!(layout(&app).label(), "Layout changed");
    let status = app.workspace_status();
    assert_eq!((status["drawing_save"].as_str(), status["layout_save"].as_str()), (Some("saved"), Some("dirty")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn downloaded_drawing_is_labelled_downloaded() {
    let _aliases = alias_lock();
    install_writer();
    let (dir, path) = drawing_path("Downloaded");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let mut saved = app.run("saveas", json!({"path": path})).unwrap();
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Saved);

    // Only the web build's save result names a download; native saves stand in for it here.
    saved["download"] = json!(true);
    app.record_drawing_save(&saved);
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Downloaded);
    assert_eq!(DrawingSave::from_app(&app).label(), "Drawing downloaded");
    assert_eq!(app.workspace_status()["drawing_save"], "downloaded");

    // An export reports another path and leaves the label alone.
    app.record_drawing_save(&json!({"path": "elsewhere.pdf", "bytes": 1}));
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Downloaded);

    app.run("line", json!({"points": [[0, 5], [10, 5]]})).unwrap();
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Dirty);
    app.run("qsave", json!({})).unwrap();
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Saved, "a save to a file is no download");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_bar_shows_both_labels_with_full_tooltips() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    app.ui.in_window_menu = false;
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 760.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    harness.state_mut().run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    harness.state_mut().run("ui.cmdline.lines", json!({"lines": 7})).unwrap();
    harness.state_mut().commit_workspace_now().unwrap();
    harness.run_steps(3);
    let drawing = harness.get_by_label("Drawing has unsaved changes").rect();
    let layout = harness.get_by_label("Layout saved").rect();
    assert!(drawing.right() < layout.left(), "the drawing state sits left of the layout state: {drawing:?} {layout:?}");
    harness.get_by_label("Layout saved").hover();
    harness.run_steps(6);
    assert!(harness.query_by_label("Layout saved on this device (not synced)").is_some(), "the tooltip has the full sentence");
}
