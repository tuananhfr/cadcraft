//! The workspace picker (first run), the Workspace Switching menu, the Save As name dialog and the
//! status bar's save-state text. All of it only calls the `ui.workspace.*` commands.

use egui::{Align2, Color32, Rect, Sense, vec2};
use serde_json::json;

use super::presets::SAFE_PRESET;
use super::startup::Fallback;
use super::{Choice, SaveStatus, WsState, commands::MAX_NAME};
use crate::CadApp;
use crate::theme::Tokens;

pub const PICKER_DIALOG: &str = "workspace_picker";
pub const SAVE_AS_DIALOG: &str = "workspace_saveas";

const PICKER_WIDTH: f32 = 460.0;
const ROW_HEIGHT: f32 = 48.0;

/// What the Workspace Switching menu asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Select(String),
    SaveAs,
    Reset,
}

fn preset_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "SIMPLE" => crate::tl!("Simple"),
        "AUTOCAD_FAMILIAR" => crate::tl!("AutoCAD Familiar"),
        "PLANNING_GIS" => crate::tl!("Planning and GIS"),
        "ARCHITECTURE" => crate::tl!("Architecture"),
        "STRUCTURE" => crate::tl!("Structure"),
        "MEP_FIRE" => crate::tl!("MEP and Fire"),
        "INFRA" => crate::tl!("Infrastructure"),
        "QS_QA" => crate::tl!("Quantity Survey and QA"),
        "SHEET_PUBLISH" => crate::tl!("Sheets and Publishing"),
        _ => return None,
    })
}

fn preset_description(id: &str) -> &'static str {
    match id {
        "SIMPLE" => crate::tl!("The CADCraft window as it is today: tool bar, Tool Sets and palettes."),
        "AUTOCAD_FAMILIAR" => crate::tl!("Ribbon with tabs and groups in the order AutoCAD users expect."),
        "PLANNING_GIS" => crate::tl!("Ribbon plus a survey tool bar and base map attachments."),
        "ARCHITECTURE" => crate::tl!("Walls and openings, schedules and an architecture tool bar."),
        "STRUCTURE" => crate::tl!("Structural grids, constraints and a structure tool bar."),
        "MEP_FIRE" => crate::tl!("Routing, symbols and take-off for services and fire protection."),
        "INFRA" => crate::tl!("Alignments and station annotation for infrastructure."),
        "QS_QA" => crate::tl!("Quantity take-off, drawing comparison and checks."),
        "SHEET_PUBLISH" => crate::tl!("Layouts, plotting and publishing."),
        _ => crate::tl!("A workspace you saved."),
    }
}

fn display_name(choice: &Choice) -> String {
    if choice.preset { preset_name(&choice.id).map_or_else(|| choice.name.clone(), str::to_string) } else { choice.name.clone() }
}

/// The picker's rows; the id of the one clicked or activated from the keyboard.
fn rows(ui: &mut egui::Ui, choices: &[Choice], active: &str) -> Option<String> {
    let t = Tokens::get();
    let mut chosen = None;
    ui.set_width(PICKER_WIDTH);
    ui.label(
        egui::RichText::new(crate::tl!("Pick the layout that suits your work. You can change it any time from the status bar.")).color(t.text_dim),
    );
    ui.add_space(6.0);
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
        for choice in choices {
            let name = display_name(choice);
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
            let fill = if choice.id == active {
                t.tab_active
            } else if response.hovered() || response.has_focus() {
                t.control_hover
            } else {
                Color32::TRANSPARENT
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, fill);
            if response.has_focus() {
                painter.rect_stroke(rect.shrink(1.0), 4.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
            let heading = painter.layout_no_wrap(name.clone(), crate::theme::body(), t.text);
            painter.galley(rect.left_top() + vec2(10.0, 8.0), heading.clone(), t.text);
            let description = painter.layout(
                preset_description(if choice.preset { &choice.id } else { "" }).to_string(),
                crate::theme::small(),
                t.text_dim,
                rect.width() - 20.0,
            );
            painter.galley(rect.left_top() + vec2(10.0, 8.0 + heading.size().y + 2.0), description, t.text_dim);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
            if choice.id == "AUTOCAD_FAMILIAR" {
                familiar_tag(ui, rect, heading.size().x);
            }
            if response.clicked() {
                chosen = Some(choice.id.clone());
            }
        }
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(crate::tl!("Discipline workspaces are drafts that await CAD Manager review.")).small().color(t.text_faint));
    chosen
}

/// A small text chip after the name; words only.
fn familiar_tag(ui: &mut egui::Ui, row: Rect, name_width: f32) {
    let t = Tokens::get();
    let text = crate::tl!("Familiar to AutoCAD users");
    let galley = ui.painter().layout_no_wrap(text.to_string(), crate::theme::small(), t.accent);
    let chip = Rect::from_min_size(row.left_top() + vec2(10.0 + name_width + 10.0, 8.0), galley.size() + vec2(10.0, 2.0));
    ui.painter().rect_stroke(chip, 8.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    ui.painter().galley(chip.min + vec2(5.0, 1.0), galley, t.accent);
    let response = ui.interact(chip, ui.id().with("familiar_tag"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
}

fn choose(app: &mut CadApp, id: &str) {
    if let Err(error) = app.run("ui.workspace.select", json!({"id": id})) {
        app.set_status(error);
    }
}

/// The picker. Closing it unchosen on a first run keeps SIMPLE and stores it, so it is not offered again.
pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let choices = app.ws.choices();
    let active = app.ws.active.clone();
    let mut chosen = None;
    egui::Window::new(crate::tl!("Choose a Workspace"))
        .id(egui::Id::new(PICKER_DIALOG))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| chosen = rows(ui, &choices, &active));
    if let Some(id) = chosen {
        *open = false;
        choose(app, &id);
        return;
    }
    let escape = ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if escape || !*open {
        *open = false;
        if std::mem::take(&mut app.ws.first_run_picker_open) {
            choose(app, SAFE_PRESET);
        }
    }
}

/// The name prompt of Save As.
pub fn save_as_dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let id = egui::Id::new(SAVE_AS_DIALOG);
    let mut name: String = ctx.data(|data| data.get_temp(id)).unwrap_or_default();
    let error: Option<String> = ctx.data(|data| data.get_temp(id.with("error")));
    let focused: bool = ctx.data(|data| data.get_temp(id.with("focused"))).unwrap_or(false);
    let (mut submit, mut cancel) = (false, false);
    egui::Window::new(crate::tl!("Save Workspace As"))
        .id(id)
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(crate::tl!("Name"));
            let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0).char_limit(MAX_NAME));
            if !focused {
                edit.request_focus();
            }
            if edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            if let Some(error) = &error {
                ui.colored_label(Tokens::get().warn, error);
            }
            ui.horizontal(|ui| {
                submit |= ui.button(crate::tl!("Save")).clicked();
                cancel |= ui.button(crate::tl!("Cancel")).clicked();
            });
        });
    cancel |= ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut finished = cancel || !*open;
    let mut failure = None;
    if submit && !cancel {
        match app.run("ui.workspace.saveas", json!({"name": name})) {
            Ok(_) => finished = true,
            Err(message) => failure = Some(message),
        }
    }
    ctx.data_mut(|data| {
        if finished {
            data.remove::<String>(id);
            data.remove::<String>(id.with("error"));
            data.remove::<bool>(id.with("focused"));
        } else {
            data.insert_temp(id, name);
            data.insert_temp(id.with("focused"), true);
            match failure {
                Some(message) => {
                    data.insert_temp(id.with("error"), message);
                }
                None => {
                    data.remove::<String>(id.with("error"));
                }
            }
        }
    });
    if finished {
        *open = false;
    }
}

