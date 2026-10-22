//! Ribbon lists edit persisted ids so selection survives label and order changes.

use serde_json::json;

use super::{State, commands, edit, row, run};
use crate::CadApp;
use crate::wsprofile::schema::{RibbonGroup, RibbonMode};

#[derive(Clone)]
struct RibbonDrag {
    tab: String,
    group: String,
    index: usize,
}

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State) {
    // egui clears the payload on Escape; stop the source from recreating it while the pointer stays down.
    if state.ribbon_drag_active && !egui::DragAndDrop::has_payload_of_type::<RibbonDrag>(ui.ctx()) && ui.input(|input| input.pointer.primary_down()) {
        ui.ctx().stop_dragging();
    }
    let mut collapsed = app.ws.effective.layout.ribbon.effective_mode() == RibbonMode::TabsOnly;
    if ui.checkbox(&mut collapsed, crate::tl!("Minimize ribbon")).changed() {
        let _ = run(app, "ui.ribbon.collapse", json!({"collapsed": collapsed}), state);
    }
    let mut tabs = app.ws.effective.layout.ribbon.tabs.clone();
    tabs.sort_by_key(|tab| tab.order);
    if !tabs.iter().any(|tab| tab.id == state.tab) {
        state.tab = tabs.first().map(|tab| tab.id.clone()).unwrap_or_default();
    }
    ui.label(crate::tl!("Ribbon tabs"));
    egui::ScrollArea::vertical().id_salt("tabs").max_height(90.0).show(ui, |ui| {
        for tab in &tabs {
            if row(ui, tab.id == state.tab, crate::ribbon::render::tab_label(tab)) {
                state.tab = tab.id.clone();
                state.group.clear();
                state.ribbon_index = None;
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
        let movable: Vec<_> = tabs.iter().filter(|tab| tab.contextual.is_none()).collect();
        if let Some(at) = movable.iter().position(|tab| tab.id == state.tab) {
            if ui.add_enabled(at > 0, egui::Button::new(crate::tl!("Tab Up"))).clicked() {
                let _ = run(app, "ui.ribbon.tab.move", json!({"tab": state.tab, "to": at.saturating_sub(1)}), state);
            }
            if ui.add_enabled(at + 1 < movable.len(), egui::Button::new(crate::tl!("Tab Down"))).clicked() {
                let _ = run(app, "ui.ribbon.tab.move", json!({"tab": state.tab, "to": at + 1}), state);
            }
        }
    });
    let selected_tab = tabs.iter().find(|tab| tab.id == state.tab);
    let groups = selected_tab.map(|tab| tab.groups.clone()).unwrap_or_default();
    if !groups.iter().any(|group| group.id == state.group) {
        state.group = groups.first().map(|group| group.id.clone()).unwrap_or_default();
    }
    ui.label(crate::tl!("Ribbon groups"));
    egui::ScrollArea::vertical().id_salt("groups").max_height(80.0).show(ui, |ui| {
        for group in &groups {
            let label = selected_tab.map_or(group.label.as_str(), |tab| crate::ribbon::render::group_label(tab, group));
            let (zone, dropped) = ui.dnd_drop_zone::<RibbonDrag, _>(egui::Frame::NONE, |ui| row(ui, group.id == state.group, label));
            if zone.inner {
                state.group = group.id.clone();
                state.ribbon_index = None;
            }
            if let Some(payload) = zone.response.dnd_hover_payload::<RibbonDrag>()
                && payload.tab == state.tab
            {
                marker(ui, zone.response.rect, true);
            }
            if let Some(payload) = dropped {
                drop_command(app, state, &payload, &group.id, group.commands.len());
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
    if let Some(group) = groups.iter().find(|group| group.id == state.group) {
        command_rows(app, ui, state, group);
    }
    state.ribbon_drag_active = egui::DragAndDrop::has_payload_of_type::<RibbonDrag>(ui.ctx());
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

fn marker(ui: &egui::Ui, rect: egui::Rect, before: bool) {
    let y = if before { rect.top() } else { rect.bottom() };
    ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(2.0, crate::theme::Tokens::get().accent));
}

fn drop_command(app: &mut CadApp, state: &mut State, drag: &RibbonDrag, target: &str, insertion: usize) {
    if drag.tab != state.tab {
        return;
    }
    let command = app
        .ws
        .effective
        .layout
        .ribbon
        .tabs
        .iter()
        .find(|tab| tab.id == drag.tab)
        .and_then(|tab| tab.groups.iter().find(|group| group.id == drag.group))
        .and_then(|group| group.commands.get(drag.index))
        .cloned();
    let Some(command) = command else { return };
    let to = if drag.group == target && drag.index < insertion { insertion.saturating_sub(1) } else { insertion };
    let params = if drag.group == target {
        json!({"tab": drag.tab, "group": drag.group, "from": drag.index, "to": to})
    } else {
        json!({"tab": drag.tab, "group": drag.group, "command": command, "to_group": target, "to": to})
    };
    if run(app, "ui.ribbon.button.move", params, state).is_some() {
        state.group = target.into();
        state.command = command;
        state.ribbon_index = Some(to);
        state.ribbon_focus = Some(to);
    }
}

fn reorder(app: &mut CadApp, state: &mut State, from: usize, to: usize) {
    if run(app, "ui.ribbon.button.move", json!({"tab": state.tab, "group": state.group, "from": from, "to": to}), state).is_some() {
        state.ribbon_index = Some(to);
        state.ribbon_focus = Some(to);
    }
}

fn command_rows(app: &mut CadApp, ui: &mut egui::Ui, state: &mut State, group: &RibbonGroup) {
    ui.label(crate::tl!("Ribbon commands"));
    let focus = state.ribbon_focus.take();
    let mut movement = None;
    egui::ScrollArea::vertical().id_salt(("ribbon_commands", &state.tab, &state.group)).max_height(160.0).show(ui, |ui| {
        for (index, command) in group.commands.iter().enumerate() {
            let payload = RibbonDrag { tab: state.tab.clone(), group: state.group.clone(), index };
            let id = ui.make_persistent_id(("ribbon_command", index));
            let (zone, dropped) = ui.dnd_drop_zone::<RibbonDrag, _>(egui::Frame::NONE, |ui| {
                ui.push_id(id, |ui| {
                    let resolved = crate::toolbars::resolve(command);
                    // A separate drag-source overlay would swallow clicks needed to select the row.
                    let response = ui
                        .add(egui::Button::selectable(state.ribbon_index == Some(index), crate::i18n::t(&resolved.label)))
                        .interact(egui::Sense::drag());
                    response.dnd_set_drag_payload(payload);
                    if focus == Some(index) {
                        response.request_focus();
                        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                        response.scroll_to_me(None);
                    }
                    if response.gained_focus() {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() || response.gained_focus() {
                        state.ribbon_index = Some(index);
                        state.command = command.clone();
                    }
                    if response.has_focus() {
                        let up = ui.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp));
                        let down = ui.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown));
                        if up || down {
                            // Suppress egui's spatial navigation even at the first and last rows.
                            response.request_focus();
                            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                            if up && index > 0 {
                                movement = Some((index, index - 1));
                            } else if down && index + 1 < group.commands.len() {
                                movement = Some((index, index + 1));
                            }
                        }
                    }
                });
            });
            let before = ui.input(|input| input.pointer.latest_pos().is_none_or(|pos| pos.y < zone.response.rect.center().y));
            if let Some(payload) = zone.response.dnd_hover_payload::<RibbonDrag>()
                && payload.tab == state.tab
            {
                marker(ui, zone.response.rect, before);
            }
            if let Some(payload) = dropped {
                drop_command(app, state, &payload, &group.id, if before { index } else { index + 1 });
            }
        }
        if group.commands.is_empty() {
            let (zone, dropped) = ui.dnd_drop_zone::<RibbonDrag, _>(egui::Frame::NONE, |ui| {
                ui.allocate_space(egui::vec2(ui.available_width(), 24.0));
            });
            if zone.response.dnd_hover_payload::<RibbonDrag>().is_some() {
                marker(ui, zone.response.rect, true);
            }
            if let Some(payload) = dropped {
                drop_command(app, state, &payload, &group.id, 0);
            }
        }
    });
    ui.horizontal(|ui| {
        let selected = state.ribbon_index.filter(|index| *index < group.commands.len());
        if ui.add_enabled(selected.is_some_and(|index| index > 0), egui::Button::new(crate::tl!("Up"))).clicked()
            && let Some(index) = selected
        {
            movement = Some((index, index.saturating_sub(1)));
        }
        if ui.add_enabled(selected.is_some_and(|index| index + 1 < group.commands.len()), egui::Button::new(crate::tl!("Down"))).clicked()
            && let Some(index) = selected
        {
            movement = Some((index, index + 1));
        }
    });
    if let Some((from, to)) = movement {
        reorder(app, state, from, to);
        ui.ctx().request_repaint();
    }
}
