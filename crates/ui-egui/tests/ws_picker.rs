//! The first-run picker stays reachable on small screens.

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, Rect, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn picker_harness(size: Vec2) -> Harness<'static, CadApp> {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.ui.in_window_menu = false;
    app.integrated_titlebar = true;
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("workspace_picker"));
    h
}

fn assert_picker_reachable(size: Vec2) {
    let mut h = picker_harness(size);
    let screen = Rect::from_min_size(egui::Pos2::ZERO, size);
    let close_rect = {
        h.query_all_by_role(egui::accesskit::Role::Button)
            .find(|node| node.accesskit_node().label().as_deref() == Some("Close window"))
            .unwrap()
            .rect()
    };
    assert!(screen.contains_rect(close_rect), "the dialog close button must stay on screen");
    let last = "Sheets and Publishing";
    h.query_all_by_label(last).find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().focus();
    h.run_steps(3);
    let rows: Vec<_> = h.query_all_by_label(last).filter(|node| node.accesskit_node().role() == egui::accesskit::Role::Button).collect();
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert!(row.is_focused(), "the final preset accepts focus");
    assert!(screen.contains_rect(row.rect()), "the full last preset row must be on screen: {:?}", row.rect());
    let window = h
        .query_all_by_role(egui::accesskit::Role::Window)
        .find(|node| node.accesskit_node().label().as_deref() == Some("Choose a Workspace"))
        .unwrap();
    assert!(screen.contains_rect(window.rect()), "the picker window stays on screen");
    assert!(window.rect().contains_rect(row.rect()), "the last row must be scrolled into the picker window");
    let close =
        h.query_all_by_role(egui::accesskit::Role::Button).find(|node| node.accesskit_node().label().as_deref() == Some("Close window")).unwrap();
    assert!(screen.contains_rect(close.rect()), "the close button remains reachable after scrolling");
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().ws.active, "SHEET_PUBLISH", "activating the row confirms the workspace choice");
    assert_eq!(h.state().ui.dialog, None);
}

#[test]
fn picker_last_preset_reachable_at_1366x768() {
    assert_picker_reachable(vec2(1366.0, 768.0));
}

#[test]
fn picker_last_preset_reachable_at_1024x640() {
    assert_picker_reachable(vec2(1024.0, 640.0));
}
