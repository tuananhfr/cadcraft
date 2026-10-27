//! Session applications, content approvals, write permissions and recent edits.

use crate::CadApp;

// Reserve room below the list so loading another application stays reachable.
const LOAD_BUTTON_SPACE: f32 = 36.0;

#[derive(Clone, Default)]
struct State {
    folders_open: bool,
    #[cfg(not(target_arch = "wasm32"))]
    path: String,
    #[cfg(not(target_arch = "wasm32"))]
    error: Option<String>,
}

pub fn body(ui: &mut egui::Ui, app: &mut CadApp) {
    let id = ui.make_persistent_id("lisp-panel-state");
    let mut state = ui.ctx().data_mut(|data| data.remove_temp::<State>(id)).unwrap_or_default();
    ui.label(crate::tl!("Startup files load from <settings>/support only"));
    ui.separator();
    let files = cadcraft_engine::lisp::loaded_files(&app.session);
    let audit = app.session.lisp_audit();
    egui::ScrollArea::vertical().id_salt("lisp-loaded-files").max_height((ui.available_height() - LOAD_BUTTON_SPACE).max(0.0)).show(ui, |ui| {
        if files.is_empty() {
            ui.label(crate::tl!("No applications loaded in this session"));
        }
        for name in files {
            ui.group(|ui| {
                ui.label(&name);
                if let Some(sha) = cadcraft_engine::lisp::loaded_file_sha(&app.session, &name) {
                    let reviewed = cadcraft_engine::lisp::trust::is_approved(sha);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if reviewed { crate::tl!("Reviewed") } else { crate::tl!("Unreviewed") });
                        ui.monospace(sha.get(..8).unwrap_or(sha));
                    });
                }
            });
        }
        ui.separator();
        if ui.button(crate::tl!("Allowed folders…")).clicked() {
            state.folders_open = !state.folders_open;
        }
        if state.folders_open {
            folders(ui, app, &mut state);
        }
        ui.separator();
        ui.strong(crate::tl!("Recent LISP edits"));
        if audit.is_empty() {
            ui.label(crate::tl!("No LISP edits in this session"));
        }
        for record in audit.iter().rev().take(20) {
            ui.group(|ui| {
                ui.label(crate::tf!("Time: {date} UTC", date = crate::autosave::utc(record.at_ms / 1000)));
                ui.label(&record.source);
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::tf!("Created: {n}", n = record.created));
                    ui.label(crate::tf!("Modified: {n}", n = record.modified));
                    ui.label(crate::tf!("Deleted: {n}", n = record.deleted));
                });
                ui.label(crate::tf!("Undo: {label}", label = record.undo_label));
            });
        }
    });
    if ui.button(crate::tl!("Load Application...")).clicked() {
        load_application(app);
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, state));
}

#[cfg(not(target_arch = "wasm32"))]
fn load_application(app: &mut CadApp) {
    app.start("appload");
}

#[cfg(target_arch = "wasm32")]
fn load_application(app: &mut CadApp) {
    // APPLOAD asks for a path the browser can't read; the picked file arrives through the host.
    match &app.services.pick_open_kind {
        Some(pick) => {
            let _ = pick(&crate::LISP_FILE);
        }
        None => app.start("appload"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn folders(ui: &mut egui::Ui, app: &mut CadApp, state: &mut State) {
    use cadcraft_engine::lisp::sandbox::write_roots;
    use serde_json::json;

    let roots = write_roots();
    if roots.is_empty() {
        ui.label(crate::tl!("No allowed folders"));
    }
    for path in roots.iter().take(64) {
        ui.push_id(path, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(path.to_string_lossy());
                if ui.button(crate::tl!("Remove")).clicked() {
                    state.error = app.run("lispsandbox", json!({"action": "remove", "path": path.to_string_lossy()})).err();
                }
            });
        });
    }
    let label = ui.label(crate::tl!("Folder path"));
    ui.add(egui::TextEdit::singleline(&mut state.path).desired_width(ui.available_width())).labelled_by(label.id);
    if ui.add_enabled(!state.path.trim().is_empty(), egui::Button::new(crate::tl!("Add"))).clicked() {
        state.error = app.run("lispsandbox", json!({"action": "add", "path": state.path.trim()})).err();
        if state.error.is_none() {
            state.path.clear();
        }
    }
    if let Some(error) = &state.error {
        ui.colored_label(crate::theme::Tokens::get().warn, error);
    }
}

#[cfg(target_arch = "wasm32")]
fn folders(ui: &mut egui::Ui, _app: &mut CadApp, _state: &mut State) {
    // LISPSANDBOX is desktop-only; browser OPEN refuses write modes.
    ui.label(crate::tl!("No allowed folders"));
    ui.label(crate::tl!("LISP cannot write files in the browser"));
}
