//! Ribbon lists edit persisted ids so selection survives label and order changes.

use serde_json::json;

use super::{State, commands, edit, row, run};
use crate::CadApp;

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State) {
    let mut tabs = app.ws.effective.layout.ribbon.tabs.clone();
    tabs.sort_by_key(|tab| tab.order);
    if !tabs.iter().any(|tab| tab.id == state.tab) {
        state.tab = tabs.first().map(|tab| tab.id.clone()).unwrap_or_default();
    }
    ui.label(crate::tl!("Ribbon tabs"));
    egui::ScrollArea::vertical().id_salt("tabs").max_height(90.0).show(ui, |ui| {
        for tab in &tabs {
            if row(ui, tab.id == state.tab, &tab.label) {
                state.tab = tab.id.clone();
                state.group.clear();
            }
        }
    });
    let response = edit(ui, &mut state.label, crate::tl!("Tab or group name"));
    if std::mem::take(&mut state.focus_label) {
        response.request_focus();
    }
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Add Tab")).clicked() {
            let label = if state.label.trim().is_empty() { crate::tl!("New Tab").to_string() } else { state.label.clone() };
            if let Some(value) = run(app, "ui.ribbon.tab.add", json!({"label": label}), state) {
                state.tab = value.get("id").and_then(serde_json::Value::as_str).unwrap_or_default().into();
                state.label.clear();
            }
        }
        if ui.button(crate::tl!("Rename Tab")).clicked() {
            if state.label.trim().is_empty() {
                state.label = tabs.iter().find(|tab| tab.id == state.tab).map(|tab| tab.label.clone()).unwrap_or_default();
                state.focus_label = true;
            } else {
                let _ = run(app, "ui.ribbon.tab.rename", json!({"tab": state.tab, "label": state.label}), state);
            }
        }
        if ui.button(crate::tl!("Remove Tab")).clicked() {
            let _ = run(app, "ui.ribbon.tab.remove", json!({"tab": state.tab}), state);
        }
        if let Some(at) = tabs.iter().position(|tab| tab.id == state.tab) {
            if ui.add_enabled(at > 0, egui::Button::new(crate::tl!("Tab Up"))).clicked() {
                let _ = run(app, "ui.ribbon.tab.move", json!({"tab": state.tab, "to": at.saturating_sub(1)}), state);
            }
            if ui.add_enabled(at + 1 < tabs.len(), egui::Button::new(crate::tl!("Tab Down"))).clicked() {
                let _ = run(app, "ui.ribbon.tab.move", json!({"tab": state.tab, "to": at + 1}), state);
            }
        }
    });
    let groups = tabs.iter().find(|tab| tab.id == state.tab).map(|tab| tab.groups.clone()).unwrap_or_default();
    if !groups.iter().any(|group| group.id == state.group) {
        state.group = groups.first().map(|group| group.id.clone()).unwrap_or_default();
    }
    ui.label(crate::tl!("Ribbon groups"));
    egui::ScrollArea::vertical().id_salt("groups").max_height(80.0).show(ui, |ui| {
        for group in &groups {
            if row(ui, group.id == state.group, &group.label) {
                state.group = group.id.clone();
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Add Group")).clicked() {
            let label = if state.label.trim().is_empty() { crate::tl!("New Group").to_string() } else { state.label.clone() };
            let _ = run(app, "ui.ribbon.group.add", json!({"tab": state.tab, "label": label}), state);
        }
        if ui.button(crate::tl!("Rename Group")).clicked() {
            let _ = run(app, "ui.ribbon.group.rename", json!({"tab": state.tab, "group": state.group, "label": state.label}), state);
        }
        if ui.button(crate::tl!("Remove Group")).clicked() {
            let _ = run(app, "ui.ribbon.group.remove", json!({"tab": state.tab, "group": state.group}), state);
        }
        if let Some(at) = groups.iter().position(|group| group.id == state.group) {
            if ui.add_enabled(at > 0, egui::Button::new(crate::tl!("Group Up"))).clicked() {
                let _ = run(app, "ui.ribbon.group.move", json!({"tab": state.tab, "group": state.group, "to": at.saturating_sub(1)}), state);
            }
            if ui.add_enabled(at + 1 < groups.len(), egui::Button::new(crate::tl!("Group Down"))).clicked() {
                let _ = run(app, "ui.ribbon.group.move", json!({"tab": state.tab, "group": state.group, "to": at + 1}), state);
            }
        }
    });
    commands(app, ui, state);
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Add Command")).clicked() {
            let _ = run(app, "ui.ribbon.button.add", json!({"tab": state.tab, "group": state.group, "id": state.command}), state);
        }
        if ui.button(crate::tl!("Remove Command")).clicked() {
            let _ = run(app, "ui.ribbon.button.remove", json!({"tab": state.tab, "group": state.group, "id": state.command}), state);
        }
    });
}
