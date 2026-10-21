//! Data-driven tool bars: docking to the four edges, floating, hiding, locking and reordering,
//! through real pointer and key input on the app (UX02, UX11, UX14).

use cadcraft_engine::Session;
use cadcraft_ui_egui::toolbars::{self, DragPhase, DropTarget};
use cadcraft_ui_egui::wsprofile::SaveStatus;
use cadcraft_ui_egui::wsprofile::schema::{Dock, ToolbarState};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    // The Window menu is part of some tests; the default differs by platform.
    app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
    app
}

fn harness_sized(app: CadApp, width: f32, height: f32) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(width, height)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    harness_sized(app, 1366.0, 768.0)
}

fn run(app: &mut CadApp, id: &str, params: Value) -> Value {
    app.run(id, params.clone()).unwrap_or_else(|error| panic!("{id} {params}: {error}"))
}

fn bar(app: &CadApp, id: &str) -> ToolbarState {
    app.ws
        .effective
        .layout
        .toolbars
        .iter()
        .find(|bar| bar.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("no toolbar {id}: {:?}", app.ws.effective.layout.toolbars))
}

fn grip(h: &Harness<'_, CadApp>, name: &str) -> Pos2 {
    h.get_by_label(&format!("{name} toolbar grip")).rect().center()
}

/// A mouse drag from `from` to `to`, released there when `release` is set.
fn drag(h: &mut Harness<'_, CadApp>, from: Pos2, to: Pos2, release: bool) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    h.hover_at(from + (to - from) * 0.5);
    h.run_steps(2);
    h.hover_at(to);
    h.run_steps(3);
    if release {
        h.drop_at(to);
        h.run_steps(4);
    }
}

fn restarted(app: &CadApp) -> CadApp {
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), snapshot_map, ..MemoryStore::default() })
}

#[test]
fn dock_each_side_and_float_survive_restart_ux02() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.show", json!({"id": "draw"}));
    let mut h = harness(app);
    let history_before = h.state().ws.history.undo_len();
    let targets = [
        (Pos2::new(600.0, 756.0), Dock::Bottom),
        (Pos2::new(6.0, 400.0), Dock::Left),
        (Pos2::new(1362.0, 400.0), Dock::Right),
        (Pos2::new(500.0, 20.0), Dock::Top),
        (Pos2::new(683.0, 380.0), Dock::Floating),
    ];
    for (index, (to, dock)) in targets.into_iter().enumerate() {
        let from = grip(&h, "Draw");
        drag(&mut h, from, to, false);
        assert!(
            matches!(&h.state().ws.toolbar_drag, DragPhase::Dragging { id, target: Some(_), .. } if id == "draw"),
            "{dock:?}: the drag shows a target while the button is down"
        );
        assert_eq!(bar(h.state(), "draw").dock, if index == 0 { Dock::Top } else { targets[index - 1].1 }, "nothing is written during the drag");
        h.drop_at(to);
        h.run_steps(2);
        assert_eq!(h.state().ws.toolbar_drag, DragPhase::Idle);
        assert_eq!(bar(h.state(), "draw").dock, dock);
        if index == 0 {
            // The release is a commit point: it is stored on the next frame, not after the debounce.
            assert_eq!(h.state().ws.status, SaveStatus::SavedLocally);
        }
        h.run_steps(2);
        let restored = restarted(h.state());
        assert_eq!(bar(&restored, "draw").dock, dock, "{dock:?} survives a restart");
        if dock == Dock::Floating {
            let draw = bar(&restored, "draw");
            assert!(draw.x_ratio > 0.05 && draw.x_ratio < 0.95 && draw.y_ratio > 0.05 && draw.y_ratio < 0.95, "{draw:?}");
        }
    }
    assert_eq!(h.state().ws.history.undo_len(), history_before + targets.len(), "one layout step per drag");
}

