use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, customize, ribbon, toolbars};
use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app
}

fn familiar() -> CadApp {
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.run("ui.customize.open", json!({})).unwrap();
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
            app.logic(ui.ctx());
            if app.ui.dialog.as_deref() == Some(customize::DIALOG) {
                let mut open = true;
                customize::dialog(app, ui.ctx(), &mut open);
                if !open {
                    app.ui.dialog = None;
                }
            } else {
                ribbon::render::show(app, ui);
            }
        },
        app,
    );
    h.run_steps(5);
    h
}

fn commands(app: &CadApp, group: &str) -> Vec<String> {
    app.ws
        .effective
        .layout
        .ribbon
        .tabs
        .iter()
        .find(|tab| tab.id == "home")
        .unwrap()
        .groups
        .iter()
        .find(|item| item.id == group)
        .unwrap()
        .commands
        .clone()
}

fn focus_command(h: &mut Harness<'_, CadApp>, command: &str) {
    let label = toolbars::resolve(command).label;
    h.query_all_by_label(&label).next().unwrap().focus();
    h.run_steps(5);
}

fn command_rect(h: &Harness<'_, CadApp>, command: &str) -> egui::Rect {
    let label = toolbars::resolve(command).label;
    h.query_all_by_label(&label).next().unwrap().rect()
}

fn begin_drag(h: &mut Harness<'_, CadApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    h.hover_at(from + (to - from) * 0.5);
    h.run_steps(2);
    h.hover_at(to);
    h.run_steps(3);
    assert!(egui::DragAndDrop::has_any_payload(&h.ctx));
}

fn drag_command(h: &mut Harness<'_, CadApp>, from: usize, to: usize) {
    let before = commands(h.state(), "draw");
    focus_command(h, &before[to]);
    let source = command_rect(h, &before[from]).center();
    let target = command_rect(h, &before[to]);
    let target = egui::pos2(target.center().x, if from < to { target.bottom() - 2.0 } else { target.top() + 2.0 });
    let steps = h.state().ws.history.undo_len();
    begin_drag(h, source, target);
    assert_eq!(commands(h.state(), "draw"), before);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    h.drop_at(target);
    h.run_steps(5);
    let mut expected = before;
    let moved = expected.remove(from);
    expected.insert(to, moved);
    assert_eq!(commands(h.state(), "draw"), expected);
    assert_eq!(h.state().ws.history.undo_len(), steps + 1);
}

