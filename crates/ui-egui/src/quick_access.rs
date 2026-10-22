//! Quick Access commands and the command line's shared, favorite-ranked search.

use serde_json::{Value, json};

use crate::CadApp;
use crate::cmdline::Suggestion;

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

fn button(app: &mut CadApp, ui: &mut egui::Ui, id: &str) {
    let command = crate::toolbars::resolve(id);
    if !command.known {
        ui.add_enabled(false, egui::Button::new(crate::toolbars::placeholder_label(&command)));
        return;
    }
    let label = crate::i18n::t(&command.label);
    let label = match command.shortcut {
        Some(shortcut) => format!("{label} ({})", crate::menus::shortcut_label(shortcut, ui.ctx().os().is_mac())),
        None => label.to_string(),
    };
    let enabled = cadcraft_engine::find_command(id).is_none_or(|spec| (spec.enabled)(&app.session).is_ok());
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            if let Some(icon) = command.icon { crate::icons::button(ui, icon, 18.0, &label, false) } else { ui.small_button(&label) }
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
    });
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let items = app.ws.effective.layout.quick_access.clone();
    if items.is_empty() {
        return;
    }
    ui.push_id("quick_access", |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for id in items.iter().take(5) {
            ui.push_id(id, |ui| button(app, ui, id));
        }
        if items.len() > 5 {
            ui.menu_button(crate::tl!("More Quick Access commands"), |ui| {
                for id in items.iter().skip(5) {
                    ui.push_id(id, |ui| button(app, ui, id));
                }
            });
        }
        let mut query = String::new();
        let response =
            ui.add(egui::TextEdit::singleline(&mut query).hint_text(crate::tl!("Search commands...")).desired_width(100.0).char_limit(1024));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, crate::tl!("Search commands...")));
        if response.clicked() || response.changed() {
            if let Err(error) = app.run("ui.palette", json!({"query": query})) {
                app.set_status(error);
            }
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