#[test]
fn escape_cancels_toolbar_drag() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.show", json!({"id": "draw"}));
    let mut h = harness(app);
    let layout = h.state().ws.user.layout.clone();
    let steps = h.state().ws.history.undo_len();
    let start = grip(&h, "Draw");
    drag(&mut h, start, Pos2::new(600.0, 756.0), false);
    assert!(matches!(h.state().ws.toolbar_drag, DragPhase::Dragging { .. }));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().ws.toolbar_drag, DragPhase::Idle);
    h.drop_at(Pos2::new(600.0, 756.0));
    h.run_steps(4);
    assert_eq!(h.state().ws.user.layout, layout);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    assert_eq!(bar(h.state(), "draw").dock, Dock::Top);

    // Without Escape the same gesture does move it.
    let start = grip(&h, "Draw");
    drag(&mut h, start, Pos2::new(600.0, 756.0), true);
    assert_eq!(bar(h.state(), "draw").dock, Dock::Bottom);
    assert_eq!(h.state().ws.history.undo_len(), steps + 1);
}

#[test]
fn hide_show_and_reorder() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.move", json!({"id": "draw", "dock": "top", "row": 0, "order": 1}));
    assert_eq!((bar(&app, "standard").order, bar(&app, "draw").order), (0, 1));
    let mut h = harness(app);

    h.get_by_label("Draw toolbar grip").click_secondary();
    h.run_steps(3);
    h.get_by_label("Hide").click();
    h.run_steps(4);
    assert!(!bar(h.state(), "draw").visible);
    assert!(h.query_by_label("Draw toolbar grip").is_none(), "a hidden bar is not drawn");

    h.get_by_label("Window").click();
    h.run_steps(3);
    h.get_by_label("Toolbars ⏵").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Draw").click();
    h.run_steps(4);
    assert!(bar(h.state(), "draw").visible, "Window > Toolbars shows it again");
    assert!(h.query_by_label("Draw toolbar grip").is_some());

    let draw = grip(&h, "Draw");
    let standard = h.get_by_label("Standard toolbar grip").rect();
    drag(&mut h, draw, Pos2::new(standard.left() + 1.0, standard.center().y), true);
    let (draw, standard) = (bar(h.state(), "draw"), bar(h.state(), "standard"));
    assert_eq!((draw.row, standard.row), (0, 0), "still on the same row");
    assert_eq!((draw.order, standard.order), (0, 1), "draw is now before standard");
    assert!(h.get_by_label("Draw toolbar grip").rect().left() < h.get_by_label("Standard toolbar grip").rect().left());
}

fn company_locks_draw() -> MemoryStore {
    let draw =
        json!({"id": "draw", "label": "Draw", "dock": "top", "row": 1, "visible": true, "commands": toolbars::builtin_toolbar_commands("draw")});
    let policy = json!({"layers": [{"level": "company", "patch": {"toolbars": [draw]}, "locks": ["/toolbars/draw"]}]});
    MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() }
}

#[test]
fn locked_by_policy_ux11() {
    let mut app = attached(company_locks_draw());
    assert!(bar(&app, "draw").visible);
    let error = app.run("ui.toolbar.hide", json!({"id": "draw"})).unwrap_err();
    assert!(error.contains("Locked by company policy"), "{error}");
    for command in ["ui.toolbar.lock", "ui.toolbar.dock", "ui.toolbar.float", "ui.toolbar.move"] {
        let params = json!({"id": "draw", "dock": "left"});
        assert!(app.run(command, params).unwrap_err().contains("Locked by company policy"), "{command}");
    }
    assert!(bar(&app, "draw").visible && bar(&app, "draw").dock == Dock::Top);
    assert!(app.run("ui.toolbar.hide", json!({"id": "standard"})).is_ok(), "other bars are not affected");
    run(&mut app, "ui.toolbar.show", json!({"id": "standard"}));

    let mut h = harness(app);
    h.get_by_label("Draw toolbar grip").click_secondary();
    h.run_steps(3);
    assert!(h.get_by_label("Hide").accesskit_node().is_disabled(), "Hide is greyed out");
    h.key_press(Key::Escape);
    h.run_steps(2);

    // The grip does not start a drag either.
    let start = grip(&h, "Draw");
    drag(&mut h, start, Pos2::new(600.0, 756.0), true);
    assert_eq!(bar(h.state(), "draw").dock, Dock::Top);
    assert_eq!(h.state().ws.toolbar_drag, DragPhase::Idle);
}

