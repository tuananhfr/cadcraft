//! Keyboard-operated command palette using the shared command search index.

use std::collections::BTreeMap;

use cadcraft_engine::cmd::registry::CommandStatus;
use egui::{Key, vec2};
use serde_json::{Value, json};

use crate::CadApp;

pub mod index;

pub const DIALOG: &str = "command_palette";

#[derive(Default)]
pub struct State {
    pub query: String,
    selected: usize,
    focused: bool,
    ime_composing: bool,
    index: Option<index::SearchIndex>,
    language: Option<&'static str>,
    shortcuts: BTreeMap<String, String>,
}

pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "ui.palette" {
        return None;
    }
    let query = params.get("query").and_then(Value::as_str).unwrap_or("").chars().take(64).collect();
    app.palette = State { query, ..State::default() };
    app.ui.dialog = Some(DIALOG.into());
    Some(Ok(json!({"dialog": DIALOG})))
}

pub(crate) fn unavailable_reason() -> &'static str {
    crate::tl!("This command is not available on this platform.")
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut state = std::mem::take(&mut app.palette);
    let shortcuts = crate::shortcuts::active(app);
    if state.language != Some(app.language()) || state.shortcuts != shortcuts || state.index.is_none() {
        state.index = Some(index::SearchIndex::build(app.language(), &shortcuts));
        state.language = Some(app.language());
        state.shortcuts = shortcuts;
    }
    let composing = state.ime_composing;
    // A commit and Enter can arrive in one frame; neither may activate the selected command.
    let ime_frame = ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Ime(_))));
    state.ime_composing = ctx.text_edit_focused()
        && ctx.input(|input| {
            input.events.iter().fold(composing, |active, event| match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => !text.is_empty(),
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => false,
                _ => active,
            })
        });
    let mut activate = None;
    let mut close = false;
    egui::Window::new(crate::tl!("Command Palette..."))
        .id(egui::Id::new(DIALOG))
        .open(open)
        .default_width(680.0)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let label = crate::tl!("Search commands...");
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id_salt("palette_query")
                    .hint_text(label)
                    .char_limit(64)
                    .desired_width(f32::INFINITY)
                    .return_key(None)
                    .event_filter(egui::EventFilter { vertical_arrows: true, escape: true, ..Default::default() }),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
            if !state.focused {
                response.request_focus();
                state.focused = true;
            }
            if response.changed() {
                state.selected = 0;
            }
            let Some(index) = &state.index else { return };
            let recent = app.session.recent_commands(8);
            let hits = index.search(&state.query, &recent, &app.ws.effective.layout.favorites, 50);
            let safe_keys = !composing && !state.ime_composing && !ime_frame;
            if safe_keys {
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::ArrowDown)) {
                    state.selected = state.selected.saturating_add(1);
                }
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::ArrowUp)) {
                    state.selected = state.selected.saturating_sub(1);
                }
            }
            state.selected = state.selected.min(hits.len().saturating_sub(1));
            let empty_query = state.query.trim().is_empty();
            let mut recent_group = None;
            egui::ScrollArea::vertical().max_height(12.0 * 30.0).show(ui, |ui| {
                for (row, hit) in hits.iter().enumerate() {
                    let Some(entry) = index.entry(hit) else { continue };
                    if empty_query {
                        let is_recent = recent.iter().any(|id| id == entry.id);
                        if recent_group != Some(is_recent) {
                            ui.strong(if is_recent { crate::tl!("Recent") } else { crate::tl!("Commands") });
                            recent_group = Some(is_recent);
                        }
                    }
                    let available = entry.status == CommandStatus::Available;
                    ui.push_id(entry.id, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::hover());
                            if let Some(icon) = entry.icon {
                                crate::icons::paint(ui.painter(), rect, icon, !available);
                            }
                            let aliases = entry.aliases.iter().take(8).map(String::as_str).collect::<Vec<_>>().join(", ");
                            let label = format!("{}  {}  {aliases}", crate::i18n::t(entry.label_en), entry.id.to_ascii_uppercase());
                            let response = ui.add_enabled(available, egui::Button::selectable(row == state.selected, label).truncate());
                            if row == state.selected {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() && safe_keys {
                                activate = Some(entry.id);
                            }
                            if let Some(shortcut) = &entry.shortcut {
                                ui.label(crate::menus::shortcut_label(shortcut, ctx.os().is_mac()));
                            }
                        });
                        if !available {
                            ui.add_enabled(false, egui::Label::new(unavailable_reason()));
                        }
                    });
                }
            });
            if hits.is_empty() {
                ui.label(crate::tl!("No matching commands."));
            }
            if safe_keys && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                activate = hits
                    .get(state.selected)
                    .and_then(|hit| index.entry(hit))
                    .filter(|entry| entry.status == CommandStatus::Available)
                    .map(|entry| entry.id);
            }
            if safe_keys && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                close = true;
            }
        });
    app.palette = state;
    if close || !*open {
        *open = false;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    } else if let Some(id) = activate {
        *open = false;
        app.ui.dialog = None;
        crate::menus::activate(app, id);
    }
}
