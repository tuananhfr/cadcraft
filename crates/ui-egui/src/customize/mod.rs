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
    command_filter: Option<String>,
    commands: Vec<(&'static str, &'static str)>,
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

pub(crate) fn route(app: &mut CadApp, command: &str, _params: &Value) -> Option<Result<Value, String>> {
    if command != "ui.customize.open" {
        return None;
    }
    app.customize = State::default();
    app.ui.dialog = Some(DIALOG.into());
    Some(Ok(json!({"dialog": DIALOG})))
}

fn edit(ui: &mut egui::Ui, text: &mut String, label: &str) -> egui::Response {
    ui.label(label);
    // Keep Escape with the editor so egui cannot clear IME focus before the dialog handles input.
    let response = ui.add(
        egui::TextEdit::singleline(text).id_salt(label).hint_text(label).char_limit(1024).desired_width(240.0).event_filter(egui::EventFilter {
            horizontal_arrows: true,
            vertical_arrows: true,
            escape: true,
            ..Default::default()
        }),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    response
}

fn row(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    let response = ui.add(egui::Button::selectable(selected, label));
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response.clicked()
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut state = std::mem::take(&mut app.customize);
    let mut close = false;
    let editing = ctx.text_edit_focused();
    egui::Window::new(crate::tl!("Customize Workspace..."))
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
    if close || (!editing && !ctx.text_edit_focused() && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))) {
        *open = false;
    }
    app.customize = state;
}

fn commands(_app: &CadApp, ui: &mut egui::Ui, state: &mut State) {
    edit(ui, &mut state.query, crate::tl!("Find command"));
    if state.command_filter.as_deref() != Some(state.query.as_str()) {
        let query = state.query.to_lowercase();
        state.commands = cadcraft_engine::cmd::registry::command_registry()
            .iter()
            .map(|command| (command.stable_id, command.label))
            .chain(crate::menus::UI_COMMANDS.iter().filter(|command| !command.1.is_empty()).map(|command| (command.0, command.1)))
            .filter(|(id, label)| id.contains(&query) || label.to_lowercase().contains(&query))
            .collect();
        state.commands.sort_by_key(|(id, _)| *id);
        state.commands.dedup_by_key(|(id, _)| *id);
        state.command_filter = Some(state.query.clone());
    }
    if state.command.is_empty()
        && let Some((id, _)) = state.commands.first()
    {
        state.command = (*id).into();
    }
    egui::ScrollArea::vertical().id_salt("command_list").max_height(110.0).show_rows(
        ui,
        ui.spacing().interact_size.y,
        state.commands.len(),
        |ui, range| {
            for &(id, label) in state.commands.get(range).unwrap_or_default() {
                ui.push_id(id, |ui| {
                    if row(ui, state.command == id, &format!("{} ({id})", crate::i18n::t(label))) {
                        state.command = id.into();
                    }
                });
            }
        },
    );
}
