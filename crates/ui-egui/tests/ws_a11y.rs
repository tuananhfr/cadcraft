//! Keyboard, IME and AccessKit coverage for workspace customization controls.

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, chrome, customize, ribbon, theme, toolbars};
use egui::accesskit::Role;
use egui::{Event, ImeEvent, Key, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

fn app() -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app
}

fn restart(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    let store = MemoryStore { current: old.read_current(), policy: old.read_policy(), snapshot_map, ..MemoryStore::default() };
    let mut restored = CadApp::new(Session::default(), Services::default());
    restored.skip_first_run_picker = true;
    restored.attach_profile_store(Box::new(store));
    restored
}

#[test]
fn new_controls_have_accesskit_labels() {
    let mut app = app();
    app.run("ui.toolbar.float", json!({"id": "draw", "x_ratio": 0.05, "y_ratio": 0.65})).unwrap();
    app.run("ui.quickaccess.pin", json!({"id": "sheetset"})).unwrap();
    app.run("ui.quickaccess.move", json!({"id": "sheetset", "to": 0})).unwrap();
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    for label in [cadcraft_engine::find_command("line").unwrap().label, "Draw toolbar grip", "Add Tab", "Rename Tab", "Reset"] {
        assert!(h.query_all_by_role_and_label(Role::Button, label).next().is_some(), "{label}");
    }
    let placeholder = h.get_by_role_and_label(Role::Button, "Sheet Set — not available in this version");
    assert!(placeholder.accesskit_node().is_disabled());
    for button in h.query_all_by_role(Role::Button) {
        assert!(button.accesskit_node().label().is_some_and(|label| !label.trim().is_empty()), "{button:?}");
    }
    assert!(h.query_by_label("Save (Ctrl+S)").is_some() || h.query_by_label("Save (⌘S)").is_some());
}

#[test]
fn rename_tab_accepts_vietnamese_ime() {
    let mut app = app();
    let tab = "home";
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            let mut open = true;
            customize::dialog(app, ui.ctx(), &mut open);
        },
        app,
    );
    h.run_steps(5);
    h.get_by_role_and_label(Role::TextInput, "Tab or group name").click();
    h.run_steps(2);
    h.event(Event::Ime(ImeEvent::Commit("Bản vẽ kiến trúc".into())));
    h.run_steps(3);
    h.get_by_label("Rename Tab").click();
    h.run_steps(3);
    assert_eq!(h.state().ws.effective.layout.ribbon.tabs.iter().find(|item| item.id == tab).unwrap().label, "Bản vẽ kiến trúc");
    let restored = restart(h.state_mut());
    assert_eq!(restored.ws.effective.layout.ribbon.tabs.iter().find(|item| item.id == tab).unwrap().label, "Bản vẽ kiến trúc");
}

#[test]
fn focus_is_visible_on_ribbon() {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| ribbon::render::show(app, ui), app());
    h.run_steps(3);
    let label = cadcraft_engine::find_command("line").unwrap().label;
    for _ in 0..60 {
        if h.query_by_label(label).is_some_and(|button| button.is_focused()) {
            break;
        }
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    let button = h.get_by_role_and_label(Role::Button, label);
    assert!(button.is_focused());
    let focused = h.ctx.memory(|memory| memory.focused()).unwrap();
    assert_eq!(focused.accesskit_id(), button.accesskit_node().locate().0);
    let accent = theme::Tokens::get().accent;
    let focus = button.rect().shrink(2.0);
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect)
        if rect.stroke.color == accent && rect.rect.center().distance(focus.center()) < 1.0)));
}

#[test]
fn integrated_title_row_draws_quick_access_when_menu_hidden() {
    let mut app = app();
    app.integrated_titlebar = true;
    app.run("ui.toggle.menubar", json!({"on": false})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| chrome::title_and_toolbar(app, ui), app);
    h.run_steps(4);
    assert!(h.query_by_label("Save (Ctrl+S)").is_some() || h.query_by_label("Save (⌘S)").is_some());
    h.state_mut().run("ui.workspace.select", json!({"id": "SIMPLE"})).unwrap();
    h.state_mut().integrated_titlebar = true;
    h.state_mut().run("ui.toggle.menubar", json!({"on": false})).unwrap();
    h.state_mut().run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Search commands...").is_none(), "an empty Quick Access list stays empty");
}

#[test]
fn ribbon_unknown_command_keeps_accessible_placeholder() {
    let mut app = app();
    let mut groups = app.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().groups.clone();
    groups.first_mut().unwrap().commands = vec!["sheetset".into()];
    app.run("ui.workspace.set", json!({"pointer": "/ribbon/tabs/home/groups", "value": groups})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            ribbon::render::show(app, ui);
            toolbars::show_floating(ui.ctx(), app);
        },
        app,
    );
    h.run_steps(3);
    assert!(h.get_by_role_and_label(Role::Button, "Sheet Set — not available in this version").accesskit_node().is_disabled());
    let restored = restart(h.state_mut());
    assert_eq!(restored.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().groups.first().unwrap().commands, ["sheetset"]);
}

#[test]
fn customize_escape_preserves_focused_text_and_ime() {
    let mut app = app();
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::dialogs::show(app, ui.ctx()), app);
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Tab or group name").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Tab or group name").type_text("Draft");
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(customize::DIALOG));
    assert!(!h.get_by_role_and_label(Role::TextInput, "Tab or group name").is_focused());
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Tab or group name").value().as_deref(), Some("Draft"));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none(), "the second Escape closes Customize");
    h.state_mut().run("ui.customize.open", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Tab or group name").click();
    h.run_steps(2);
    h.event(Event::Ime(ImeEvent::Preedit { text: "Bản vẽ".into(), active_range_chars: None }));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(customize::DIALOG));
    assert!(h.get_by_role_and_label(Role::TextInput, "Tab or group name").is_focused());
    h.event(Event::Ime(ImeEvent::Commit("Bản vẽ kiến trúc".into())));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(customize::DIALOG));
    assert!(!h.get_by_role_and_label(Role::TextInput, "Tab or group name").is_focused());
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Tab or group name").value().as_deref(), Some("Bản vẽ kiến trúc"));
    h.get_by_label("Rename Tab").click();
    h.run_steps(2);
    assert_eq!(h.state().ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().label, "Bản vẽ kiến trúc");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none(), "Escape closes Customize after focus leaves the text field");
}
