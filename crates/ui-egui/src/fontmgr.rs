//! Tools ▸ Fonts...: the fonts the open drawing's text styles ask for, what is drawn where this
//! computer lacks one, and the font files the user adds. The table and FONTALT are the engine's
//! `fonts` command ([`cadcraft_engine::cmd::fonts`]); the added fonts live in
//! [`cadcraft_fonts::ttf`]. The host keeps both between runs: the table in the profile, the added
//! fonts through [`CadApp::take_font_changes`].
//!
//! The interface text is English only for now, so this file is not in the translated sources.

use std::collections::BTreeMap;

use cadcraft_fonts::BUILTIN_FONT;
use cadcraft_fonts::ttf::{self, FontOrigin};
use egui::RichText;
use serde_json::{Value, json};

use crate::CadApp;
use crate::theme::Tokens;

/// A change to the fonts the user added, for the host to keep between runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontChange {
    Added { file_name: String, bytes: Vec<u8> },
    Removed { stem: String },
}

/// Reads a font file chosen by the user: its name and contents. Set by a host with a file picker.
pub type PickFont = Box<dyn Fn() -> Option<(String, Vec<u8>)>>;

#[derive(Default)]
pub struct State {
    changes: Vec<FontChange>,
    /// The web host shows its own picker when this is set (see [`CadApp::take_font_pick_request`]).
    pick_wanted: bool,
    /// The desktop host's picker; `None` where the host has none.
    pub pick_file: Option<PickFont>,
    /// The browser can list the fonts installed on the computer (Chromium's `queryLocalFonts`);
    /// set by the web host.
    pub local_supported: bool,
    /// The user asked for the list; the host answers with [`CadApp::set_local_fonts`].
    local_wanted: bool,
    /// The installed fonts the browser listed, and the ones the user chose to add.
    local_fonts: Option<Vec<String>>,
    local_chosen: Vec<String>,
    /// What the last add or remove said.
    note: Option<String>,
    /// The report the dialog shows, and the revision and table it was made for.
    report: Option<(u64, usize, Value)>,
}

impl CadApp {
    /// Add a font file the user chose. Kept for the next session by the host
    /// ([`Self::take_font_changes`]).
    pub fn add_font(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<ttf::UserFont, String> {
        let font = ttf::add_user_font(file_name, bytes.clone())?;
        self.fontmgr.changes.push(FontChange::Added { file_name: file_name.to_string(), bytes });
        self.after_font_change(format!("Added font {}", font.families.first().unwrap_or(&font.stem)));
        Ok(font)
    }

    /// Add a font the host kept from an earlier run (nothing to keep again).
    pub fn restore_font(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<(), String> {
        ttf::add_user_font(file_name, bytes).map(|_| ())
    }

    /// Remove a font the user added.
    pub fn remove_font(&mut self, stem: &str) {
        if ttf::remove_user_font(stem) {
            self.fontmgr.changes.push(FontChange::Removed { stem: stem.to_string() });
            self.after_font_change(format!("Removed font {stem}"));
        }
    }

    /// The added or removed fonts the host has not kept yet.
    pub fn take_font_changes(&mut self) -> Vec<FontChange> {
        std::mem::take(&mut self.fontmgr.changes)
    }

    /// Whether the user asked to add a font file and the host has no picker of its own.
    pub fn take_font_pick_request(&mut self) -> bool {
        std::mem::take(&mut self.fontmgr.pick_wanted)
    }

    /// Whether the user asked to list the installed fonts (the web host asks the browser).
    pub fn take_local_fonts_request(&mut self) -> bool {
        std::mem::take(&mut self.fontmgr.local_wanted)
    }

    /// The names of the installed fonts the browser listed.
    pub fn set_local_fonts(&mut self, names: Vec<String>) {
        self.fontmgr.local_fonts = Some(names);
        self.fontmgr.note = None;
    }

    /// The installed fonts the user chose to add; the host reads each and calls [`Self::add_font`].
    pub fn take_local_font_choices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.fontmgr.local_chosen)
    }

