//! Responsive ribbon groups collapse before the remaining groups need scrolling.

use egui::{Id, Key, Modifiers, Stroke, StrokeKind, vec2};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;
use crate::wsprofile::schema::{RibbonGroup, RibbonTab};

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    if !app.ws.effective.layout.ribbon.visible {
        return;
    }
    let tabs = super::visible_tabs(app);
    let state_id = Id::new("cc_ribbon_active");
    let mut active: String = ui.ctx().data(|data| data.get_temp(state_id)).unwrap_or_default();
    if !tabs.iter().any(|tab| tab.id == active) {
        active = tabs.first().map(|tab| tab.id.clone()).unwrap_or_default();
    }
    let collapsed = app.ws.effective.layout.ribbon.collapsed;
    egui::Panel::top("cc_ribbon").exact_size(if collapsed { 28.0 } else { 142.0 }).frame(egui::Frame::NONE.fill(Tokens::get().chrome)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                for tab in &tabs {
                    let response = ui.add(egui::Button::selectable(active == tab.id, crate::i18n::t(&tab.label)));
                    if response.clicked() || response.double_clicked() {
                        active = tab.id.clone();
                    }
                    if response.double_clicked() {
                        run(app, "ui.ribbon.collapse", json!({"collapsed": !collapsed}));
                    }
                }
            });
            if !collapsed && let Some(tab) = tabs.iter().find(|tab| tab.id == active) {
                groups(app, ui, tab);
            }
        },
    );
    ui.ctx().data_mut(|data| data.insert_temp(state_id, active));
}

fn run(app: &mut CadApp, command: &str, params: serde_json::Value) {
    if let Err(error) = app.run(command, params) {
        app.set_status(error);
    }
}

fn scroll_button(ui: &mut egui::Ui, glyph: &str, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(glyph));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.on_hover_text(label)
}

fn groups(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab) {
    let available = ui.available_width().max(0.0);
    let widths: Vec<_> = tab.groups.iter().map(|group| ((group.commands.len().div_ceil(3) as f32 * 34.0) + 12.0).max(90.0)).collect();
    let mut collapsed = vec![false; widths.len()];
    let mut total = widths.iter().sum::<f32>() + widths.len() as f32 * 8.0;
    for (index, width) in widths.iter().enumerate().rev() {
        if total <= available {
            break;
        }
        if let Some(slot) = collapsed.get_mut(index) {
            *slot = true;
        }
        total -= (width - 76.0).max(0.0);
    }
    let scroll = total > available;
    let scroll_id = Id::new(("cc_ribbon_scroll", &tab.id));
    let mut start: usize = ui.ctx().data(|data| data.get_temp(scroll_id)).unwrap_or(0);
    start = start.min(tab.groups.len().saturating_sub(1));
    if !scroll {
        start = 0;
    }
    ui.horizontal_top(|ui| {
        if scroll && scroll_button(ui, "‹", crate::tl!("Previous ribbon groups"), start > 0).clicked() {
            start = start.saturating_sub(1);
        }
        let mut remaining = (available - if scroll { 68.0 } else { 0.0 }).max(0.0);
        let mut end = start;
        for (index, group) in tab.groups.iter().enumerate().skip(start) {
            let compact = collapsed.get(index).copied().unwrap_or(true);
            let width = if compact { 76.0 } else { widths.get(index).copied().unwrap_or(90.0) };
            if remaining < width && end > start {
                break;
            }
            ui.push_id((&tab.id, &group.id), |ui| {
                ui.allocate_ui(vec2(width.min(remaining).max(1.0), 100.0), |ui| {
                    if compact {
                        ui.menu_button(crate::i18n::t(&group.label), |ui| {
                            for id in &group.commands {
                                button(app, ui, tab, group, id, false);
                            }
                        });
                    } else {
                        for (column, commands) in group.commands.chunks(3).enumerate() {
                            let rect = ui.max_rect();
                            let column_rect = egui::Rect::from_min_size(rect.min + vec2(column as f32 * 34.0, 0.0), vec2(32.0, 82.0));
                            ui.scope_builder(egui::UiBuilder::new().max_rect(column_rect), |ui| {
                                for id in commands {
                                    button(app, ui, tab, group, id, true);
                                }
                            });
                        }
                        ui.allocate_space(vec2(width, 82.0));
                        ui.label(crate::i18n::t(&group.label));
                    }
                });
            });
            remaining -= width + 8.0;
            end = index.saturating_add(1);
        }
        if scroll && scroll_button(ui, "›", crate::tl!("Next ribbon groups"), end < tab.groups.len()).clicked() {
            start = start.saturating_add(1).min(tab.groups.len().saturating_sub(1));
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(scroll_id, start));
}

fn button(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab, group: &RibbonGroup, id: &str, icon: bool) {
    ui.push_id(id, |ui| {
        let mut command = crate::toolbars::resolve(id);
        command.known = super::known(id);
        if let Some(spec) = cadcraft_engine::find_command(id) {
            command.label = spec.label.to_string();
        }
        let label = if command.known { crate::i18n::t(&command.label).to_string() } else { crate::toolbars::placeholder_label(&command) };
        let enabled = command.known && cadcraft_engine::find_command(id).is_none_or(|spec| (spec.enabled)(&app.session).is_ok());
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                if icon && let Some(icon) = command.icon {
                    crate::icons::button(ui, icon, 24.0, &label, false)
                } else {
                    ui.add_sized(if icon { vec2(28.0, 24.0) } else { vec2(140.0, 24.0) }, egui::Button::new(label.as_str()).truncate())
                }
            })
            .inner;
        if response.has_focus() {
            ui.painter().rect_stroke(response.rect.shrink(1.0), 2.0, Stroke::new(1.0, Tokens::get().accent), StrokeKind::Inside);
        }
        crate::control::record_widget(ui, &response, &label, "button", None);
        if response.clicked() {
            crate::menus::activate(app, id);
        }
        let keyboard = response.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::F10));
        let open = (keyboard || response.secondary_clicked()).then_some(egui::SetOpenCommand::Bool(true));
        egui::Popup::menu(&response).id(response.id.with("ribbon_menu")).open_memory(open).show(|ui| {
            let pin = ui.button(crate::tl!("Add to Quick Access"));
            if keyboard {
                pin.request_focus();
            }
            if pin.clicked() {
                run(app, "ui.quickaccess.pin", json!({"id": id}));
                ui.close();
            }
            if ui.button(crate::tl!("Remove from Ribbon")).clicked() {
                run(app, "ui.ribbon.button.remove", json!({"tab": tab.id, "group": group.id, "id": id}));
                ui.close();
            }
        });
        response.on_hover_text(label);
    });
}