#[test]
fn offscreen_toolbar_pulled_back() {
    // The ratios go in literally, through the generic setter, so nothing clamps them on input.
    let floating_draw = |x: f64, y: f64| json!({"label": "Draw", "dock": "floating", "visible": true, "x_ratio": x, "y_ratio": y, "commands": toolbars::builtin_toolbar_commands("draw")});
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.workspace.set", json!({"pointer": "/toolbars/draw", "value": floating_draw(1.4, 1.2)}));
    assert_eq!(bar(&app, "draw").x_ratio, 1.0, "the profile schema itself keeps ratios in 0..=1; the window clamp below is a separate step");
    let h = harness(app);
    let content = egui::Rect::from_min_size(Pos2::ZERO, vec2(1366.0, 768.0));
    let seen = |h: &Harness<'_, CadApp>, content: egui::Rect| {
        // The grip sits 5 pt inside the bar's left and top edges, so the painted bar starts there.
        let grip = h.get_by_label("Draw toolbar grip").rect();
        let bar = egui::Rect::from_min_size(grip.min - vec2(5.0, 5.0), vec2(34.0, 34.0)).intersect(content);
        assert!(grip.intersect(content).is_positive(), "the grip is on screen: {grip:?}");
        assert!(bar.width() >= 24.0 - 0.5 && bar.height() >= 24.0 - 0.5, "{grip:?} leaves {bar:?} of the bar inside {content:?}");
    };
    seen(&h, content);
    assert_eq!(bar(h.state(), "draw").x_ratio, 1.0, "painting does not write anything back");

    // A smaller window pulls it back again without touching what is stored.
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.workspace.set", json!({"pointer": "/toolbars/draw", "value": floating_draw(0.99, 0.99)}));
    let small = harness_sized(app, 800.0, 600.0);
    seen(&small, egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
    assert_eq!(bar(small.state(), "draw").x_ratio, 0.99, "the clamp is never written back");
}

#[test]
fn unknown_command_placeholder() {
    let mut app = attached(MemoryStore::default());
    run(
        &mut app,
        "ui.workspace.set",
        json!({"pointer": "/toolbars/legacy", "value": {"label": "Legacy", "dock": "top", "row": 1, "commands": ["sheetset", "line"]}}),
    );
    let mut h = harness(app);
    let placeholder = h.get_by_label("Sheet Set — not available in this version");
    assert!(placeholder.accesskit_node().is_disabled());
    h.state_mut().commit_workspace_now().unwrap();
    let kept = restarted(h.state());
    assert_eq!(bar(&kept, "legacy").commands, ["sheetset", "line"], "the unknown id is kept when the profile is saved");
    assert!(kept.ws.report.unknown_commands.contains(&"sheetset".to_string()));
}

#[test]
fn toolbar_keyboard_only_ux14() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.show", json!({"id": "draw"}));
    let polar = app.session.settings.polarmode;
    let mut h = harness(app);
    let tab_to = |h: &mut Harness<'_, CadApp>, label: &str| {
        for _ in 0..120 {
            if h.query_by_label(label).is_some_and(|node| node.is_focused()) {
                return;
            }
            h.key_press(Key::Tab);
            h.run_steps(2);
        }
        panic!("Tab did not reach {label}");
    };
    tab_to(&mut h, "Draw toolbar grip");
    h.key_press_modifiers(Modifiers::SHIFT, Key::F10);
    h.run_steps(3);
    assert!(h.get_by_label("Hide").is_focused(), "the menu takes the keyboard");
    assert_eq!(h.state().session.settings.polarmode, polar, "Shift+F10 is not the Polar Tracking key here");
    tab_to(&mut h, "Dock ⏵");
    h.key_press(Key::Enter);
    h.run_steps(3);
    tab_to(&mut h, "Left");
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert_eq!(bar(h.state(), "draw").dock, Dock::Left);
    assert_eq!(h.state().session.settings.polarmode, polar);
}