    /// Say why a font file could not be added (the host's picker or its read failed).
    pub fn font_add_failed(&mut self, why: impl Into<String>) {
        let why = why.into();
        self.set_status(why.clone());
        self.fontmgr.note = Some(why);
    }

    fn request_font_file(&mut self) {
        let Some(pick) = self.fontmgr.pick_file.as_ref() else {
            self.fontmgr.pick_wanted = true;
            return;
        };
        if let Some((name, bytes)) = pick()
            && let Err(e) = self.add_font(&name, bytes)
        {
            self.font_add_failed(e);
        }
    }

    fn after_font_change(&mut self, note: String) {
        self.set_status(note.clone());
        self.fontmgr.note = Some(note);
        self.fontmgr.report = None;
        self.session.touch();
    }

    /// After a drawing opens: say which of its fonts this computer lacks (the command line and the
    /// status bar), so a user knows why text looks different and where to change it.
    pub fn notify_missing_fonts(&mut self) {
        let report = cadcraft_engine::cmd::fonts::report(&self.session);
        let missing: Vec<&str> = report["styles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["used"] == true && s["state"] == "missing" && s["mapped"] != true)
            .filter_map(|s| s["font"].as_str())
            .collect();
        if missing.is_empty() {
            return;
        }
        let mut names: Vec<&str> = Vec::new();
        for m in missing {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(m)) {
                names.push(m);
            }
        }
        let shown = names.iter().take(4).copied().collect::<Vec<_>>().join(", ");
        let more = if names.len() > 4 { format!(" and {} more", names.len() - 4) } else { String::new() };
        let text = format!(
            "{} font{} missing: {shown}{more}. Tools > Fonts... lets you choose what to draw instead.",
            names.len(),
            if names.len() == 1 { "" } else { "s" }
        );
        self.session.echo(text.clone());
        self.set_status(text);
    }
}

/// FONTS typed or chosen from the menu opens the dialog; with parameters it is the engine command.
pub fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !params.is_null() || cadcraft_engine::cmd::resolve_alias(id) != "fonts" {
        return None;
    }
    app.ui.dialog = Some("fonts".into());
    Some(Ok(Value::Null))
}

/// One font a drawing asks for, with the styles that use it.
struct Row {
    font: String,
    styles: Vec<String>,
    used: bool,
    state: String,
    with: Option<String>,
    mapped: bool,
    legacy: Option<String>,
}

fn rows(report: &Value) -> Vec<Row> {
    let mut by_font: BTreeMap<String, Row> = BTreeMap::new();
    for s in report["styles"].as_array().into_iter().flatten() {
        let font = s["font"].as_str().unwrap_or_default().to_string();
        let row = by_font.entry(font.to_ascii_lowercase()).or_insert_with(|| Row {
            font: font.clone(),
            styles: Vec::new(),
            used: false,
            state: s["state"].as_str().unwrap_or("stroke").to_string(),
            with: s["with"].as_str().map(str::to_string),
            mapped: s["mapped"] == true,
            legacy: s["legacy"].as_str().map(str::to_string),
        });
        row.styles.push(s["style"].as_str().unwrap_or_default().to_string());
        row.used |= s["used"] == true;
    }
    let mut rows: Vec<Row> = by_font.into_values().collect();
    // What needs a decision first: missing fonts in use, then the rest in use, then unused styles.
    rows.sort_by_key(|r| (!r.used, r.state != "missing"));
    rows
}

