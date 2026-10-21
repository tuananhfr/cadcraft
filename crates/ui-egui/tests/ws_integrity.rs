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
use egui_kittest::kittest::{Queryable, by};
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
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Show Properties").click();
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
    let edge = h.get_by_role_and_label(egui::accesskit::Role::Button, "Show Properties").rect().center();
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
    assert!(h.get_by_label("Pin panel").is_focused());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Show Properties").focus();
    h.run_steps(2);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Show Properties").is_focused());
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
    h.get_by_label("Load Application...").click();
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

#[test]
fn autohide_keeps_other_panels_tree() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.dock", json!({"operation": "move", "panel": "layers", "target": "toolSets", "zone": "center", "before": "toolSets"})).unwrap();
    app.run("ui.dock", json!({"operation": "activate", "panel": "toolSets"})).unwrap();
    app.run("ui.dock", json!({"operation": "resizeSplit", "path": [], "size": {"FixedFirst": 256.0}})).unwrap();
    let before = app.ui.docking.clone();
    let others: Vec<_> = app.ws.effective.layout.panels.iter().filter(|panel| panel.id != "properties").cloned().collect();
    let mut hidden = before.clone();
    hidden.set_visible(Panel::Properties, false).unwrap();
    app.run("ui.panel.autohide", json!({"panel": "properties"})).unwrap();
    assert_eq!(app.ui.docking.layout, hidden.layout, "only Properties leaves the live tree");
    assert!(app.ws.user.layout.dock_tree.is_some());
    let mut app = restart(&mut app);
    assert_eq!(app.ui.docking.layout, hidden.layout, "the remaining tree survives restart");
    app.run("ui.panel.pin", json!({"panel": "properties"})).unwrap();
    assert_eq!(app.ui.docking.layout, before.layout, "all split sizes, tab order and active tabs survive pinning");
    assert_eq!(app.ws.effective.layout.panels.iter().filter(|panel| panel.id != "properties").cloned().collect::<Vec<_>>(), others);
    assert_eq!(restart(&mut app).ui.docking.layout, before.layout);
}

#[test]
fn pin_docked_panel_is_noop() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.dock", json!({"operation": "move", "panel": "layers", "target": "toolSets", "zone": "center"})).unwrap();
    let before = app.ws.user.layout.clone();
    let tree = app.ui.docking.layout.clone();
    let history = (app.ws.history.undo_len(), app.ws.history.redo_len());
    app.run("ui.panel.pin", json!({"panel": "properties", "side": "left"})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    assert_eq!(app.ui.docking.layout, tree);
    assert_eq!((app.ws.history.undo_len(), app.ws.history.redo_len()), history);
}

#[test]
fn autohide_hover_works_in_steady_unfocused_window() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.panel.autohide", json!({"panel": "properties"})).unwrap();
    let mut h = harness(app);
    h.input_mut().focused = false;
    h.event(Event::WindowFocused(false));
    h.run_steps(2);
    let edge = h.get_by_role_and_label(egui::accesskit::Role::Button, "Show Properties").rect().center();
    h.event(Event::PointerMoved(edge));
    h.run_steps(40);
    assert!(!h.ctx.input(|input| input.focused));
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
}

#[test]
fn autohide_popup_keeps_overlay_open() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.panel.autohide", json!({"panel": "properties"})).unwrap();
    let mut h = harness(app);
    // The first frame reapplies the workspace for its actual size and clears temporary peeks.
    h.state_mut().run("ui.panel.peek", json!({"panel": "properties"})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "■ ByLayer").click();
    h.run_steps(3);
    let item = h.get_by_role_and_label(egui::accesskit::Role::Button, "Red").rect().center();
    h.event(Event::PointerMoved(item));
    h.run_steps(40);
    assert!(h.ctx.any_popup_open());
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    h.event(Event::PointerGone);
    h.run_steps(40);
    assert_eq!(h.state().autohide.active, Some(Panel::Properties), "a body popup suspends pointer dismissal");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.ctx.any_popup_open());
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    h.run_steps(40);
    assert!(h.state().autohide.active.is_none());
}

