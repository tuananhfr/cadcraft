//! Session application names come from the engine, never from workspace profile fields.

use crate::CadApp;

// Reserve room below the list so loading another application stays reachable.
const LOAD_BUTTON_SPACE: f32 = 36.0;

pub fn body(ui: &mut egui::Ui, app: &mut CadApp) {
    ui.label(crate::i18n::t("Startup files load from <settings>/support only"));
    ui.separator();
    let files = cadcraft_engine::lisp::loaded_files(&app.session);
    if files.is_empty() {
        ui.label(crate::i18n::t("No applications loaded in this session"));
    }
    egui::ScrollArea::vertical().id_salt("lisp-loaded-files").max_height((ui.available_height() - LOAD_BUTTON_SPACE).max(0.0)).show(ui, |ui| {
        for name in files {
            ui.label(name);
        }
    });
    if ui.button(crate::i18n::t("Load Application...")).clicked() {
        app.start("appload");
    }
}