/// What a click in the dialog asks for.
enum Act {
    Map(String, String),
    Alt(String),
    Add,
    Remove(String),
    ListLocal,
    AddLocal(String),
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let revision = app.session.state().map_or(0, |st| st.revision);
    let table = cadcraft_fonts::font_map().len();
    let stale = !matches!(&app.fontmgr.report, Some((r, t, _)) if *r == revision && *t == table);
    if stale {
        app.fontmgr.report = Some((revision, table, cadcraft_engine::cmd::fonts::report(&app.session)));
    }
    let report = app.fontmgr.report.as_ref().map(|r| r.2.clone()).unwrap_or(Value::Null);
    let rows = rows(&report);
    let alt = report["alt"].as_str().unwrap_or_default().to_string();
    let added = ttf::user_fonts();
    let t = Tokens::get();
    let mut act = None;
    let note = app.fontmgr.note.clone();
    let local_supported = app.fontmgr.local_supported;
    let local = app.fontmgr.local_fonts.clone();
    let can_pick = app.fontmgr.pick_file.is_some() || cfg!(target_arch = "wasm32");
    egui::Window::new("Fonts")
        .id(egui::Id::new("cc_fonts"))
        .open(open)
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.label(RichText::new("Fonts stay on this computer. They are not uploaded and never change the drawing.").color(t.text_dim));
            ui.add_space(6.0);
            ui.heading("In this drawing");
            if rows.is_empty() {
                ui.label(RichText::new("No drawing is open.").color(t.text_dim));
            } else {
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).id_salt("fonts-rows").show(ui, |ui| {
                    egui::Grid::new("fonts-grid").num_columns(4).striped(true).spacing([12.0, 6.0]).show(ui, |ui| {
                        for h in ["Font", "Used by", "Status", "Draw with"] {
                            ui.label(RichText::new(h).color(t.text_dim));
                        }
                        ui.end_row();
                        for r in &rows {
                            font_row(ui, r, &mut act);
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.heading("Any other missing font");
            ui.horizontal(|ui| {
                ui.label("Draw with");
                let shown = if alt.is_empty() { "Built-in stroke font".to_string() } else { alt.clone() };
                egui::ComboBox::from_id_salt("fonts-alt").selected_text(shown).width(260.0).show_ui(ui, |ui| {
                    if ui.selectable_label(alt.is_empty(), "Built-in stroke font").clicked() {
                        act = Some(Act::Alt(String::new()));
                    }
                    choices(ui, |ui, label, name| {
                        if ui.selectable_label(alt.eq_ignore_ascii_case(name), label).clicked() {
                            act = Some(Act::Alt(name.to_string()));
                        }
                    });
                });
            });
            ui.add_space(8.0);
            ui.heading("Fonts you added");
            if added.is_empty() {
                ui.label(RichText::new("None yet. Add the font files a drawing needs (.ttf, .otf, .ttc, .shx).").color(t.text_dim));
            }
            for f in &added {
                ui.horizontal(|ui| {
                    ui.label(f.families.first().unwrap_or(&f.stem));
                    ui.label(RichText::new(format!("{} · {} KB", f.stem, f.bytes.div_ceil(1024))).color(t.text_dim));
                    if ui.small_button("Remove").clicked() {
                        act = Some(Act::Remove(f.stem.clone()));
                    }
                });
            }
            if let Some(list) = &local {
                ui.add_space(4.0);
                ui.label(RichText::new("Fonts installed on this computer (click to add)").color(t.text_dim));
                local_list(ui, list, &mut act);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(can_pick, egui::Button::new("Add font file...")).clicked() {
                    act = Some(Act::Add);
                }
                if local_supported && ui.button("Add installed font...").clicked() {
                    act = Some(Act::ListLocal);
                }
                if let Some(n) = &note {
                    ui.label(RichText::new(n).color(t.text_dim));
                }
            });
        });
    match act {
        Some(Act::Map(font, to)) => apply(app, json!({ "map": { font: to } })),
        Some(Act::Alt(name)) => apply(app, json!({ "alt": name })),
        Some(Act::Add) => app.request_font_file(),
        Some(Act::Remove(stem)) => app.remove_font(&stem),
        Some(Act::ListLocal) => app.fontmgr.local_wanted = true,
        Some(Act::AddLocal(name)) => app.fontmgr.local_chosen.push(name),
        None => {}
    }
}

fn apply(app: &mut CadApp, params: Value) {
    if let Err(e) = app.run("fonts", params) {
        app.set_status(e);
    }
    app.fontmgr.report = None;
}

fn font_row(ui: &mut egui::Ui, r: &Row, act: &mut Option<Act>) {
    let t = Tokens::get();
    let dim = |text: String| if r.used { RichText::new(text) } else { RichText::new(text).color(t.text_dim) };
    let name = if r.font.is_empty() { BUILTIN_FONT } else { r.font.as_str() };
    ui.label(dim(name.to_string()));
    let styles = r.styles.join(", ");
    let shown = if r.used { styles.clone() } else { format!("{styles} (unused)") };
    ui.label(dim(shown)).on_hover_text(styles);
    let (text, color) = match r.state.as_str() {
        "found" => ("Found", t.text),
        "stroke" => ("Stroke font", t.text),
        _ => ("Missing", t.warn),
    };
    let status = ui.label(RichText::new(text).color(color));
    if let Some(l) = &r.legacy {
        status.on_hover_text(format!("Text typed for this {l} font is shown as Unicode."));
    }
    if r.state == "found" {
        ui.label("");
    } else {
        let shown = match (&r.with, r.mapped) {
            (Some(w), _) => w.clone(),
            (None, true) => "Built-in stroke font".to_string(),
            (None, false) => "Default".to_string(),
        };
        egui::ComboBox::from_id_salt(("fonts-with", &r.font)).selected_text(shown).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(!r.mapped, "Default").clicked() {
                *act = Some(Act::Map(r.font.clone(), String::new()));
            }
            if ui.selectable_label(r.mapped && r.with.is_none(), "Built-in stroke font").clicked() {
                *act = Some(Act::Map(r.font.clone(), BUILTIN_FONT.to_string()));
            }
            choices(ui, |ui, label, name| {
                if ui.selectable_label(r.mapped && r.with.as_deref() == Some(name), label).clicked() {
                    *act = Some(Act::Map(r.font.clone(), name.to_string()));
                }
            });
        });
    }
    ui.end_row();
}

