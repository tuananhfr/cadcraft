//! Quick Access and favorites share W8a's persisted command APIs.

use serde_json::json;

use super::{State, commands, row, run};
use crate::CadApp;

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State) {
    let items = app.ws.effective.layout.quick_access.clone();
    ui.label(crate::tl!("Pinned commands"));
    egui::ScrollArea::vertical().id_salt("pins").max_height(130.0).show(ui, |ui| {
        for id in &items {
            if row(ui, state.command == *id, &crate::toolbars::resolve(id).label) {
                state.command = id.clone();
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Remove from Quick Access")).clicked() {
            let _ = run(app, "ui.quickaccess.unpin", json!({"id": state.command}), state);
        }
        if let Some(at) = items.iter().position(|id| *id == state.command) {
            if ui.add_enabled(at > 0, egui::Button::new(crate::tl!("Up"))).clicked() {
                let _ = run(app, "ui.quickaccess.move", json!({"id": state.command, "to": at.saturating_sub(1)}), state);
            }
            if ui.add_enabled(at + 1 < items.len(), egui::Button::new(crate::tl!("Down"))).clicked() {
                let _ = run(app, "ui.quickaccess.move", json!({"id": state.command, "to": at + 1}), state);
            }
        }
    });
    commands(app, ui, state);
    if ui.button(crate::tl!("Add to Quick Access")).clicked() {
        let _ = run(app, "ui.quickaccess.pin", json!({"id": state.command}), state);
    }
    let mut favorite = app.ws.effective.layout.favorites.contains(&state.command);
    if ui.checkbox(&mut favorite, crate::tl!("Favorite")).changed() {
        let _ = run(app, if favorite { "ui.favorite.add" } else { "ui.favorite.remove" }, json!({"id": state.command}), state);
    }
}
