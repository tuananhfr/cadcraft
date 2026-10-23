use std::collections::BTreeMap;

use cadcraft_engine::sheetplan::profile::{ProfileLevel, check_change};
use serde_json::{Value, json};

use crate::CadApp;

pub(crate) const DIALOG: &str = "ubplot_profile";

#[derive(Clone, Default)]
struct State {
    drawing: bool,
    version: String,
    values: BTreeMap<String, String>,
    error: Option<String>,
}

pub(crate) fn show(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let effective = app.session.effective_print_profile();
    let id = egui::Id::new(DIALOG);
    let mut state = ctx.data_mut(|data| data.remove_temp::<State>(id)).unwrap_or_default();
    if state.version != effective.version {
        state.values.clear();
        state.version = effective.version.clone();
    }
    let settings = match serde_json::to_value(&effective.settings) {
        Ok(Value::Object(settings)) => settings,
        _ => return,
    };
    let mut edit = None;
    let max_width = (ctx.content_rect().width() - 32.0).max(0.0);
    egui::Window::new(crate::tl!("Print Profile")).id(id).open(open).default_width(720.0_f32.min(max_width)).max_width(max_width).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(crate::tl!("Edit level"));
            ui.radio_value(&mut state.drawing, false, crate::tl!("User"));
            ui.radio_value(&mut state.drawing, true, crate::tl!("Drawing"));
        });
        egui::ScrollArea::both().auto_shrink([false, true]).max_height((ctx.content_rect().height() - 180.0).max(100.0)).show(ui, |ui| {
            let text_width = |text: &str| {
                ui.painter().layout_no_wrap(text.to_owned(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color()).size().x
            };
            let spacing = ui.spacing().item_spacing.x;
            let mut field_width = text_width(crate::tl!("Field")).max(ui.spacing().interact_size.x);
            let mut origin_width = text_width(crate::tl!("From level")).max(ui.spacing().interact_size.x);
            for field in settings.keys() {
                let pointer = format!("/{field}");
                field_width = field_width.max(text_width(field_label(field)));
                let level_width = text_width(level_label(effective.origin.get(&pointer).copied().unwrap_or(ProfileLevel::Platform)));
                let action_width = if check_change(&effective, &pointer).is_err() {
                    18.0 + spacing + text_width(crate::tl!("Locked by company policy"))
                } else {
                    (text_width(crate::tl!("Apply")) + 2.0 * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x)
                };
                origin_width = origin_width.max(level_width + spacing + action_width);
            }
            let value_width = (ui.available_width() - field_width - origin_width - 2.0 * spacing).max(220.0);
            egui::Grid::new("print_profile_fields").num_columns(3).striped(true).show(ui, |ui| {
                ui.strong(crate::tl!("Field"));
                ui.strong(crate::tl!("Value"));
                ui.strong(crate::tl!("From level"));
                ui.end_row();
                for (field, value) in settings {
                    let pointer = format!("/{field}");
                    let locked = check_change(&effective, &pointer).is_err();
                    let label = field_label(&field);
                    let field_label = ui.label(label);
                    let optional_text = matches!(field.as_str(), "printer" | "paper" | "plot_scale");
                    let text = state.values.entry(field.clone()).or_insert_with(|| match &value {
                        Value::Null if optional_text => String::new(),
                        Value::String(text) => text.clone(),
                        _ => value.to_string(),
                    });
                    // Grid cells inherit their header width; a minimum prevents TextEdit from clamping to that narrow measurement.
                    let editor = egui::TextEdit::singleline(&mut *text).desired_width(value_width).min_size(egui::vec2(value_width, 0.0));
                    let response = ui.add_enabled(!locked, editor).labelled_by(field_label.id);
                    if locked {
                        response.on_disabled_hover_text(crate::tl!("Locked by company policy"));
                    }
                    ui.horizontal(|ui| {
                        ui.label(level_label(effective.origin.get(&pointer).copied().unwrap_or(ProfileLevel::Platform)));
                        if locked {
                            let _ = crate::icons::button(ui, crate::icons::Icon::Lock, 18.0, crate::tl!("Locked by company policy"), false);
                            ui.label(crate::tl!("Locked by company policy"));
                        } else if ui.button(crate::tl!("Apply")).clicked() {
                            let parsed = if optional_text && text.is_empty() {
                                Ok(Value::Null)
                            } else if value.is_string() || optional_text {
                                Ok(Value::String(text.clone()))
                            } else {
                                serde_json::from_str::<Value>(text)
                            };
                            match parsed {
                                Ok(value) => edit = Some((pointer, value)),
                                Err(error) => state.error = Some(error.to_string()),
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
        if let Some(error) = &state.error {
            ui.colored_label(crate::theme::Tokens::get().warn, error);
        }
    });
    if let Some((key, value)) = edit {
        let level = if state.drawing { "drawing" } else { "user" };
        state.error = app.run("ubplot.profile", json!({"action": "set", "level": level, "key": key, "value": value})).err();
    }
    if *open {
        ctx.data_mut(|data| data.insert_temp(id, state));
    }
}

fn level_label(level: ProfileLevel) -> &'static str {
    match level {
        ProfileLevel::Platform => crate::tl!("Platform"),
        ProfileLevel::Company => crate::tl!("Company"),
        ProfileLevel::Project => crate::tl!("Project"),
        ProfileLevel::SheetSet => crate::tl!("Sheet Set"),
        ProfileLevel::Drawing => crate::tl!("Drawing"),
        ProfileLevel::User => crate::tl!("User"),
    }
}

fn field_label(field: &str) -> &'static str {
    match field {
        "device" => crate::tl!("Device"),
        "printer" => crate::tl!("Printer"),
        "plot_style_table" => crate::tl!("Plot style table"),
        "paper" => crate::tl!("Paper"),
        "landscape" => crate::tl!("Landscape"),
        "plot_scale" => crate::tl!("Scale"),
        "order" => crate::tl!("Sheet order"),
        "frame_templates" => crate::tl!("Frame templates"),
        "naming" => crate::tl!("Naming"),
        "output_mode" => crate::tl!("Output mode"),
        "pages_per_file" => crate::tl!("Pages per file"),
        "pdf_layers" => crate::tl!("PDF layers"),
        "text_as_geometry" => crate::tl!("Text as geometry"),
        "lineweights" => crate::tl!("Lineweights"),
        "retry_limit" => crate::tl!("Retry limit"),
        _ => crate::tl!("Field"),
    }
}