/// The installed fonts the browser listed, narrowed by what the user types; a click adds one.
fn local_list(ui: &mut egui::Ui, list: &[String], act: &mut Option<Act>) {
    let id = ui.id().with("fonts-local-filter");
    let mut filter = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter").desired_width(f32::INFINITY));
    let needle = filter.trim().to_lowercase();
    ui.data_mut(|d| d.insert_temp(id, filter));
    egui::ScrollArea::vertical().max_height(160.0).id_salt("fonts-local").show(ui, |ui| {
        for name in list.iter().filter(|n| needle.is_empty() || n.to_lowercase().contains(&needle)) {
            if ui.selectable_label(false, name).clicked() {
                *act = Some(Act::AddLocal(name.clone()));
            }
        }
    });
}

/// The fonts to pick from, those added first, narrowed by what the user types. `item` gets the
/// label to show and the name to store (the name finds the font again).
fn choices(ui: &mut egui::Ui, mut item: impl FnMut(&mut egui::Ui, &str, &str)) {
    let id = ui.id().with("fonts-filter");
    let mut filter = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter").desired_width(f32::INFINITY));
    let needle = filter.trim().to_lowercase();
    ui.data_mut(|d| d.insert_temp(id, filter));
    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
        for c in ttf::font_choices() {
            if needle.is_empty() || c.name.to_lowercase().contains(&needle) {
                let label = if c.origin == FontOrigin::Added { format!("{} (added)", c.name) } else { c.name.clone() };
                item(ui, &label, &c.name);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_grouped_with_missing_ones_in_use_first() {
        let report = json!({"styles": [
            {"style": "A", "font": "arial.ttf", "used": true, "state": "found"},
            {"style": "B", "font": "ROMANS.SHX", "used": false, "state": "stroke"},
            {"style": "C", "font": "vn.ttf", "used": true, "state": "missing", "legacy": "TCVN3"},
            {"style": "D", "font": "Arial.ttf", "used": true, "state": "found"},
        ]});
        let r = rows(&report);
        assert_eq!(r.iter().map(|r| r.font.as_str()).collect::<Vec<_>>(), ["vn.ttf", "arial.ttf", "ROMANS.SHX"]);
        assert_eq!(r[1].styles, ["A", "D"]);
        assert_eq!(r[0].legacy.as_deref(), Some("TCVN3"));
    }
}
