use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::schema::{Ribbon, RibbonMode};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

// App restarts share the engine's process-wide alias store.
fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app
}

fn restarted(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), policy: old.read_policy(), snapshot_map, ..MemoryStore::default() })
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
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
fn legacy_ribbon_flags_map_to_modes() {
    for (flags, expected) in [
        (json!({"visible": false}), RibbonMode::Hidden),
        (json!({"visible": false, "collapsed": true}), RibbonMode::Hidden),
        (json!({"visible": true, "collapsed": true}), RibbonMode::TabsOnly),
        (json!({"mode": null, "visible": true, "collapsed": true}), RibbonMode::TabsOnly),
        (json!({"visible": true, "collapsed": false}), RibbonMode::Expanded),
        (json!({"visible": true}), RibbonMode::Expanded),
        (json!({}), RibbonMode::Hidden),
    ] {
        let ribbon: Ribbon = serde_json::from_value(flags).unwrap();
        assert_eq!(ribbon.mode, None);
        assert_eq!(ribbon.effective_mode(), expected);
        assert!(serde_json::to_value(&ribbon).unwrap().get("mode").is_none());
    }
    for mode in [RibbonMode::Expanded, RibbonMode::Compact, RibbonMode::TabsOnly, RibbonMode::Hidden] {
        let ribbon = Ribbon { mode: Some(mode), visible: mode == RibbonMode::Hidden, collapsed: mode != RibbonMode::TabsOnly, ..Ribbon::default() };
        assert_eq!(ribbon.effective_mode(), mode, "explicit mode takes precedence over legacy flags");
    }
}

#[test]
fn set_mode_round_trips_and_keeps_legacy_flags() {
    let mut ribbon: Ribbon = serde_json::from_value(json!({"tabs": [{"id": "home", "label": "Home"}]})).unwrap();
    let tabs = ribbon.tabs.clone();
    for (mode, name, visible, collapsed) in [
        (RibbonMode::Expanded, "expanded", true, false),
        (RibbonMode::Compact, "compact", true, false),
        (RibbonMode::TabsOnly, "tabs_only", true, true),
        (RibbonMode::Hidden, "hidden", false, false),
    ] {
        ribbon.set_mode(mode);
        assert_eq!(ribbon.mode, Some(mode));
        assert_eq!(ribbon.visible, visible);
        assert_eq!(ribbon.collapsed, collapsed);
        assert_eq!(ribbon.tabs, tabs);
        let raw = serde_json::to_string(&ribbon).unwrap();
        let restored: Ribbon = serde_json::from_str(&raw).unwrap();
        assert_eq!(restored, ribbon);
        assert_eq!(restored.effective_mode(), mode);
        assert_eq!(serde_json::to_value(&restored).unwrap()["mode"], name);
    }
}

#[test]
fn ribbon_mode_command_persists_across_restart() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|(id, label, path, shortcut)| {
        *id == "ui.ribbon.mode" && *label == "Ribbon Display Mode" && *path == ["View", "Ribbon", "Display Mode"].as_slice() && shortcut.is_none()
    }));
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.ribbon.mode", json!({"mode": "compact"})).unwrap()["changed"], true);
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.ribbon.mode", json!({"mode": "compact"})).unwrap()["changed"], false);
    assert_eq!(app.ws.history.undo_len(), steps);
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.ribbon.mode, Some(RibbonMode::Compact));
    assert_eq!(restored.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert!(restored.ws.effective.layout.ribbon.visible);
    assert!(!restored.ws.effective.layout.ribbon.collapsed);
    assert!(!app.session.state().unwrap().is_dirty());
    let h = harness(restored);
    assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert!(h.query_by_label("Home").is_some());
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_mode_rejects_unknown_value_and_respects_lock() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [json!({"mode": "giant"}), json!({"mode": "Expanded"}), json!({"mode": null}), json!({"mode": 1}), json!({})] {
        assert_eq!(app.run("ui.ribbon.mode", params).unwrap_err(), "mode must be expanded, compact, tabs_only or hidden");
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    for pointer in ["/ribbon", "/ribbon/mode", "/ribbon/visible", "/ribbon/collapsed"] {
        let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": [pointer]}]});
        let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
        let before = app.ws.user.layout.clone();
        let effective = app.ws.effective.layout.ribbon.clone();
        let steps = app.ws.history.undo_len();
        for (command, params) in [("ui.ribbon.mode", json!({"mode": "tabs_only"})), ("ui.ribbon.collapse", json!({"collapsed": true}))] {
            assert_eq!(app.run(command, params).unwrap_err(), LOCKED_MESSAGE, "{command}: {pointer}");
            assert_eq!(app.ws.user.layout, before);
            assert_eq!(app.ws.effective.layout.ribbon, effective);
            assert_eq!(app.ws.history.undo_len(), steps);
            assert!(!app.session.state().unwrap().is_dirty());
        }
    }
}

#[test]
fn collapse_command_maps_to_tabs_only() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    for mode in ["expanded", "compact", "tabs_only", "hidden"] {
        app.run("ui.ribbon.mode", json!({"mode": mode})).unwrap();
        app.run("ui.ribbon.collapse", json!({"collapsed": true})).unwrap();
        let ribbon = &app.ws.effective.layout.ribbon;
        assert_eq!(ribbon.mode, Some(RibbonMode::TabsOnly));
        assert!(ribbon.visible && ribbon.collapsed);
        app.run("ui.ribbon.collapse", json!({"collapsed": false})).unwrap();
        let ribbon = &app.ws.effective.layout.ribbon;
        assert_eq!(ribbon.mode, Some(RibbonMode::Expanded));
        assert!(ribbon.visible && !ribbon.collapsed);
    }
    app.run("ui.ribbon.collapse", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::TabsOnly);
    app.run("ui.ribbon.collapse", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Expanded);
    assert!(!app.session.state().unwrap().is_dirty());
}