#[test]
fn commands_edit_the_user_layer_and_undo() {
    let mut app = attached(MemoryStore::default());
    assert!(app.ui.show_toolbar);
    run(&mut app, "ui.toolbar.hide", json!({"id": "standard"}));
    assert!(!app.ui.show_toolbar, "the Standard bar is the old tool bar switch");
    run(&mut app, "ui.layout.undo", json!({}));
    assert!(app.ui.show_toolbar);
    let steps = app.ws.history.undo_len();
    assert_eq!(run(&mut app, "ui.toolbar.show", json!({"id": "standard"}))["changed"], json!(false));
    assert_eq!(app.ws.history.undo_len(), steps, "no change, no step");

    run(&mut app, "ui.toolbar.dock", json!({"id": "modify", "dock": "left"}));
    let modify = bar(&app, "modify");
    assert_eq!((modify.dock, modify.row, modify.order, modify.visible), (Dock::Left, 0, 0, true));
    assert_eq!(modify.commands, toolbars::builtin_toolbar_commands("modify"));
    run(&mut app, "ui.toolbar.lock", json!({"id": "modify"}));
    let error = app.run("ui.toolbar.float", json!({"id": "modify"})).unwrap_err();
    assert!(error.contains("locked"), "{error}");
    run(&mut app, "ui.toolbar.unlock", json!({"id": "modify"}));
    run(&mut app, "ui.toolbar.float", json!({"id": "modify", "x_ratio": 0.3, "y_ratio": 0.4}));
    assert_eq!((bar(&app, "modify").dock, bar(&app, "modify").x_ratio), (Dock::Floating, 0.3));
    assert!(app.run("ui.toolbar.hide", json!({"id": "nonsense"})).is_err());
    assert!(app.run("ui.toolbar.dock", json!({"id": "modify", "dock": "middle"})).is_err());
    assert!(app.run("ui.toolbar.hide", Value::Null).unwrap_err().contains("id is required"));
}

#[test]
fn applying_a_workspace_mid_drag_cancels_the_toolbar_drag() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.show", json!({"id": "draw"}));
    let mut h = harness(app);
    let start = grip(&h, "Draw");
    drag(&mut h, start, Pos2::new(600.0, 756.0), false);
    assert!(matches!(h.state().ws.toolbar_drag, DragPhase::Dragging { .. }));
    // Selecting the active workspace is a no-op, so switch to another one.
    run(h.state_mut(), "ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"}));
    assert_eq!(h.state().ws.toolbar_drag, DragPhase::Idle);
    h.drop_at(Pos2::new(600.0, 756.0));
    h.run_steps(4);
    assert!(h.state().ws.effective.layout.toolbars.iter().all(|bar| bar.id != "draw" || bar.dock != Dock::Bottom));
}

#[test]
fn drop_targets_are_ordered_rows_on_an_edge() {
    let mut bars = vec![
        ToolbarState { id: "a".into(), order: 0, ..ToolbarState::default() },
        ToolbarState { id: "b".into(), order: 1, ..ToolbarState::default() },
        ToolbarState { id: "c".into(), dock: Dock::Left, ..ToolbarState::default() },
    ];
    toolbars::drag::apply_drop(&mut bars, "c", &DropTarget::Edge { dock: Dock::Top, row: 0, order: 1 }, &[]).unwrap();
    let orders: Vec<(&str, u32)> = bars.iter().map(|bar| (bar.id.as_str(), bar.order)).collect();
    assert_eq!(orders, [("a", 0), ("b", 2), ("c", 1)]);
    toolbars::drag::apply_drop(&mut bars, "c", &DropTarget::Edge { dock: Dock::Top, row: 0, order: 99 }, &[]).unwrap();
    let after_b = bars.iter().find(|bar| bar.id == "c").map(|bar| bar.order) > bars.iter().find(|bar| bar.id == "b").map(|bar| bar.order);
    assert!(after_b, "dropped past the end, c is last");
    toolbars::drag::apply_drop(&mut bars, "a", &DropTarget::Float { pos_ratio: [2.0, f32::NAN] }, &[]).unwrap();
    let a = &bars[0];
    assert_eq!((a.dock, a.x_ratio, a.y_ratio), (Dock::Floating, 1.0, 0.0));
    assert!(toolbars::drag::apply_drop(&mut bars, "zzz", &DropTarget::Float { pos_ratio: [0.0, 0.0] }, &[]).is_err());
    assert!(toolbars::drag::apply_drop(&mut bars, "a", &DropTarget::Edge { dock: Dock::Floating, row: 0, order: 0 }, &[]).is_err());
}