#[test]
fn autohide_text_edit_keeps_overlay_open() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.panel.autohide", json!({"panel": "quickCalc"})).unwrap();
    let mut h = harness(app);
    h.state_mut().run("ui.panel.peek", json!({"panel": "quickCalc"})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().autohide.active, Some(Panel::QuickCalc));
    // QuickCalc's main editor precedes the separate Variables expression editor.
    h.get_all(by().role(egui::accesskit::Role::TextInput).predicate(|node| node.placeholder() == Some("Expression"))).next().unwrap().focus();
    h.run_steps(2);
    let expression =
        h.get_all(by().role(egui::accesskit::Role::TextInput).predicate(|node| node.placeholder() == Some("Expression"))).next().unwrap();
    assert!(expression.is_focused());
    h.event(Event::PointerGone);
    h.run_steps(40);
    assert!(h.ctx.text_edit_focused());
    assert_eq!(h.state().autohide.active, Some(Panel::QuickCalc));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Show QuickCalc").focus();
    h.run_steps(2);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Show QuickCalc").is_focused());
    assert!(h.state().autohide.active.is_none());
}

#[test]
fn parameter_only_panel_commands_are_hidden_but_callable() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    for id in ["ui.panel.autohide", "ui.panel.pin", "ui.panel.peek"] {
        assert!(menus::is_ui_command(id));
        assert!(cadcraft_ui_egui::quick_access::search(&app, id).is_empty());
        assert!(app.run(id, json!({})).is_err(), "panel is required");
    }
    let suggestions = cadcraft_ui_egui::quick_access::search(&app, "ui.panel.");
    assert!(suggestions.iter().any(|entry| entry.id.eq_ignore_ascii_case("ui.panel.fonts")));
    for id in ["ui.panel.autohide", "ui.panel.pin", "ui.panel.peek"] {
        assert!(suggestions.iter().all(|entry| !entry.id.eq_ignore_ascii_case(id)));
    }
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Keyboard").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").focus();
    h.run_steps(2);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").is_focused());
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").type_text("ui.panel.");
    h.run_steps(2);
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").value().as_deref(), Some("ui.panel."));
    assert!(h.query_by_label("Fonts (ui.panel.fonts)").is_some());
    for label in ["Auto-hide (ui.panel.autohide)", "Pin panel (ui.panel.pin)", "Show Auto-hide Panel (ui.panel.peek)"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    h.state_mut().run("ui.panel.autohide", json!({"panel": "properties"})).unwrap();
    h.state_mut().run("ui.panel.peek", json!({"panel": "properties"})).unwrap();
    assert_eq!(h.state().autohide.active, Some(Panel::Properties));
    h.state_mut().run("ui.panel.pin", json!({"panel": "properties"})).unwrap();
    assert!(h.state().ui.docking.layout.contains(&Panel::Properties));
}

#[test]
fn window_menu_has_one_toolsets_entry_and_legacy_toggle_works() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let tree = menus::tree(&app);
    let window = &tree.iter().find(|(label, _)| label == "Window").unwrap().1;
    assert!(window.iter().all(|entry| !matches!(entry, Entry::Item { label, .. } if label == "Tool Sets")));
    let panels = window
        .iter()
        .find_map(|entry| match entry {
            Entry::Sub { label, children } if label == "Panels" => Some(children),
            _ => None,
        })
        .unwrap();
    assert_eq!(panels.iter().filter(|entry| matches!(entry, Entry::Item { label, .. } if label == "Tool Sets")).count(), 1);
    let mut h = harness(app);
    h.state_mut().run("ui.toggle.toolsets", json!({"on": false})).unwrap();
    assert!(!h.state().ui.show_toolsets);
    assert!(!h.state().ws.effective.layout.toolsets.visible);
    h.run_steps(2);
    assert!(!h.state().ui.docking.layout.contains(&Panel::ToolSets));
    h.state_mut().run("ui.toggle.toolsets", json!({"on": true})).unwrap();
    assert!(h.state().ui.show_toolsets);
    assert!(h.state().ws.effective.layout.toolsets.visible);
    h.run_steps(2);
    assert!(h.state().ui.docking.layout.contains(&Panel::ToolSets));
}

