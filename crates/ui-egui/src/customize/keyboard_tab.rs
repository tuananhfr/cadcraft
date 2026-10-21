//! Shortcut edits retain the shared binder's reserved-key and conflict checks.

use serde_json::json;

use super::{State, commands, edit, run};
use crate::CadApp;

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State) {
    commands(app, ui, state);
    if let Some(key) = crate::shortcuts::active(app).get(&state.command) {
        ui.label(crate::tf!("Current shortcut: {key}", key = key));
    }
    edit(ui, &mut state.key, crate::tl!("Key combination"));
    ui.checkbox(&mut state.replace, crate::tl!("Replace conflicting shortcut"));
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Assign Shortcut")).clicked() {
            let _ = run(app, "ui.shortcut.bind", json!({"command": state.command, "key": state.key, "replace": state.replace}), state);
        }
        if ui.button(crate::tl!("Remove Shortcut")).clicked() {
            let _ = run(app, "ui.shortcut.unbind", json!({"command": state.command}), state);
        }
    });
}