#[test]
fn simple_draws_todays_toolbar_with_a_grip() {
    let h = harness(attached(MemoryStore::default()));
    assert!(h.query_by_label("Standard toolbar grip").is_some());
    assert!(h.query_by_label("New Drawing (Ctrl+N)").is_some());
    assert!(h.query_by_label("Zoom Window").is_some());
    assert!(h.state().ws.user.layout.toolbars.iter().all(|bar| bar.id == "standard"), "the other shipped bars are only offered, not stored");
}

/// A company layer that holds `draw` at row 0, position `order`, and locks it.
fn company_holds_draw_at(order: u32) -> MemoryStore {
    let draw = json!({
        "id": "draw", "label": "Draw", "dock": "top", "row": 0, "order": order, "visible": true,
        "commands": toolbars::builtin_toolbar_commands("draw")
    });
    let policy = json!({"layers": [{"level": "company", "patch": {"toolbars": [draw]}, "locks": ["/toolbars/draw"]}]});
    MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() }
}

fn order_of(app: &CadApp, id: &str) -> u32 {
    bar(app, id).order
}

#[test]
fn dropping_next_to_a_locked_bar_leaves_its_order_alone() {
    // Gap before the locked bar: there is room right in front of it.
    let mut app = attached(company_holds_draw_at(3));
    run(&mut app, "ui.toolbar.move", json!({"id": "modify", "dock": "top", "row": 0, "order": 1}));
    assert_eq!((order_of(&app, "standard"), order_of(&app, "modify"), order_of(&app, "draw")), (0, 1, 3));

    // After the locked bar.
    let mut app = attached(company_holds_draw_at(1));
    run(&mut app, "ui.toolbar.move", json!({"id": "modify", "dock": "top", "row": 0, "order": 2}));
    assert_eq!((order_of(&app, "standard"), order_of(&app, "draw"), order_of(&app, "modify")), (0, 1, 2));

    // Directly before it there is no room without moving it: the nearest legal slot is taken.
    let mut app = attached(company_holds_draw_at(1));
    run(&mut app, "ui.toolbar.dock", json!({"id": "modify", "dock": "top", "row": 0, "order": 1}));
    assert_eq!(order_of(&app, "draw"), 1, "the locked bar did not move");
    assert!(order_of(&app, "modify") > 1, "{:?}", bar(&app, "modify"));
    assert_eq!(app.ws.user.layout.toolbars.iter().find(|bar| bar.id == "draw").map(|bar| bar.order), Some(1));
}

#[test]
fn apply_drop_never_renumbers_a_frozen_bar() {
    let bar = |id: &str, order: u32| ToolbarState { id: id.into(), order, ..ToolbarState::default() };
    let frozen = ["f".to_string()];
    let orders = |bars: &[ToolbarState]| bars.iter().map(|bar| (bar.id.clone(), bar.order)).collect::<Vec<_>>();
    let mut bars = vec![bar("a", 0), bar("f", 1), bar("g", 2), bar("m", 9)];
    toolbars::drag::apply_drop(&mut bars, "m", &DropTarget::Edge { dock: Dock::Top, row: 0, order: 0 }, &frozen).unwrap();
    assert_eq!(bars.iter().find(|bar| bar.id == "f").map(|bar| bar.order), Some(1));
    assert_eq!(
        orders(&bars),
        [("a".to_string(), 0), ("f".to_string(), 1), ("g".to_string(), 3), ("m".to_string(), 2)],
        "no room before f: the nearest slot is after it"
    );

    // Free bars after a frozen one are pushed when they collide.
    let mut bars = vec![bar("f", 0), bar("g", 1), bar("m", 5)];
    toolbars::drag::apply_drop(&mut bars, "m", &DropTarget::Edge { dock: Dock::Top, row: 0, order: 1 }, &["f".to_string()]).unwrap();
    assert_eq!(orders(&bars), [("f".to_string(), 0), ("g".to_string(), 2), ("m".to_string(), 1)]);
}

