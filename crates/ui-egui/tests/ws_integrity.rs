//! Workspace panels and switches preserve drawing and application execution state.

use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::{Session, lisp};
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::menus::{self, Entry};
use cadcraft_ui_egui::wsprofile::presets::PRESET_IDS;
use cadcraft_ui_egui::wsprofile::schema::{Dock, PanelMode};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, autohide, fontmgr};
use craft_ui::docking::Placement;
use egui::{Event, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

static LOCK: Mutex<()> = Mutex::new(());

fn test_lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("ui.sample", json!({})).unwrap();
    app.canvas.zoom_pending = false;
    app
}

fn restart(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), policy: old.read_policy(), snapshot_map, ..MemoryStore::default() })
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

#[test]
fn font_panel_docks_and_matches_dialog() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.panel.fonts", json!({})).unwrap();
    assert!(app.ui.docking.layout.contains(&Panel::Fonts));
    assert!(!app.ws.report.unknown_panels.iter().any(|id| id == "fonts"));
    let mut h = harness(app);
    assert!(h.query_by_label("In this drawing").is_some());
    assert!(h.query_by_label("Fonts you added").is_some());
    let panel_report = fontmgr::report(h.state_mut());
    assert_eq!(panel_report, cadcraft_engine::cmd::fonts::report(&h.state().session));
    h.state_mut().run("ui.dock", json!({"operation": "close", "panel": "fonts"})).unwrap();
    h.state_mut().run("fonts", Value::Null).unwrap();
    h.run_steps(5);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("fonts"));
    assert!(h.query_by_label("In this drawing").is_some());
    assert!(h.query_by_label("Fonts you added").is_some());
    assert_eq!(fontmgr::report(h.state_mut()), panel_report);
}

#[test]
fn profile_import_never_runs_lisp_ux12() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.session.lisp_load("already-loaded.lsp", "(setq w10_guard 42)", true).unwrap();
    let files = lisp::loaded_files(&app.session);
    assert_eq!(files, ["already-loaded.lsp"]);
    let history = app.session.log.clone();
    let drawing = app.session.doc().unwrap().clone();
    for payload in [json!({"layout": {"extra_startup": "acad.lsp"}}), json!({"x": "(load \"a.lsp\")"})] {
        assert!(app.run("ui.workspace.import", json!({"text": payload.to_string()})).is_err());
        assert_eq!(lisp::loaded_files(&app.session), files);
        assert_eq!(app.session.log, history, "import cannot dispatch commands");
        assert!(app.session.recent_commands(100).iter().all(|id| id != "load" && id != "appload"));
        assert_eq!(*app.session.doc().unwrap(), drawing);
        assert!(!app.session.state().unwrap().is_dirty());
    }
}

#[test]
fn autohide_round_trip() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Properties").click_button(PointerButton::Secondary);
    h.run_steps(2);
    h.get_by_label("Auto-hide").click();
    h.run_steps(2);
    let app = h.state();
    assert!(!app.ui.docking.layout.contains(&Panel::Properties));
    assert!(autohide::edge_panels(app).iter().any(|panel| panel.id == "properties" && panel.side == Some(Dock::Right)));
    let app = restart(h.state_mut());
    assert!(app.autohide.active.is_none(), "a peek is not persisted");
    assert!(!app.ui.docking.layout.contains(&Panel::Properties));
    assert_eq!(app.ws.effective.layout.panels.iter().find(|panel| panel.id == "properties").unwrap().mode, PanelMode::AutoHide);
    let drawing = app.session.doc().unwrap().clone();
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Properties").click();
    h.run_steps(3);
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    assert!(h.query_by_label("Pin panel").is_some());
    h.get_by_label("Pin panel").click();
    h.run_steps(3);
    assert!(h.state().ui.docking.layout.contains(&Panel::Properties));
    assert!(autohide::edge_panels(h.state()).iter().all(|panel| panel.id != "properties"));
    assert_eq!(h.state().ws.effective.layout.panels.iter().find(|panel| panel.id == "properties").unwrap().side, Some(Dock::Right));
    assert_eq!(*h.state().session.doc().unwrap(), drawing);
    assert!(!h.state().session.state().unwrap().is_dirty());
    let app = restart(h.state_mut());
    assert!(app.ui.docking.layout.contains(&Panel::Properties));
    assert_eq!(app.ws.effective.layout.panels.iter().find(|panel| panel.id == "properties").unwrap().mode, PanelMode::Dock);
}

