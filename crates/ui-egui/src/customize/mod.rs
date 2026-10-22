//! Keyboard-operable workspace customization; all mutations go through UI commands.

use egui::{Key, vec2};
use serde_json::{Value, json};

use crate::CadApp;

pub mod alias_tab;
mod keyboard_tab;
mod quick_access_tab;
mod ribbon_tab;
mod toolbars_tab;

pub const DIALOG: &str = "customize_workspace";

#[derive(Default)]
pub struct State {
    page: usize,
    list_height: f32,
    scroll_ribbon_page: bool,
    aliases: alias_tab::State,
    tab: String,
    group: String,
    ribbon_index: Option<usize>,
    ribbon_focus: Option<usize>,
    ribbon_drag_active: bool,
    toolbar: String,
    command: String,
    query: String,
    commands: Vec<(&'static str, &'static str)>,
    label: String,
    key: String,
    replace: bool,
    error: Option<String>,
    focus_label: bool,
    focused: bool,
    ime_composing: bool,
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
        egui::TextEdit::singleline(text)
            .id_salt(label)
            .hint_text(label)
            .char_limit(1024)
            .desired_width(ui.available_width().min(240.0))
            .event_filter(egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() }),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    response
}

fn row(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    let response = ui.add(egui::Button::selectable(selected, label).truncate());
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response.clicked()
}

fn list<R>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    height: Option<f32>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if let Some(height) = height {
        egui::ScrollArea::vertical()
            .id_salt(id_salt)
            .min_scrolled_height(height)
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, add_contents)
            .inner
    } else {
        ui.push_id(id_salt, add_contents).inner
    }
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut state = std::mem::take(&mut app.customize);
    let mut close = false;
    let editing = ctx.text_edit_focused();
    // egui keeps composition state private; preedit lasts until commit or focus loss.
    state.ime_composing = editing
        && ctx.input(|input| {
            input.events.iter().fold(state.ime_composing, |composing, event| match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => !text.is_empty(),
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => false,
                _ => composing,
            })
        });
    let screen = ctx.content_rect();
    egui::Window::new(crate::tl!("Customize Workspace..."))
        .id(egui::Id::new(DIALOG))
        .open(open)
        .fixed_size(vec2((screen.width() - 48.0).clamp(240.0, 1100.0), (screen.height() - 80.0).clamp(120.0, 660.0)))
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let pages = [crate::tl!("Ribbon"), crate::tl!("Toolbars"), crate::tl!("Keyboard"), crate::tl!("Quick Access"), crate::tl!("Aliases")];
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
            let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            let body = (ui.available_height() - row - 16.0 - if state.error.is_some() { row * 2.0 } else { 0.0 }).max(1.0);
            state.list_height = (body - row * 6.0).max(row * 5.0);
            // An inner list consumes scroll-to requests before the page can reveal it on short screens.
            state.scroll_ribbon_page = body < row * 11.0;
            egui::ScrollArea::vertical().id_salt("customize_page").auto_shrink([false, false]).min_scrolled_height(body).max_height(body).show(
                ui,
                |ui| match state.page {
                    0 => ribbon_tab::show(app, ui, &mut state),
                    1 => toolbars_tab::show(app, ui, &mut state),
                    2 => keyboard_tab::show(app, ui, &mut state),
                    3 => quick_access_tab::show(app, ui, &mut state),
                    _ => alias_tab::show(app, ui, &mut state),
                },
            );
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
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        if editing || ctx.text_edit_focused() {
            if !state.ime_composing
                && let Some(id) = ctx.memory(|memory| memory.focused())
            {
                ctx.memory_mut(|memory| memory.surrender_focus(id));
            }
        } else {
            close = true;
        }
    }
    if close {
        *open = false;
    }
    app.customize = state;
}

fn commands(app: &CadApp, ui: &mut egui::Ui, state: &mut State) {
    edit(ui, &mut state.query, crate::tl!("Find command"));
    let index = crate::palette::index::SearchIndex::build(app.language(), &crate::shortcuts::active(app));
    state.commands = if state.query.trim().is_empty() {
        let mut entries: Vec<_> = index.entries().map(|entry| (entry.id, entry.label_en)).collect();
        entries.sort_by_key(|(id, _)| *id);
        entries
    } else {
        index
            .search(&state.query, &app.session.recent_commands(8), &app.ws.effective.layout.favorites, 50)
            .iter()
            .filter_map(|hit| index.entry(hit).map(|entry| (entry.id, entry.label_en)))
            .collect()
    };
    if state.command.is_empty()
        && let Some((id, _)) = state.commands.first()
    {
        state.command = (*id).into();
    }
    let height = (!(state.page == 0 && state.scroll_ribbon_page)).then_some(state.list_height);
    list(ui, "command_list", height, |ui| {
        for &(id, label) in &state.commands {
            ui.push_id(id, |ui| {
                if row(ui, state.command == id, &format!("{} ({id})", crate::i18n::t(label))) {
                    state.command = id.into();
                }
            });
        }
    });
}
