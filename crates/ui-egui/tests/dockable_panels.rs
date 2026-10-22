//! Blocks and command history use the shared docking and workspace persistence paths.

use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::doc::EntityKind;
use cadcraft_engine::{Input, Session};
use cadcraft_ui_egui::docking::{Panel, Workspace};
use cadcraft_ui_egui::menus::UI_COMMANDS;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use craft_ui::docking::{Placement, Zone};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.ui.start_tab = false;
    app.ui.in_window_menu = false;
    app
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn double_click(h: &mut Harness<'_, CadApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(2);
}

#[test]
fn blocks_panel_docks_and_inserts() {
    let _guard = test_lock();
    let mut app = attached();
    let result = app.run("circle", json!({"center":[0,0], "radius":1})).unwrap();
    app.run("block", json!({"name":"A", "base":[0,0], "handles":[result["handle"]], "keep":"delete"})).unwrap();
    let drawing = app.session.doc().unwrap().clone();
    app.run("ui.panel.blocks", json!({})).unwrap();
    assert!(app.ui.docking.layout.contains(&Panel::Blocks));
    assert!(app.ui.docking.layout.floating.is_empty());
    app.ui.docking.validate().unwrap();
    assert_eq!(app.session.doc().unwrap(), &drawing);
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "A").click();
    h.run_steps(3);
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("insert"));
    assert_eq!(h.state().session.current_prompt().unwrap().message, "Specify insertion point");
    assert_eq!(h.state().ui.dialog, None);
    h.state_mut().session.input(Input::Point(cadcraft_engine::geom::Vec2::new(5.0, 6.0))).unwrap();
    for _ in 0..3 {
        h.state_mut().session.input(Input::Enter).unwrap();
    }
    assert!(h.state().session.doc().unwrap().model.iter().any(|entity| matches!(&entity.kind, EntityKind::Insert(insert) if insert.block == "A")));
}

#[test]
fn history_panel_lists_recent_commands() {
    let _guard = test_lock();
    let mut app = attached();
    app.session.log.clear();
    assert_eq!(app.cmdline("line 0,0 10,0"), None);
    app.session.input(Input::Enter).unwrap();
    assert_eq!(app.cmdline("circle 20,20 5"), None);
    assert!(app.session.running.is_none());
    assert_eq!(app.session.log.iter().filter(|line| line.starts_with("Command: ")).count(), 2);
    app.run("ui.panel.history", json!({})).unwrap();
    assert!(!app.cmd.expanded);
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Command: LINE");
    let pos = h.get_by_role_and_label(egui::accesskit::Role::Button, "Command: CIRCLE").rect().center();
    let drawing = h.state().session.doc().unwrap().clone();
    double_click(&mut h, pos);
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("circle"));
    assert_eq!(h.state().session.doc().unwrap(), &drawing);
    assert!(h.state().ui.docking.layout.contains(&Panel::History));
    assert!(!h.state().cmd.expanded);
}

#[test]
fn new_panels_survive_restart_and_validate() {
    let _guard = test_lock();
    let mut app = attached();
    for (panel, id, label) in [(Panel::Blocks, "blocks", "Blocks"), (Panel::History, "history", "Command History")] {
        assert_eq!(panel.id(), id);
        assert_eq!(panel.label(), label);
        assert_eq!(Panel::parse(id).unwrap(), panel);
        assert_eq!(serde_json::from_value::<Panel>(json!(id)).unwrap(), panel);
        app.run(&format!("ui.panel.{id}"), json!({})).unwrap();
        app.run("ui.dock", json!({"operation":"move", "panel":id, "target":"canvas", "zone":"right"})).unwrap();
        app.ui.docking.validate().unwrap();
        assert_eq!(app.ui.docking.layout.location(&panel).unwrap().placement, Placement::Split(Zone::Right));
    }
    app.commit_workspace_now().unwrap();
    let store = app.ws.store.take().unwrap();
    let mut again = CadApp::new(Session::new(), Services::default());
    again.skip_first_run_picker = true;
    again.attach_profile_store(store);
    again.ui.start_tab = false;
    again.ui.docking.validate().unwrap();
    for panel in [Panel::Blocks, Panel::History] {
        assert!(again.ui.docking.layout.contains(&panel));
        assert_eq!(again.ui.docking.layout.location(&panel).unwrap().placement, Placement::Split(Zone::Right));
        assert!(!again.ws.report.unknown_panels.contains(&panel.id().to_string()));
    }
    let restored: Workspace = serde_json::from_str(&serde_json::to_string(&again.ui.docking).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.layout, again.ui.docking.layout);
    for size in [vec2(1366.0, 768.0), vec2(683.0, 384.0)] {
        for panel in [Panel::Blocks, Panel::History] {
            let mut workspace = restored.clone();
            workspace.apply(craft_ui::docking::Action::Float { panel, rect: [-500.0, -500.0, 5000.0, 5000.0] }).unwrap();
            let area = egui::Rect::from_min_size(Pos2::ZERO, size);
            assert!(cadcraft_ui_egui::dock_clamp::clamp_layout(&mut workspace.layout, area));
            workspace.validate().unwrap();
            let [x, y, width, height] = workspace.layout.floating.iter().find(|group| group.panels.contains(&panel)).unwrap().rect;
            assert!(area.contains_rect(egui::Rect::from_min_size(egui::pos2(x, y), vec2(width, height))));
            assert!(width <= size.x * cadcraft_ui_egui::dock_clamp::MAX_FLOAT_SHARE);
            assert!(height <= size.y * cadcraft_ui_egui::dock_clamp::MAX_FLOAT_SHARE);
        }
    }
}

#[test]
fn legacy_blocks_dialog_id_still_works() {
    let _guard = test_lock();
    let mut app = attached();
    assert!(UI_COMMANDS.iter().any(|entry| entry.0 == "ui.dialog.blocks"));
    for id in ["ui.dialog.blocks", "ui.panel.blocks"] {
        app.run(id, json!({})).unwrap();
        assert!(app.ui.docking.layout.contains(&Panel::Blocks));
        assert_eq!(app.ui.dialog, None);
        app.run("ui.dock", json!({"operation":"close", "panel":"blocks"})).unwrap();
    }
    app.run("ui.panel.history", json!({})).unwrap();
    let layout = app.ui.docking.layout.clone();
    app.run("ui.toggle.history", json!({"on":true})).unwrap();
    assert!(app.cmd.expanded);
    app.run("ui.toggle.history", json!({"on":false})).unwrap();
    assert!(!app.cmd.expanded);
    assert_eq!(app.ui.docking.layout, layout);
    assert!(UI_COMMANDS.iter().any(|entry| entry.0 == "ui.toggle.history" && entry.3 == Some("F2")));
}

#[test]
fn sheet_set_stays_a_disabled_placeholder() {
    let _guard = test_lock();
    let mut app = attached();
    assert!(!UI_COMMANDS.iter().any(|entry| entry.0 == "sheetset"));
    assert!(cadcraft_engine::find_command("sheetset").is_none());
    app.run("ui.workspace.set", json!({"pointer":"/toolbars/legacy", "value":{"label":"Legacy", "dock":"top", "row":1, "commands":["sheetset"]}}))
        .unwrap();
    let mut h = harness(app);
    let item = h.get_by_label("Sheet Set — not available in this version");
    assert!(item.accesskit_node().is_disabled());
    item.hover();
    h.run_steps(60);
    h.get_by_label("Planned");
    assert!(h.state().ws.report.unknown_commands.contains(&"sheetset".to_string()));
}
