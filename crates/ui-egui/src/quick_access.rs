//! Quick Access commands and the command line's shared, favorite-ranked search.

use serde_json::{Value, json};

use crate::CadApp;
use crate::cmdline::Suggestion;
use crate::wsprofile::schema::{Density, QuickAccessPosition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAccessRow {
    TitleBar,
    OwnRowAboveRibbon,
    OwnRowBelowRibbon,
}

pub fn quick_access_row(app: &CadApp) -> QuickAccessRow {
    match app.ws.effective.layout.quick_access_position {
        // The title row sits above the in-window menu bar, as in a desktop CAD window.
        QuickAccessPosition::TitleBar => QuickAccessRow::TitleBar,
        QuickAccessPosition::AboveRibbon => QuickAccessRow::OwnRowAboveRibbon,
        QuickAccessPosition::BelowRibbon => QuickAccessRow::OwnRowBelowRibbon,
    }
}

struct DragItem(String);

pub fn search(app: &CadApp, prefix: &str) -> Vec<Suggestion> {
    if prefix.is_empty() || prefix.contains(char::is_whitespace) {
        return Vec::new();
    }
    let index = crate::palette::index::SearchIndex::build(app.language(), &crate::shortcuts::active(app));
    let legacy = crate::cmdline::all_suggestions(prefix);
    index
        .search(prefix, &app.session.recent_commands(8), &app.ws.effective.layout.favorites, 8)
        .iter()
        .filter_map(|hit| {
            let entry = index.entry(hit)?;
            let id = entry.id.to_ascii_uppercase();
            let display = legacy.iter().find(|suggestion| suggestion.id == id).map_or_else(|| id.clone(), |suggestion| suggestion.display.clone());
            Some(Suggestion { id, display, label: entry.label_en })
        })
        .collect()
}

pub(crate) fn route(app: &mut CadApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if command == "ui.quickaccess.position" {
        return Some((|| {
            let position = match params.get("position").and_then(Value::as_str) {
                Some("title_bar") => QuickAccessPosition::TitleBar,
                Some("above_ribbon") => QuickAccessPosition::AboveRibbon,
                Some("below_ribbon") => QuickAccessPosition::BelowRibbon,
                _ => return Err(crate::tl!("position must be title_bar, above_ribbon or below_ribbon").to_string()),
            };
            let changed = crate::wsprofile::edit_layout(app, |layout| {
                layout.quick_access_position = position;
                Ok(())
            })?;
            Ok(json!({"position": position, "changed": changed}))
        })());
    }
    if !matches!(command, "ui.quickaccess.pin" | "ui.quickaccess.unpin" | "ui.quickaccess.move" | "ui.favorite.add" | "ui.favorite.remove") {
        return None;
    }
    Some((|| {
        let id = params.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| crate::tl!("id is required").to_string())?;
        let favorites = command.starts_with("ui.favorite.");
        let mut items = if favorites { app.ws.effective.layout.favorites.clone() } else { app.ws.effective.layout.quick_access.clone() };
        match command {
            "ui.quickaccess.pin" | "ui.favorite.add" => {
                if !items.iter().any(|held| held == id) {
                    items.push(id.to_string());
                }
            }
            "ui.quickaccess.move" => {
                let to = params
                    .get("to")
                    .and_then(Value::as_u64)
                    .and_then(|to| usize::try_from(to).ok())
                    .ok_or_else(|| crate::tl!("to must be a whole number").to_string())?;
                let from = items.iter().position(|held| held == id).ok_or_else(|| crate::tl!("Quick Access item is not pinned").to_string())?;
                let item = items.remove(from);
                items.insert(to.min(items.len()), item);
            }
            _ => items.retain(|held| held != id),
        }
        let changed = crate::wsprofile::edit_layout(app, |layout| {
            if favorites {
                layout.favorites = items;
            } else {
                layout.quick_access = items;
            }
            Ok(())
        })?;
        Ok(json!({"id": id, "changed": changed}))
    })())
}

fn button(app: &mut CadApp, ui: &mut egui::Ui, id: &str) -> egui::Response {
    let command = crate::toolbars::resolve(id);
    let density = app.ws.effective.layout.density;
    let comfortable = density == Density::Comfortable;
    let size = if comfortable { 32.0 } else { 18.0 };
    if !command.known {
        return ui.add_enabled(false, egui::Button::new(crate::toolbars::placeholder_label(&command)).min_size(egui::vec2(size, size)));
    }
    let label = crate::i18n::t(&command.label);
    let label = match command.shortcut {
        Some(shortcut) => format!("{label} ({})", crate::menus::shortcut_label(shortcut, ui.ctx().os().is_mac())),
        None => label.to_string(),
    };
    let enabled = cadcraft_engine::find_command(id).is_none_or(|spec| (spec.enabled)(&app.session).is_ok());
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            if let Some(icon) = command.icon {
                crate::icons::button(ui, icon, size, &label, false)
            } else if density != Density::Normal {
                ui.add(egui::Button::new(&label).min_size(egui::vec2(size, size)))
            } else {
                ui.small_button(&label)
            }
        })
        .inner;
    if response.clicked() {
        crate::menus::activate(app, id);
    }
    response.context_menu(|ui| {
        if ui.button(crate::tl!("Remove from Quick Access")).clicked() {
            if let Err(error) = app.run("ui.quickaccess.unpin", json!({"id": id})) {
                app.set_status(error);
            }
            ui.close();
        }
        let favorite = app.ws.effective.layout.favorites.iter().any(|held| held == id);
        if ui.button(if favorite { crate::tl!("Remove Favorite") } else { crate::tl!("Add Favorite") }).clicked() {
            if let Err(error) = app.run(if favorite { "ui.favorite.remove" } else { "ui.favorite.add" }, json!({"id": id})) {
                app.set_status(error);
            }
            ui.close();
        }
        ui.separator();
        position_menu(app, ui);
    });
    response
}