#[test]
fn dragging_a_bar_past_a_locked_one_is_not_refused() {
    let mut h = harness(attached(company_holds_draw_at(1)));
    let history = h.state().ws.history.undo_len();
    let start = grip(&h, "Standard");
    let row_y = start.y;
    drag(&mut h, start, Pos2::new(1300.0, row_y), true);
    let state = h.state();
    assert_eq!(order_of(state, "draw"), 1, "the locked bar kept its place");
    assert!(order_of(state, "standard") > 1, "standard now follows draw");
    assert_eq!(state.ws.history.undo_len(), history + 1, "{:?}", state.ws.status);
    assert!(h.get_by_label("Standard toolbar grip").rect().left() > h.get_by_label("Draw toolbar grip").rect().left());
}

#[test]
fn locked_bar_is_unlocked_from_the_keyboard() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toolbar.lock", json!({"id": "standard"}));
    let polar = app.session.settings.polarmode;
    let mut h = harness(app);
    assert!(h.query_by_label("Standard toolbar grip").is_none(), "a locked bar has no grip");
    for _ in 0..120 {
        if h.query_by_label("New Drawing (Ctrl+N)").is_some_and(|node| node.is_focused()) {
            break;
        }
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert!(h.get_by_label("New Drawing (Ctrl+N)").is_focused(), "Tab reaches a button of the locked bar");
    h.key_press_modifiers(Modifiers::SHIFT, Key::F10);
    h.run_steps(3);
    assert!(h.get_by_label("Hide").is_focused(), "Shift+F10 on a button opens the bar's menu");
    assert_eq!(h.state().session.settings.polarmode, polar);
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(h.get_by_label("Unlock").is_focused());
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert!(!bar(h.state(), "standard").locked);
    assert!(h.query_by_label("Standard toolbar grip").is_some(), "the grip is back");
}

#[test]
fn commands_materialize_a_shipped_bar_the_profile_does_not_hold() {
    let cases = [
        ("ui.toolbar.lock", json!({"id": "modify"})),
        ("ui.toolbar.unlock", json!({"id": "modify"})),
        ("ui.toolbar.dock", json!({"id": "modify", "dock": "right"})),
        ("ui.toolbar.float", json!({"id": "modify"})),
        ("ui.toolbar.move", json!({"id": "modify", "dock": "bottom"})),
    ];
    for (command, params) in cases {
        let mut app = attached(MemoryStore::default());
        assert!(app.ws.user.layout.toolbars.iter().all(|bar| bar.id != "modify"));
        run(&mut app, command, params);
        let modify = bar(&app, "modify");
        assert_eq!(modify.commands, toolbars::builtin_toolbar_commands("modify"), "{command}");
        assert_eq!(modify.locked, command == "ui.toolbar.lock", "{command}");
    }
    let mut app = attached(MemoryStore::default());
    assert!(app.run("ui.toolbar.lock", json!({"id": "nonsense"})).unwrap_err().contains("Unknown toolbar"));
}

#[test]
fn every_disabled_item_of_a_policy_locked_bar_explains_why() {
    let mut h = harness(attached(company_locks_draw()));
    h.get_by_label("Draw toolbar grip").click_secondary();
    h.run_steps(3);
    for label in ["Hide", "Lock", "Dock ⏵", "Float"] {
        let item = h.get_by_label(label);
        assert!(item.accesskit_node().is_disabled(), "{label} is greyed out");
        item.hover();
        h.run_steps(6);
        assert!(h.query_by_label("Locked by company policy").is_some(), "{label} has the policy tooltip");
        h.hover_at(Pos2::new(1200.0, 600.0));
        h.run_steps(3);
    }
}

#[test]
fn arrange_equal_order_next_to_frozen_bar_still_places() {
    let make_bar = |id: &str, order: u32| ToolbarState { id: id.into(), order, ..ToolbarState::default() };
    for free_count in [1, 2] {
        let mut bars = vec![make_bar("a", 0)];
        if free_count == 2 {
            bars.push(make_bar("b", 0));
        }
        bars.push(make_bar("frozen", 0));
        bars.push(ToolbarState { id: "moving".into(), dock: Dock::Left, order: 9, ..ToolbarState::default() });
        toolbars::drag::apply_drop(&mut bars, "moving", &DropTarget::Edge { dock: Dock::Top, row: 0, order: 0 }, &["frozen".to_string()]).unwrap();
        assert_eq!(bars.iter().find(|bar| bar.id == "frozen").map(|bar| bar.order), Some(0));
        assert_eq!(bars.iter().find(|bar| bar.id == "moving").map(|bar| (bar.dock, bar.order)), Some((Dock::Top, 1)));
    }
}