#[test]
fn loaded_files_preserve_case_and_exclude_failed_loads() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.session.lisp_load("Tools/MiXeD.LSP", "(princ)", true).unwrap();
    assert!(app.session.lisp_load("SyntaxError.LSP", "(", true).is_err());
    assert!(matches!(app.session.lisp_load("RuntimeError.LSP", "(/ 1 0)", true).unwrap(), Some(Err(_))));
    assert_eq!(lisp::loaded_files(&app.session), ["MiXeD.LSP"]);
    assert!(app.session.lisp_load("Later.LSP", "(getstring \"Value: \")", false).unwrap().is_none());
    assert_eq!(lisp::loaded_files(&app.session), ["MiXeD.LSP"], "waiting is not success");
    app.session.input(cadcraft_engine::Input::Text("done".into())).unwrap();
    assert_eq!(lisp::loaded_files(&app.session), ["Later.LSP", "MiXeD.LSP"]);
    assert!(app.session.lisp_load("Cancelled.LSP", "(getstring \"Value: \")", false).unwrap().is_none());
    assert_eq!(lisp::loaded_files(&app.session), ["Later.LSP", "MiXeD.LSP"], "a previous result cannot complete this pending load");
    app.session.lisp_abort("Function cancelled");
    assert_eq!(lisp::loaded_files(&app.session), ["Later.LSP", "MiXeD.LSP"]);
    app.session.lisp_eval("(defun *error* (msg) (getstring \"Handle: \"))", false, true, Vec::new()).unwrap();
    app.session.lisp_load("HandledError.LSP", "(/ 1 0)", false).unwrap();
    app.session.input(cadcraft_engine::Input::Text("handled".into())).unwrap();
    assert_eq!(lisp::loaded_files(&app.session), ["Later.LSP", "MiXeD.LSP"]);
    app.run("ui.panel.lisp", json!({})).unwrap();
    let h = harness(app);
    assert!(h.query_by_label("MiXeD.LSP").is_some());
    assert!(h.query_by_label("SyntaxError.LSP").is_none());
    assert!(h.query_by_label("RuntimeError.LSP").is_none());
    assert!(h.query_by_label("Cancelled.LSP").is_none());
    assert!(h.query_by_label("HandledError.LSP").is_none());
}

#[test]
fn lisp_load_success_echo_follows_trailing_output() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    for (name, source, trailing, pending) in [
        ("Immediate.LSP", "(princ \"immediate trailing\")", "immediate trailing", false),
        ("Deferred.LSP", "(getstring \"Value: \") (princ \"deferred trailing\")", "deferred trailing", true),
    ] {
        let before = app.session.log.len();
        let result = app.session.lisp_load(name, source, false).unwrap();
        if pending {
            assert!(result.is_none());
            assert!(!lisp::loaded_files(&app.session).iter().any(|file| file == name));
            app.session.input(cadcraft_engine::Input::Text("done".into())).unwrap();
        } else {
            assert!(matches!(result, Some(Ok(_))));
        }
        let output = &app.session.log[before..];
        let success = format!("{name} successfully loaded.");
        let trailing_index = output.iter().position(|line| line == trailing).unwrap();
        let success_index = output.iter().position(|line| line == &success).unwrap();
        assert!(trailing_index < success_index, "the routine's trailing output precedes the success echo");
        assert_eq!(output.iter().filter(|line| *line == &success).count(), 1);
        assert!(lisp::loaded_files(&app.session).iter().any(|file| file == name));
    }
}

#[test]
fn lisp_load_button_stays_visible_with_many_files() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    for index in 0..64 {
        app.session.lisp_load(&format!("Application-{index:02}.LSP"), "(princ)", true).unwrap();
    }
    app.run("ui.panel.lisp", json!({})).unwrap();
    let mut h = harness(app);
    let button = h.get_by_role_and_label(egui::accesskit::Role::Button, "Load Application...").rect();
    assert!(h.ctx.content_rect().contains_rect(button));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Load Application...").click();
    h.run_steps(2);
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("appload"));
}
