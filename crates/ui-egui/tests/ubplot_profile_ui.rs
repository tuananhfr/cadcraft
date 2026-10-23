use cadcraft_engine::Session;
use cadcraft_engine::sheetplan::profile::{PRINT_PROFILE_SCHEMA, ProfileDoc, ProfileLevel};
use cadcraft_ui_egui::{CadApp, Services};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

#[test]
fn profile_dialog_shows_origin_and_lock() {
    let mut session = Session::new();
    session
        .write_print_profile(&ProfileDoc {
            schema_version: PRINT_PROFILE_SCHEMA,
            level: ProfileLevel::Company,
            id: "default".into(),
            project_id: None,
            revision: 1,
            settings: json!({"device": "CADCraft PDF"}),
            locks: vec!["/device".into()],
            updated_ms: 0,
        })
        .unwrap();
    let mut app = CadApp::new(session, Services::default());
    app.skip_first_run_picker = true;
    app.ui.start_tab = false;
    app.run("ui.ubplot.profile", Value::Null).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    assert!(harness.query_by_label("From level").is_some());
    assert!(harness.query_by_label("Company").is_some());
    assert!(harness.query_by_label("User").is_some());
    assert!(harness.query_by_label("Drawing").is_some());
    assert!(harness.query_all_by_label("Locked by company policy").count() > 0);
    let editor = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Device");
    assert!(editor.accesskit_node().is_disabled());
    let naming = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Naming").rect();
    assert!(naming.width() >= 200.0, "Naming editor must remain readable: {naming:?}");
    let before = harness.state_mut().session.effective_print_profile();
    let error = harness.state_mut().run("ubplot.profile", json!({"action": "set", "key": "device", "value": "X"})).unwrap_err();
    assert!(error.contains("LOCKED_BY_POLICY: /device (company)"));
    assert_eq!(harness.state_mut().session.effective_print_profile(), before);
}

#[test]
fn profile_dialog_value_column_fills_available_width_and_fits_at_200_percent() {
    for size in [egui::vec2(1366.0, 768.0), egui::vec2(683.0, 384.0)] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.skip_first_run_picker = true;
        app.ui.start_tab = false;
        app.run("ui.ubplot.profile", Value::Null).unwrap();
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, app: &mut CadApp| {
                app.logic(&ui.ctx().clone());
                app.ui(ui);
            },
            app,
        );
        harness.run_steps(5);
        let dialog = harness.get_by_role_and_label(egui::accesskit::Role::Window, "Print Profile").rect();
        assert!(harness.ctx.content_rect().contains_rect(dialog), "Profile dialog must fit at {size:?}: {dialog:?}");
        let naming = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Naming").rect();
        assert!(naming.width() >= 200.0, "Naming editor must remain readable at {size:?}: {naming:?}");
        assert!(naming.width() >= dialog.width() * 0.5, "Value column must use the available dialog width at {size:?}: {naming:?}");
    }
}