pub(crate) fn pin_menu(ui: &mut egui::Ui, pinned: bool, id: &str) -> Option<(&'static str, Value)> {
    let (label, command) = if pinned {
        (crate::tl!("Remove from Quick Access"), "ui.quickaccess.unpin")
    } else {
        (crate::tl!("Add to Quick Access"), "ui.quickaccess.pin")
    };
    if ui.button(label).clicked() {
        ui.close();
        Some((command, json!({"id": id})))
    } else {
        None
    }
}

fn position_menu(app: &mut CadApp, ui: &mut egui::Ui) {
    let below = quick_access_row(app) == QuickAccessRow::OwnRowBelowRibbon;
    let label = if below { crate::tl!("Show Above the Ribbon") } else { crate::tl!("Show Below the Ribbon") };
    if ui.button(label).clicked() {
        let position = if below { "above_ribbon" } else { "below_ribbon" };
        if let Err(error) = app.run("ui.quickaccess.position", json!({"position": position})) {
            app.set_status(error);
        }
        ui.close();
    }
}

fn item(app: &mut CadApp, ui: &mut egui::Ui, id: &str, at: usize, count: usize) {
    // Explicit identity keeps focus on the command when its position changes.
    ui.scope_builder(egui::UiBuilder::new().id(ui.id().with(id)), |ui| {
        // `dnd_drag_source` puts a drag-only widget on top of the button, which swallows its clicks.
        let response = button(app, ui, id).interact(egui::Sense::drag());
        response.dnd_set_drag_payload(DragItem(id.to_string()));
        if let Some(payload) = response.dnd_release_payload::<DragItem>()
            && let Err(error) = app.run("ui.quickaccess.move", json!({"id": payload.0, "to": at}))
        {
            app.set_status(error);
        }
        if response.has_focus() && !app.cmd.ime_composing() && !ui.ctx().text_edit_focused() {
            let to = if ui.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)) {
                Some(at.saturating_sub(1))
            } else if ui.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)) {
                Some(at.saturating_add(1).min(count.saturating_sub(1)))
            } else {
                None
            };
            if let Some(to) = to {
                if let Err(error) = app.run("ui.quickaccess.move", json!({"id": id, "to": to})) {
                    app.set_status(error);
                }
                response.request_focus();
                ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            }
        }
    });
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let items = app.ws.effective.layout.quick_access.clone();
    if items.is_empty() {
        return;
    }
    ui.push_id("quick_access", |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (at, id) in items.iter().take(5).enumerate() {
            item(app, ui, id, at, items.len());
        }
        if items.len() > 5 {
            let size = if app.ws.effective.layout.density == Density::Comfortable { 32.0 } else { 18.0 };
            // A glyph keeps the title row's width for the drawing name; the label stays for tooltips and readers.
            let more = crate::icons::button(ui, crate::icons::Icon::ChevronDown, size, crate::tl!("More Quick Access commands"), false);
            egui::Popup::menu(&more).id(more.id.with("more_quick_access")).show(|ui| {
                for (at, id) in items.iter().enumerate().skip(5) {
                    item(app, ui, id, at, items.len());
                }
            });
        }
        let mut query = String::new();
        let response =
            ui.add(egui::TextEdit::singleline(&mut query).hint_text(crate::tl!("Search commands...")).desired_width(160.0).char_limit(1024));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, crate::tl!("Search commands...")));
        if (response.clicked() || response.changed())
            && let Err(error) = app.run("ui.palette", json!({"query": query}))
        {
            app.set_status(error);
        }
        let mut action = None;
        let active =
            app.ws.choices().into_iter().find(|choice| choice.id == app.ws.active).map(|choice| choice.name).unwrap_or_else(|| app.ws.active.clone());
        egui::ComboBox::from_id_salt("workspace")
            .selected_text(crate::tf!("Workspace: {name}", name = active))
            .width(110.0)
            .show_ui(ui, |ui| action = crate::wsprofile::picker::menu(ui, &app.ws))
            .response
            .on_hover_text(crate::tl!("Workspace Switching"));
        if let Some(action) = action {
            crate::wsprofile::picker::run_menu_action(app, action);
        }
        ui.separator();
    });
}
