//! Tool bar customization reuses the shipped bar commands and profile field editor.

use serde_json::json;

use super::{State, commands, edit, row, run};
use crate::CadApp;

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State) {
    let bars = app.ws.effective.layout.toolbars.clone();
    if state.toolbar.is_empty() {
        state.toolbar = crate::toolbars::known_ids(app).first().cloned().unwrap_or_default();
    }
    ui.label(crate::tl!("Toolbars"));
    for id in crate::toolbars::known_ids(app) {
        let label = bars.iter().find(|bar| bar.id == id).map(|bar| bar.label.clone()).unwrap_or_else(|| {
            crate::toolbars::BUILTIN.iter().find(|(known, _)| *known == id).map(|(_, label)| (*label).into()).unwrap_or_else(|| id.clone())
        });
        if row(ui, state.toolbar == id, &label) {
            state.toolbar = id;
        }
    }
    edit(ui, &mut state.label, crate::tl!("Toolbar name"));
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Add")).clicked() {
            let _ = run(app, "ui.toolbar.show", json!({"id": state.toolbar}), state);
        }
        if ui.button(crate::tl!("Remove")).clicked() {
            let _ = run(app, "ui.toolbar.hide", json!({"id": state.toolbar}), state);
        }
        if ui.button(crate::tl!("Rename")).clicked() {
            let pointer = format!("/toolbars/{}/label", crate::wsprofile::merge::escape_token(&state.toolbar));
            let _ = run(app, "ui.workspace.set", json!({"pointer": pointer, "value": state.label}), state);
        }
        if let Some(bar) = bars.iter().find(|bar| bar.id == state.toolbar) {
            let dock = match bar.dock {
                crate::wsprofile::schema::Dock::Top => "top",
                crate::wsprofile::schema::Dock::Bottom => "bottom",
                crate::wsprofile::schema::Dock::Left => "left",
                crate::wsprofile::schema::Dock::Right => "right",
                crate::wsprofile::schema::Dock::Floating => "floating",
            };
            for (label, order) in [(crate::tl!("Up"), bar.order.saturating_sub(1)), (crate::tl!("Down"), bar.order.saturating_add(1))] {
                if ui.button(label).clicked() {
                    let _ = run(app, "ui.toolbar.move", json!({"id": bar.id, "dock": dock, "row": bar.row, "order": order}), state);
                }
            }
        }
    });
    commands(app, ui, state);
    if let Some(bar) = bars.iter().find(|bar| bar.id == state.toolbar) {
        ui.horizontal(|ui| {
            let mut items = bar.commands.clone();
            let add = ui.button(crate::tl!("Add Command")).clicked();
            let remove = ui.button(crate::tl!("Remove Command")).clicked();
            if add || remove {
                if add && !items.contains(&state.command) {
                    items.push(state.command.clone());
                }
                if remove {
                    items.retain(|id| *id != state.command);
                }
                let pointer = format!("/toolbars/{}/commands", crate::wsprofile::merge::escape_token(&bar.id));
                let _ = run(app, "ui.workspace.set", json!({"pointer": pointer, "value": items}), state);
            }
        });
    }
}
