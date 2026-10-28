//! About names the MPL-2.0 library UbotCAD ships (acadrust, DWG reading and writing) and where its
//! source is: MPL-2.0 §3.2(a) asks every executable form to tell its recipients that.

use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, Services, dialogs};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn about_names_acadrust_its_licence_and_its_source() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.dialog = Some("about".into());
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| dialogs::show(app, ui.ctx()), app);
    h.run_steps(2);
    h.get_by_label_contains("Includes acadrust (DWG reading and writing)");
    h.get_by_label_contains("Mozilla Public License 2.0");
    h.get_by_label("https://github.com/hakanaktt/acadrust");
    h.get_by_label("https://mozilla.org/MPL/2.0/");
}