#[test]
fn locking_hidden_shipped_bar_keeps_it_hidden() {
    for command in ["ui.toolbar.lock", "ui.toolbar.unlock"] {
        let mut app = attached(MemoryStore::default());
        assert!(app.ws.user.layout.toolbars.iter().all(|bar| bar.id != "modify"));
        run(&mut app, command, json!({"id": "modify"}));
        assert!(!bar(&app, "modify").visible, "{command}");
        assert_eq!(bar(&app, "modify").locked, command == "ui.toolbar.lock");
        let mut h = harness(app);
        assert!(h.query_by_label("Modify toolbar grip").is_none());
        h.state_mut().commit_workspace_now().unwrap();
        let restored = restarted(h.state());
        assert!(!bar(&restored, "modify").visible, "{command} survives restart without showing the bar");
    }
}

#[test]
fn drag_preview_matches_committed_slot_next_to_frozen_bar() {
    let mut app = attached(company_holds_draw_at(1));
    run(&mut app, "ui.toolbar.float", json!({"id": "modify", "x_ratio": 0.2, "y_ratio": 0.4}));
    let mut h = harness(app);
    let start = grip(&h, "Modify");
    let draw_rect = h.get_by_label("Draw toolbar grip").rect();
    let pointer = Pos2::new(draw_rect.left(), draw_rect.center().y);
    drag(&mut h, start, pointer, false);
    let target = match &h.state().ws.toolbar_drag {
        DragPhase::Dragging { target: Some(target), .. } => target.clone(),
        phase => panic!("expected a preview target, got {phase:?}"),
    };
    assert_eq!(target, DropTarget::Edge { dock: Dock::Top, row: 0, order: 2 }, "preview snaps after the frozen bar");
    h.drop_at(pointer);
    h.run_steps(4);
    assert_eq!((bar(h.state(), "modify").dock, order_of(h.state(), "modify")), (Dock::Top, 2));
    assert_eq!(order_of(h.state(), "draw"), 1);
    assert!(h.get_by_label("Modify toolbar grip").rect().left() > h.get_by_label("Draw toolbar grip").rect().left());
}

fn focus_toolbar_control(h: &mut Harness<'_, CadApp>, label: &str) {
    for _ in 0..120 {
        if h.query_by_label(label).is_some_and(|node| node.is_focused()) {
            return;
        }
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    panic!("Tab did not reach {label}");
}

#[test]
fn shift_f10_in_overflow_popup_opens_menu() {
    let mut app = attached(MemoryStore::default());
    run(
        &mut app,
        "ui.workspace.set",
        json!({
            "pointer": "/toolbars/standard/commands",
            "value": [
                "line", "circle", "arc", "rectang", "erase", "move", "copy", "rotate", "scale", "hatch",
                "dist", "list", "area", "pan", "zoom.window", "zoom.extents", "plot", "publish", "open", "qsave"
            ]
        }),
    );
    let polar = app.session.settings.polarmode;
    let mut h = harness_sized(app, 600.0, 600.0);
    h.get_by_label("More toolbar commands").click();
    h.run_steps(3);
    focus_toolbar_control(&mut h, "List");
    h.key_press_modifiers(Modifiers::SHIFT, Key::F10);
    h.run_steps(3);
    assert!(h.get_by_label("Hide").is_focused(), "the overflow button opens its toolbar's context menu");
    assert_eq!(h.state().session.settings.polarmode, polar);
}

#[test]
fn escape_on_toolbar_focus_cancels_command() {
    let mut h = harness(attached(MemoryStore::default()));
    h.state_mut().start("line");
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("line"));
    focus_toolbar_control(&mut h, "New Drawing (Ctrl+N)");
    h.state_mut().cmd.buffer = "unfinished".into();
    // egui clears focus on Escape before our handlers run.
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.running.is_none());
    assert!(h.state().cmd.buffer.is_empty(), "Escape reaches the command line's existing handler");
}