#[test]
fn autohide_hover_delay_and_focus_leave_close_overlay() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.panel.autohide", json!({"panel": "properties"})).unwrap();
    let mut h = harness(app);
    let edge = h.get_by_role_and_label(egui::accesskit::Role::Button, "Properties").rect().center();
    h.event(Event::PointerMoved(edge));
    h.run_steps(3);
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    h.event(Event::PointerGone);
    h.run_steps(20);
    assert!(h.state().autohide.active.is_some(), "less than 0.5 seconds");
    h.run_steps(15);
    assert!(h.state().autohide.active.is_none(), "leaving for 0.5 seconds closes the peek");
    h.state_mut().run("ui.panel.peek", json!({"panel": "properties"})).unwrap();
    h.run_steps(2);
    let pin = h.get_by_label("Pin panel").rect().center();
    h.event(Event::PointerMoved(pin));
    h.run_steps(2);
    h.get_by_label("Pin panel").focus();
    h.run_steps(2);
    h.get_by_label("Command line input").focus();
    h.run_steps(2);
    assert!(h.state().autohide.active.is_none(), "keyboard focus left the overlay");
    h.state_mut().run("ui.panel.peek", json!({"panel": "properties"})).unwrap();
    h.run_steps(2);
    // Window focus loss must close even while the pointer remains inside the overlay.
    h.event(Event::WindowFocused(false));
    h.run_steps(2);
    assert!(h.state().autohide.active.is_none());
}

#[test]
fn switch_100_times_keeps_drawing_ux13() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let drawing = app.session.doc().unwrap().clone();
    let entity_count = drawing.model.len();
    let layer = drawing.header.str("CLAYER", "").to_string();
    let view = app.session.state().unwrap().view();
    assert!(!app.session.state().unwrap().is_dirty());
    for id in PRESET_IDS.iter().cycle().take(100) {
        app.run("ui.workspace.select", json!({"id": id})).unwrap();
        let state = app.session.state().unwrap();
        assert!(!state.is_dirty(), "{id}");
        assert_eq!(state.doc.model.len(), entity_count, "{id}");
        assert_eq!(state.doc.header.str("CLAYER", ""), layer, "{id}");
        assert_eq!(state.doc.header, drawing.header, "UCS and all drawing variables: {id}");
        assert_eq!(state.view(), view, "{id}");
    }
    assert_eq!(*app.session.doc().unwrap(), drawing);
}

#[test]
fn sheetset_placeholder() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.set", json!({"pointer": "/panels/sheetset", "value": {"id": "sheetset", "mode": "dock", "side": "left"}})).unwrap();
    let tree = menus::tree(&app);
    let window = &tree.iter().find(|(label, _)| label == "Window").unwrap().1;
    let panels = window
        .iter()
        .find_map(|entry| match entry {
            Entry::Sub { label, children } if label == "Panels" => Some(children),
            _ => None,
        })
        .unwrap();
    assert!(panels.iter().any(|entry| matches!(
        entry,
        Entry::Item { label, enabled: false, .. } if label == "Sheet Set (not available in this version)"
    )));
    assert!(app.ws.report.unknown_panels.iter().any(|id| id == "sheetset"));
    let restored = restart(&mut app);
    assert!(restored.ws.effective.layout.panels.iter().any(|panel| panel.id == "sheetset"));
    assert!(restored.ws.report.unknown_panels.iter().any(|id| id == "sheetset"));
}

