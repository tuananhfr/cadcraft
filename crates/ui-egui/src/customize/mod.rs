//! Keyboard-operable workspace customization; all mutations go through UI commands.

use egui::{Key, vec2};
use serde_json::{Value, json};

use crate::CadApp;

mod keyboard_tab;
mod quick_access_tab;
mod ribbon_tab;
mod toolbars_tab;

pub const DIALOG: &str = "customize_workspace";

#[derive(Default)]
pub struct State {
    page: usize,
    tab: String,
    group: String,
    toolbar: String,
    command: String,
    query: String,
    label: String,
    key: String,
    replace: bool,
    error: Option<String>,
    focus_label: bool,
    focused: bool,
}

pub(crate) fn run(app: &mut CadApp, command: &str, params: Value, state: &mut State) -> Option<Value> {
    match app.run(command, params) {
        Ok(value) => {
            state.error = None;
            Some(value)
        }
        Err(error) => {
            state.error = Some(error);
            None
        }
    }
}

fn edit(ui: &mut egui::Ui, text: &mut String, label: &str) -> egui::Response {
    ui.label(label);
    let response = ui.add(egui::TextEdit::singleline(text).id_salt(label).hint_text(label).char_limit(1024).desired_width(240.0));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    response
}

fn row(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    let response = ui.add(egui::Button::selectable(selected, label));
    if response.has_focus() {
        response.scroll_to_me(None);
        let down = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
        let up = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
        if down || up {
            ui.memory_mut(|memory| memory.move_focus(if down { egui::FocusDirection::Next } else { egui::FocusDirection::Previous }));
        }
    }
    response.clicked()
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut state = std::mem::take(&mut app.customize);
    let mut close = false;
    egui::Window::new(crate::tl!("Customize Workspace…"))
        .id(egui::Id::new(DIALOG))
        .open(open)
        .default_width(700.0)
        .default_height(520.0)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let pages = [crate::tl!("Ribbon"), crate::tl!("Toolbars"), crate::tl!("Keyboard"), crate::tl!("Quick Access")];
                for (page, label) in pages.iter().enumerate() {
                    let response = ui.add(egui::Button::selectable(state.page == page, *label));
                    if !state.focused && page == 0 {
                        response.request_focus();
                    }
                    if response.clicked() {
                        state.page = page;
                    }
                }
                state.focused = true;
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| match state.page {
                0 => ribbon_tab::show(app, ui, &mut state),
                1 => toolbars_tab::show(app, ui, &mut state),
                2 => keyboard_tab::show(app, ui, &mut state),
                _ => quick_access_tab::show(app, ui, &mut state),
            });
            if let Some(error) = &state.error {
                ui.colored_label(crate::theme::Tokens::get().warn, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(crate::tl!("Reset")).clicked() {
                    let _ = run(app, "ui.workspace.reset", json!({}), &mut state);
                    state.tab.clear();
                    state.group.clear();
                }
                if ui.button(crate::tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
    if close || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        *open = false;
    }
    app.customize = state;
}

fn commands(_app: &CadApp, ui: &mut egui::Ui, state: &mut State) {
    edit(ui, &mut state.query, crate::tl!("Find command"));
    let query = state.query.to_lowercase();
    let registry = cadcraft_engine::cmd::registry::command_registry();
    let mut commands: Vec<_> = registry
        .iter()
        .map(|command| (command.stable_id, command.label))
        .chain(crate::menus::UI_COMMANDS.iter().filter(|command| !command.1.is_empty()).map(|command| (command.0, command.1)))
        .filter(|(id, label)| id.contains(&query) || label.to_lowercase().contains(&query))
        .collect();
    commands.sort_by_key(|(id, _)| *id);
    commands.dedup_by_key(|(id, _)| *id);
    if state.command.is_empty()
        && let Some((id, _)) = commands.first()
    {
        state.command = (*id).into();
    }
    egui::ScrollArea::vertical().id_salt("command_list").max_height(110.0).show(ui, |ui| {
        for (id, label) in commands {
            ui.push_id(id, |ui| {
                if row(ui, state.command == id, &format!("{} ({id})", crate::i18n::t(label))) {
                    state.command = id.into();
                }
            });
        }
    });
}