/// The Workspace Switching menu: the workspaces, then Customize, Save As and Reset.
pub fn menu(ui: &mut egui::Ui, ws: &WsState) -> Option<MenuAction> {
    let mut action = None;
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        for choice in ws.choices() {
            if ui.add(egui::Button::selectable(choice.id == ws.active, display_name(&choice)).truncate()).clicked() {
                action = Some(MenuAction::Select(choice.id));
                ui.close();
            }
        }
    });
    ui.separator();
    ui.add_enabled(false, egui::Button::new(crate::tl!("Customize..."))).on_disabled_hover_text(crate::tl!("Available in a later update"));
    if ui.button(crate::tl!("Save As...")).clicked() {
        action = Some(MenuAction::SaveAs);
        ui.close();
    }
    if ui.button(crate::tl!("Reset")).clicked() {
        action = Some(MenuAction::Reset);
        ui.close();
    }
    action
}

pub fn run_menu_action(app: &mut CadApp, action: MenuAction) {
    match action {
        MenuAction::Select(id) => choose(app, &id),
        MenuAction::SaveAs => app.start("ui.workspace.saveas"),
        MenuAction::Reset => {
            if let Err(error) = app.run("ui.workspace.reset", json!({})) {
                app.set_status(error);
            }
        }
    }
}

/// The save state in the status bar's words, and a longer explanation when there is one. `None`
/// while there is nothing to say: no storage, or a first run that has changed nothing yet.
pub fn save_text(ws: &WsState) -> Option<(&'static str, Option<String>)> {
    ws.store.as_ref()?;
    Some(match &ws.status {
        SaveStatus::SavedLocally => (crate::tl!("Saved locally"), None),
        SaveStatus::Unsaved if ws.autosave.pending() => (crate::tl!("Saving…"), None),
        SaveStatus::Unsaved if matches!(ws.fallback, Some(Fallback::NoProfile)) => return None,
        SaveStatus::Unsaved => (crate::tl!("Not saved"), None),
        SaveStatus::Failed(error) => (crate::tl!("Not saved"), Some(format!("{}: {error}", crate::tl!("Could not save workspace")))),
        SaveStatus::Syncing => (crate::tl!("Syncing"), None),
        SaveStatus::Synced => (crate::tl!("Synced"), None),
        SaveStatus::Offline => (crate::tl!("Offline"), None),
        SaveStatus::Conflict => (crate::tl!("Sync conflict"), None),
    })
}

/// Paints the save state with its right edge 8 px left of `right`; returns where the controls to
/// its left must end (`right` itself when there is nothing to show).
pub fn status(app: &CadApp, ui: &mut egui::Ui, right: f32, cy: f32) -> f32 {
    let Some((text, detail)) = save_text(&app.ws) else { return right };
    let t = Tokens::get();
    let color = if matches!(app.ws.status, SaveStatus::Failed(_) | SaveStatus::Conflict) { t.warn } else { t.text_dim };
    let galley = ui.painter().layout_no_wrap(text.to_string(), crate::theme::small(), color);
    let rect = Rect::from_min_size(egui::pos2(right - 8.0 - galley.size().x, cy - galley.size().y / 2.0), galley.size());
    ui.painter().galley(rect.min, galley, color);
    let response = ui.interact(rect.expand2(vec2(2.0, 4.0)), ui.id().with("ws_save_state"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    if let Some(detail) = detail {
        response.on_hover_text(detail);
    }
    rect.left() - 8.0
}
