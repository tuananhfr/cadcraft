use cadcraft_engine::Session;
use cadcraft_ui_egui::customize;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn app_with_line() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    assert_eq!(app.session.doc().unwrap().model.len(), 1);
    app
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
            }
        },
        app,
    );
    h.run_steps(3);
    h
}

fn standard_toolbar_visible(app: &CadApp) -> bool {
    app.ws.effective.layout.toolbars.iter().any(|toolbar| toolbar.id == "standard" && toolbar.visible)
}

#[test]
fn ctrl_z_in_customize_undoes_layout_not_drawing() {
    let _guard = test_lock();
    let mut app = app_with_line();
    app.run("ui.customize.open", json!({})).unwrap();
    assert!(standard_toolbar_visible(&app));
    app.run("ui.toolbar.hide", json!({"id": "standard"})).unwrap();
    let mut h = harness(app);
    assert!(!standard_toolbar_visible(h.state()));

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert!(standard_toolbar_visible(h.state()));
    assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
    assert_eq!(h.state().ws.history.redo_len(), 1);

    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run_steps(3);
    assert!(!standard_toolbar_visible(h.state()));
    assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
    assert_eq!(h.state().ws.history.redo_len(), 0);

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert!(standard_toolbar_visible(h.state()));
    assert_eq!(h.state().ws.history.redo_len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(3);
    assert!(!standard_toolbar_visible(h.state()));
    assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
    assert_eq!(h.state().ws.history.redo_len(), 0);
}

#[test]
fn ctrl_z_in_customize_text_field_touches_neither() {
    let _guard = test_lock();
    let mut app = app_with_line();
    app.run("ui.customize.open", json!({})).unwrap();
    app.run("ui.toolbar.hide", json!({"id": "standard"})).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Keyboard").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").focus();
    h.run_steps(2);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").is_focused());
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").type_text("ab");
    h.run_steps(2);
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Find command").value().as_deref(), Some("ab"));
    let undo_len = h.state().ws.history.undo_len();
    let redo_len = h.state().ws.history.redo_len();
    assert!(undo_len > 0);
    let drawing = h.state().session.doc().unwrap().clone();
    let layout = h.state().ws.effective.layout.clone();

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
    assert_eq!(*h.state().session.doc().unwrap(), drawing);
    assert_eq!(h.state().ws.effective.layout, layout);
    assert_eq!(h.state().ws.history.undo_len(), undo_len);
    assert_eq!(h.state().ws.history.redo_len(), redo_len);
}

#[test]
fn ctrl_z_outside_customize_still_undoes_drawing() {
    let _guard = test_lock();
    let mut h = harness(app_with_line());
    assert!(h.state().ui.dialog.is_none());
    let layout = h.state().ws.effective.layout.clone();
    let undo_len = h.state().ws.history.undo_len();

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().session.doc().unwrap().model.len(), 0);
    assert_eq!(h.state().ws.effective.layout, layout);
    assert_eq!(h.state().ws.history.undo_len(), undo_len);
}