fn assert_rendered_order(h: &Harness<'_, CadApp>, ids: &[String]) {
    let mut rendered: Vec<_> = ids.iter().map(|id| (id.clone(), command_rect(h, id).center())).collect();
    rendered.sort_by(|(_, a), (_, b)| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    assert_eq!(rendered.iter().map(|(id, _)| id).collect::<Vec<_>>(), ids.iter().collect::<Vec<_>>());
}

#[test]
fn eight_commands_reorder_by_drag_h01() {
    let _guard = test_lock();
    let mut h = harness(familiar());
    let before = commands(h.state(), "draw");
    assert!(before.len() >= 8);
    for from in 0..8 {
        drag_command(&mut h, from, from + 1);
    }
    let expected = commands(h.state(), "draw");
    assert_ne!(expected, before);
    h.state_mut().ui.dialog = None;
    h.run_steps(5);
    assert_rendered_order(&h, &expected);
}

#[test]
fn keyboard_reorder_matches_drag() {
    let _guard = test_lock();
    let mut keyboard = harness(familiar());
    let first = commands(keyboard.state(), "draw")[0].clone();
    focus_command(&mut keyboard, &first);
    let steps = keyboard.state().ws.history.undo_len();
    for _ in 0..3 {
        keyboard.key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
        keyboard.run_steps(5);
        let label = toolbars::resolve(&first).label;
        assert!(keyboard.query_all_by_label(&label).next().unwrap().is_focused());
    }
    let expected = commands(keyboard.state(), "draw");
    assert_eq!(expected[3], first);
    assert_eq!(keyboard.state().ws.history.undo_len(), steps + 3);
    let mut dragged = harness(familiar());
    drag_command(&mut dragged, 0, 3);
    assert_eq!(commands(dragged.state(), "draw"), expected);
    keyboard.key_press_modifiers(Modifiers::ALT, Key::ArrowUp);
    keyboard.run_steps(5);
    assert_eq!(commands(keyboard.state(), "draw")[2], first);
}

#[test]
fn escape_cancels_a_drag() {
    let _guard = test_lock();
    let mut h = harness(familiar());
    let before = commands(h.state(), "draw");
    focus_command(&mut h, &before[2]);
    let source = command_rect(&h, &before[0]).center();
    let target = command_rect(&h, &before[2]).center();
    let steps = h.state().ws.history.undo_len();
    begin_drag(&mut h, source, target);
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(!egui::DragAndDrop::has_any_payload(&h.ctx));
    assert_eq!(h.state().ui.dialog.as_deref(), Some(customize::DIALOG));
    h.drop_at(target);
    h.run_steps(5);
    assert_eq!(commands(h.state(), "draw"), before);
    assert_eq!(h.state().ws.history.undo_len(), steps);
}

#[test]
fn ten_reordered_buttons_keep_icons_after_restart_tc01() {
    let _guard = test_lock();
    let mut app = familiar();
    app.run("ui.ribbon.button.remove", json!({"tab": "home", "group": "draw", "id": "point"})).unwrap();
    app.run("ui.ribbon.button.add", json!({"tab": "home", "group": "draw", "id": "move"})).unwrap();
    let before = commands(&app, "draw");
    assert_eq!(before.len(), 10);
    let icons: Vec<_> = before.iter().map(|id| (id.clone(), toolbars::resolve(id).icon)).collect();
    let mut expected = before.clone();
    for to in 0..9 {
        let from = 9;
        app.run("ui.ribbon.button.move", json!({"tab": "home", "group": "draw", "from": from, "to": to})).unwrap();
        let moved = expected.remove(from);
        expected.insert(to, moved);
    }
    assert_ne!(expected, before);
    let restored = restarted(&mut app);
    assert_eq!(commands(&restored, "draw"), expected);
    for (id, icon) in icons {
        let resolved = toolbars::resolve(&id);
        assert!(resolved.known, "{id}");
        assert!(resolved.icon.is_some(), "{id}");
        assert_eq!(resolved.icon, icon, "{id}");
    }
    assert!(!restored.session.state().unwrap().is_dirty());
    let h = harness(restored);
    assert_rendered_order(&h, &expected);
}

#[test]
fn customize_ctrl_z_undoes_the_layout_not_the_drawing_tc19() {
    let _guard = test_lock();
    let mut app = familiar();
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let drawing = app.session.doc().unwrap().clone();
    let mut h = harness(app);
    let before = commands(h.state(), "draw");
    drag_command(&mut h, 0, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(5);
    assert_eq!(commands(h.state(), "draw"), before);
    assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
    assert_eq!(*h.state().session.doc().unwrap(), drawing);
    assert_eq!(h.state().ws.history.redo_len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(5);
    assert_ne!(commands(h.state(), "draw"), before);
    assert_eq!(*h.state().session.doc().unwrap(), drawing);
}

#[test]
fn moving_between_groups_is_one_undo_step() {
    let _guard = test_lock();
    let mut app = familiar();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    let mut expected = commands(&app, "modify");
    expected.insert(2, "line".into());
    app.run("ui.ribbon.button.move", json!({"tab": "home", "group": "draw", "command": "line", "to_group": "modify", "to": 2})).unwrap();
    assert_eq!(commands(&app, "modify"), expected);
    assert!(!commands(&app, "draw").iter().any(|id| id == "line"));
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
}

#[test]
fn ribbon_button_move_rejects_invalid_params_and_respects_lock() {
    let _guard = test_lock();
    let mut app = familiar();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [
        json!({}),
        json!({"tab": "missing", "group": "draw", "from": 0, "to": 1}),
        json!({"tab": "home", "group": "missing", "from": 0, "to": 1}),
        json!({"tab": "home", "group": "draw", "from": -1, "to": 1}),
        json!({"tab": "home", "group": "draw", "from": u64::MAX, "to": 1}),
        json!({"tab": "home", "group": "draw", "from": 0, "to": "1"}),
        json!({"tab": "home", "group": "draw", "command": "missing", "to_group": "modify", "to": 0}),
        json!({"tab": "home", "group": "draw", "command": "line", "to_group": "missing", "to": 0}),
    ] {
        assert!(app.run("ui.ribbon.button.move", params).is_err());
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/ribbon/tabs/home/groups/draw/commands"]}]});
    let mut locked = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    locked.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let before = locked.ws.user.layout.clone();
    let steps = locked.ws.history.undo_len();
    for params in [
        json!({"tab": "home", "group": "draw", "from": 0, "to": 1}),
        json!({"tab": "home", "group": "draw", "command": "line", "to_group": "modify", "to": 0}),
    ] {
        assert_eq!(locked.run("ui.ribbon.button.move", params).unwrap_err(), LOCKED_MESSAGE);
        assert_eq!(locked.ws.user.layout, before);
        assert_eq!(locked.ws.history.undo_len(), steps);
    }
}

#[test]
fn ribbon_reorder_buttons_and_keyboard_boundaries() {
    let _guard = test_lock();
    let mut h = harness(familiar());
    let before = commands(h.state(), "draw");
    let steps = h.state().ws.history.undo_len();
    focus_command(&mut h, &before[0]);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowUp);
    h.run_steps(5);
    assert_eq!(commands(h.state(), "draw"), before);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    assert!(h.query_all_by_label(&toolbars::resolve(&before[0]).label).next().unwrap().is_focused());
    h.query_all_by_label("Down").next().unwrap().scroll_to_me();
    h.run_steps(5);
    h.query_all_by_label("Down").next().unwrap().click();
    h.run_steps(5);
    let mut swapped = before.clone();
    swapped.swap(0, 1);
    assert_eq!(commands(h.state(), "draw"), swapped);
    h.query_all_by_label("Up").next().unwrap().scroll_to_me();
    h.run_steps(5);
    h.query_all_by_label("Up").next().unwrap().click();
    h.run_steps(5);
    assert_eq!(commands(h.state(), "draw"), before);
    focus_command(&mut h, before.last().unwrap());
    let steps = h.state().ws.history.undo_len();
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    h.run_steps(5);
    assert_eq!(commands(h.state(), "draw"), before);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    assert!(h.query_all_by_label(&toolbars::resolve(before.last().unwrap()).label).next().unwrap().is_focused());
}

#[test]
fn ribbon_button_move_clamps_destination_and_ignores_noop() {
    let _guard = test_lock();
    let mut app = familiar();
    let before = commands(&app, "draw");
    let steps = app.ws.history.undo_len();
    let result = app.run("ui.ribbon.button.move", json!({"tab": "home", "group": "draw", "from": 0, "to": 0})).unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(app.ws.history.undo_len(), steps);
    app.run("ui.ribbon.button.move", json!({"tab": "home", "group": "draw", "from": 0, "to": 500})).unwrap();
    assert_eq!(commands(&app, "draw").last(), before.first());
    app.run("ui.ribbon.button.add", json!({"tab": "home", "group": "modify", "id": "line"})).unwrap();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    assert!(app.run("ui.ribbon.button.move", json!({"tab": "home", "group": "draw", "command": "line", "to_group": "modify", "to": 0})).is_err());
    assert_eq!(app.ws.user.layout, before);
    assert_eq!(app.ws.history.undo_len(), steps);
}