#[test]
fn moving_unlocked_panel_keeps_locked_neighbour() {
    let _guard = test_lock();
    for locked in ["properties", "layers"] {
        let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": [format!("/panels/{locked}")]}]});
        let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
        let before = app.ws.effective.layout.panels.iter().find(|panel| panel.id == locked).unwrap().clone();
        let moving = if locked == "properties" { "layers" } else { "properties" };
        app.run("ui.dock", json!({"operation": "move", "panel": moving, "target": "toolSets", "zone": "center"})).unwrap();
        assert_ne!(app.ws.message.as_deref(), Some("Locked by company policy"));
        assert_eq!(app.ws.effective.layout.panels.iter().find(|panel| panel.id == locked), Some(&before));
        assert!(app.ws.user.layout.dock_tree.is_some(), "an allowed move keeps its tree");
        let moved = Panel::parse(moving).unwrap();
        assert!(matches!(app.ui.docking.layout.location(&moved).unwrap().placement, Placement::Tab { .. }));
        assert_eq!(app.ui.docking.layout.location(&moved).unwrap().anchor, Some(Panel::ToolSets));
        let restored = restart(&mut app);
        assert_eq!(restored.ws.effective.layout.panels.iter().find(|panel| panel.id == locked), Some(&before));
        assert_eq!(restored.ui.docking.layout.location(&moved).unwrap().anchor, Some(Panel::ToolSets));
    }
}

#[test]
fn lisp_panel_lists_engine_files_and_starts_appload() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.session.lisp_load("zeta.lsp", "(princ)", true).unwrap();
    app.session.lisp_load("alpha.lsp", "(princ)", true).unwrap();
    assert_eq!(lisp::loaded_files(&app.session), ["alpha.lsp", "zeta.lsp"]);
    app.run("ui.panel.lisp", json!({})).unwrap();
    assert!(app.ui.docking.layout.contains(&Panel::Lisp));
    let mut h = harness(app);
    assert!(h.query_by_label("alpha.lsp").is_some());
    assert!(h.query_by_label("zeta.lsp").is_some());
    assert!(h.query_by_label("Startup files load from <settings>/support only").is_some());
    h.get_by_label("Load Application…").click();
    h.run_steps(2);
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("appload"));
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn autohide_retains_each_edge_and_obeys_policy() {
    let _guard = test_lock();
    for (name, side) in [("left", Dock::Left), ("right", Dock::Right), ("top", Dock::Top), ("bottom", Dock::Bottom)] {
        let mut app = attached(MemoryStore::default());
        app.run("ui.panel.autohide", json!({"panel": "properties", "side": name})).unwrap();
        let mut restored = restart(&mut app);
        assert_eq!(autohide::edge_panels(&restored).iter().find(|panel| panel.id == "properties").unwrap().side, Some(side));
        restored.run("ui.panel.pin", json!({"panel": "properties"})).unwrap();
        assert_eq!(restored.ws.effective.layout.panels.iter().find(|panel| panel.id == "properties").unwrap().side, Some(side));
        assert!(restored.ui.docking.layout.contains(&Panel::Properties));
    }
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/panels/properties"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    let before = app.ws.effective.layout.clone();
    assert!(app.run("ui.panel.autohide", json!({"panel": "properties"})).unwrap_err().contains("Locked by company policy"));
    assert_eq!(app.ws.effective.layout, before);
    assert!(app.ui.docking.layout.contains(&Panel::Properties));
}

#[test]
fn moving_unlocked_panel_gives_space_to_unlocked_sibling() {
    let _guard = test_lock();
    let policy = json!({"layers": [{
        "level": "company",
        "patch": {"panels": [
            {"id": "layers", "height_ratio": 0.4},
            {"id": "properties", "height_ratio": 0.4},
            {"id": "fonts", "mode": "dock", "side": "right", "width_ratio": 0.21, "height_ratio": 0.2}
        ]},
        "locks": ["/panels/properties"]
    }]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    let before = app.ws.effective.layout.panels.iter().find(|panel| panel.id == "properties").unwrap().clone();
    app.run("ui.dock", json!({"operation": "move", "panel": "layers", "target": "toolSets", "zone": "center"})).unwrap();
    assert_ne!(app.ws.message.as_deref(), Some("Locked by company policy"));
    assert_eq!(app.ws.effective.layout.panels.iter().find(|panel| panel.id == "properties"), Some(&before));
    let fonts = app.ws.effective.layout.panels.iter().find(|panel| panel.id == "fonts").unwrap();
    assert!((fonts.height_ratio - 0.6).abs() < 0.00001, "the unlocked sibling receives the freed space");
}
