//! Dialogs: Drafting Settings, Drawing Units, About, command reference, blocks; dispatches the
//! Layer Properties Manager ([`crate::layers`]), Quick Select ([`crate::quick`]), Parameters
//! Manager ([`crate::parametric`]) and the style and settings managers ([`crate::managers`]).

use egui::{RichText, vec2};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    crate::mtext_editor::show(app, ctx);
    crate::quick::quick_properties(app, ctx);
    let Some(d) = app.ui.dialog.clone() else { return };
    let mut open = true;
    match d.as_str() {
        crate::palette::DIALOG => crate::palette::dialog(app, ctx, &mut open),
        crate::customize::DIALOG => crate::customize::dialog(app, ctx, &mut open),
        crate::ubplot::profile_dialog::DIALOG => crate::ubplot::profile_dialog::show(app, ctx, &mut open),
        crate::ubplot::DIALOG => crate::ubplot::dialog(app, ctx, &mut open),
        "spacemouse" => crate::spacemouse::dialog(app, ctx, &mut open),
        "language" => language(app, ctx, &mut open),
        "layers" => crate::layers::dialog(app, ctx, &mut open),
        "qselect" => crate::quick::qselect_dialog(app, ctx, &mut open),
        "parameters" => crate::parametric::parameters_dialog(app, ctx, &mut open),
        "dsettings" => dsettings(app, ctx, &mut open),
        "units" => units(app, ctx, &mut open),
        "hatch" => crate::hatchdlg::dialog(app, ctx, &mut open),
        "about" => about(ctx, &mut open),
        "preview" => crate::exchange::dialog(app, ctx, &mut open),
        "commands" => commands(app, ctx, &mut open),
        "blocks" => blocks(app, ctx, &mut open),
        "bedit" => crate::bedit::dialog(app, ctx, &mut open),
        "authoring" => crate::bedit::authoring_dialog(app, ctx, &mut open),
        "stylesmanager" => crate::plotstyles::dialog(app, ctx, &mut open),
        "fonts" => crate::fontmgr::dialog(app, ctx, &mut open),
        crate::wsprofile::import_preview::DIALOG => crate::wsprofile::import_preview::dialog(app, ctx, &mut open),
        crate::wsprofile::tab_conflict::DIALOG => crate::wsprofile::tab_conflict::dialog(app, ctx, &mut open),
        crate::wsprofile::picker::PICKER_DIALOG => crate::wsprofile::picker::dialog(app, ctx, &mut open),
        crate::wsprofile::picker::SAVE_AS_DIALOG => crate::wsprofile::picker::save_as_dialog(app, ctx, &mut open),
        o if o == "options" || o.starts_with("options:") => crate::options::dialog(app, ctx, o, &mut open),
        other => crate::managers::dialog(app, ctx, other, &mut open),
    }
    // A dialog that closed by opening another (Plot Preview back to Plot) keeps the new one.
    if !open && app.ui.dialog.as_deref() == Some(d.as_str()) {
        app.ui.dialog = None;
    }
}

fn dsettings(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let settings = app.session.drafting_settings();
    let (mut snapunit, mut gridunit) = (settings.snapunit, settings.gridunit);
    egui::Window::new(crate::tl!("Drafting Settings")).id(egui::Id::new("Drafting Settings")).open(open).resizable(false).show(ctx, |ui| {
        let s = &mut app.session.settings;
        ui.heading(crate::tl!("Snap and Grid"));
        ui.checkbox(&mut s.snapmode, crate::tl!("Snap On (F9)"));
        // SNAPSTYL: the isometric snap and grid (ISODRAFT).
        ui.horizontal(|ui| {
            ui.radio_value(&mut s.isodraft, false, crate::tl!("Rectangular snap"));
            ui.radio_value(&mut s.isodraft, true, crate::tl!("Isometric snap"));
        });
        ui.horizontal(|ui| {
            ui.label(crate::tl!("Snap X spacing"));
            ui.add(egui::DragValue::new(&mut snapunit.x).speed(0.05).range(0.0001..=1e6).clamp_existing_to_range(false));
            ui.label(crate::tl!("Y"));
            ui.add(egui::DragValue::new(&mut snapunit.y).speed(0.05).range(0.0001..=1e6).clamp_existing_to_range(false));
        });
        // SNAPTYPE and POLARDIST.
        ui.horizontal(|ui| {
            ui.label(crate::tl!("Snap type"));
            ui.radio_value(&mut s.polarsnap, false, crate::tl!("Grid snap"));
            ui.radio_value(&mut s.polarsnap, true, crate::tl!("PolarSnap"));
        });
        ui.add_enabled_ui(s.polarsnap, |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::tl!("Polar distance"));
                ui.add(egui::DragValue::new(&mut s.polardist).speed(0.05).range(0.0..=1e6));
            });
        });
        ui.checkbox(&mut s.gridmode, crate::tl!("Grid On (F7)"));
        ui.horizontal(|ui| {
            ui.label(crate::tl!("Grid spacing"));
            ui.add(egui::DragValue::new(&mut gridunit.x).speed(0.05).range(0.0..=1e6).clamp_existing_to_range(false));
            ui.label(crate::tl!("Major line every"));
            ui.add(egui::DragValue::new(&mut s.gridmajor).range(1..=100));
        });
        ui.separator();
        ui.heading(crate::tl!("Polar Tracking"));
        ui.checkbox(&mut s.polarmode, crate::tl!("Polar Tracking On (F10)"));
        let mut deg = s.polarang.to_degrees();
        ui.horizontal(|ui| {
            ui.label(crate::tl!("Increment angle"));
            egui::ComboBox::from_id_salt("polarang").selected_text(format!("{deg}")).show_ui(ui, |ui| {
                for a in [90.0, 45.0, 30.0, 22.5, 18.0, 15.0, 10.0, 5.0] {
                    ui.selectable_value(&mut deg, a, format!("{a}"));
                }
            });
        });
        s.polarang = deg.to_radians();
        tracking_options(ui, s);
        ui.separator();
        ui.heading(crate::tl!("Object Snap"));
        let mut on = s.osmode & cadcraft_engine::snap::mode::OFF == 0;
        if ui.checkbox(&mut on, crate::tl!("Object Snap On (F3)")).changed() {
            if on {
                s.osmode &= !cadcraft_engine::snap::mode::OFF;
            } else {
                s.osmode |= cadcraft_engine::snap::mode::OFF;
            }
        }
        egui::Grid::new("osnap_grid").num_columns(2).show(ui, |ui| {
            for (i, (bit, name)) in cadcraft_engine::snap::mode::ALL.iter().enumerate() {
                let mut v = s.osmode & bit != 0;
                if ui.checkbox(&mut v, crate::i18n::t(name)).changed() {
                    if v {
                        s.osmode |= bit;
                    } else {
                        s.osmode &= !bit;
                    }
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        ui.checkbox(&mut s.dynmode, crate::tl!("Enable Dynamic Input (F12)"));
        // Pointer input format for second and next points (DYNPIFORMAT, DYNPICOORDS).
        ui.indent("dynpi", |ui| {
            ui.add_enabled_ui(s.dynmode, |ui| {
                ui.label(crate::tl!("Second and next points:"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.dynpi_cartesian, false, crate::tl!("Polar (distance < angle)"));
                    ui.radio_value(&mut s.dynpi_cartesian, true, crate::tl!("Cartesian (x, y)"));
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.dynpi_absolute, false, crate::tl!("Relative"));
                    ui.radio_value(&mut s.dynpi_absolute, true, crate::tl!("Absolute"));
                });
                // DYNPROMPT: the prompt and its options (Down Arrow) beside the cursor.
                ui.checkbox(&mut s.dynprompt, crate::tl!("Show command prompting near the crosshairs"));
            });
        });
        ui.checkbox(&mut s.orthomode, crate::tl!("Ortho (F8)"));
    });
    // Drawing edits go through the same command as the CLI, including dirty state and undo.
    for (name, before, after) in [("SNAPUNIT", settings.snapunit, snapunit), ("GRIDUNIT", settings.gridunit, gridunit)] {
        if before != after {
            let _ = app.run("setvar", json!({ "name": name, "value": [after.x, after.y] }));
        }
    }
}

/// The polar and object snap tracking options of Drafting Settings: additional angles
/// (POLARMODE bit 4, POLARADDANG), tracking on every polar angle (bit 2) and angles relative to
/// the last segment (bit 1).
fn tracking_options(ui: &mut egui::Ui, s: &mut cadcraft_engine::Settings) {
    use cadcraft_engine::snap::tracking::polarmode;
    /// A radio button for POLARMODE `bit` being `on`.
    fn flag(ui: &mut egui::Ui, flags: &mut u32, bit: u32, on: bool, label: &str) {
        if ui.radio((*flags & bit != 0) == on, label).clicked() {
            *flags = if on { *flags | bit } else { *flags & !bit };
        }
    }
    let mut additional = s.polar_flags & polarmode::ADDITIONAL != 0;
    ui.horizontal(|ui| {
        if ui.checkbox(&mut additional, crate::tl!("Additional angles")).changed() {
            s.polar_flags ^= polarmode::ADDITIONAL;
        }
        // The list as typed (degrees separated by `;`), kept while it doesn't parse yet.
        let id = ui.id().with("polaraddang");
        let shown = cadcraft_engine::sysvars::format_angle_list(&s.polaraddang);
        let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
        let resp = ui.add_enabled(additional, egui::TextEdit::singleline(&mut text).desired_width(140.0));
        if resp.changed()
            && let Some(angles) = cadcraft_engine::sysvars::parse_angle_list(&text)
        {
            s.polaraddang = angles;
        }
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
    });
    ui.label(crate::tl!("Object Snap Tracking Settings"));
    ui.horizontal(|ui| {
        flag(ui, &mut s.polar_flags, polarmode::TRACK_POLAR, false, crate::tl!("Track orthogonally only"));
        flag(ui, &mut s.polar_flags, polarmode::TRACK_POLAR, true, crate::tl!("Track using all polar angle settings"));
    });
    ui.label(crate::tl!("Polar angle measurement"));
    ui.horizontal(|ui| {
        flag(ui, &mut s.polar_flags, polarmode::RELATIVE, false, crate::tl!("Absolute"));
        flag(ui, &mut s.polar_flags, polarmode::RELATIVE, true, crate::tl!("Relative to last segment"));
    });
}

/// LUNITS values in the order the Type list shows them.
pub(crate) const LENGTH_TYPES: [(i64, &str); 5] = [(4, "Architectural"), (2, "Decimal"), (3, "Engineering"), (5, "Fractional"), (1, "Scientific")];
/// AUNITS values.
pub(crate) const ANGLE_TYPES: [(i64, &str); 5] = [(0, "Decimal Degrees"), (1, "Deg/Min/Sec"), (2, "Grads"), (3, "Radians"), (4, "Surveyor's Units")];
/// INSUNITS names, indexed by value (DXF Reference, header group code 70).
pub(crate) const INSERTION_UNITS: [&str; 25] = [
    "Unitless",
    "Inches",
    "Feet",
    "Miles",
    "Millimeters",
    "Centimeters",
    "Meters",
    "Kilometers",
    "Microinches",
    "Mils",
    "Yards",
    "Angstroms",
    "Nanometers",
    "Microns",
    "Decimeters",
    "Decameters",
    "Hectometers",
    "Gigameters",
    "Astronomical Units",
    "Light Years",
    "Parsecs",
    "US Survey Feet",
    "US Survey Inch",
    "US Survey Yard",
    "US Survey Mile",
];

/// A combo box over `(value, label)` options.
fn pick(ui: &mut egui::Ui, salt: &str, value: &mut i64, options: &[(i64, String)]) {
    let current = options.iter().find(|o| o.0 == *value).map(|o| o.1.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt(salt).selected_text(current).width(180.0).show_ui(ui, |ui| {
        for (k, label) in options {
            ui.selectable_value(value, *k, label);
        }
    });
}

/// Drawing Units (UNITS): edits a copy of LUNITS/LUPREC/AUNITS/AUPREC/INSUNITS and applies it
/// through the `units` command on OK.
fn units(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    use cadcraft_engine::units::{format_angle, format_distance};
    let id = egui::Id::new("units_dialog_values");
    let mut v = ctx.data_mut(|d| d.get_temp::<[i64; 5]>(id)).unwrap_or_else(|| {
        app.session
            .doc()
            .map(|d| {
                let h = &d.header;
                [h.i64("LUNITS", 2), h.i64("LUPREC", 4), h.i64("AUNITS", 0), h.i64("AUPREC", 0), h.i64("INSUNITS", 1)]
            })
            .unwrap_or([2, 4, 0, 0, 1])
    });
    let (mut apply, mut cancel) = (false, false);
    egui::Window::new(crate::tl!("Drawing Units")).id(egui::Id::new("Drawing Units")).open(open).resizable(false).collapsible(false).show(
        ctx,
        |ui| {
            let [lu, lp, au, ap, ins] = &mut v;
            let types = |t: &[(i64, &str)]| t.iter().map(|(k, l)| (*k, crate::i18n::t(l).to_string())).collect::<Vec<_>>();
            egui::Grid::new("units_grid").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.strong(crate::tl!("Length"));
                ui.end_row();
                ui.label(crate::tl!("Type"));
                pick(ui, "units_lunits", lu, &types(&LENGTH_TYPES));
                ui.end_row();
                ui.label(crate::tl!("Precision"));
                pick(ui, "units_luprec", lp, &(0..=8).map(|p| (p, format_distance(1.5, *lu, p))).collect::<Vec<_>>());
                ui.end_row();
                ui.strong(crate::tl!("Angle"));
                ui.end_row();
                ui.label(crate::tl!("Type"));
                pick(ui, "units_aunits", au, &types(&ANGLE_TYPES));
                ui.end_row();
                ui.label(crate::tl!("Precision"));
                pick(ui, "units_auprec", ap, &(0..=8).map(|p| (p, format_angle(45f64.to_radians(), *au, p))).collect::<Vec<_>>());
                ui.end_row();
                ui.strong(crate::tl!("Insertion scale"));
                ui.end_row();
                ui.label(crate::tl!("Units to scale inserted content"));
                let names = INSERTION_UNITS.iter().enumerate().map(|(k, l)| (k as i64, crate::i18n::t(l).to_string())).collect::<Vec<_>>();
                pick(ui, "units_insunits", ins, &names);
                ui.end_row();
                ui.strong(crate::tl!("Sample output"));
                ui.end_row();
                ui.label("");
                ui.label(format!("{},{},{}", format_distance(1.5, *lu, *lp), format_distance(2.0039, *lu, *lp), format_distance(0.0, *lu, *lp)));
                ui.end_row();
                ui.label("");
                ui.label(format_angle(45f64.to_radians(), *au, *ap));
                ui.end_row();
            });
            ui.separator();
            ui.horizontal(|ui| {
                apply = ui.button(crate::tl!("OK")).clicked();
                cancel = ui.button(crate::tl!("Cancel")).clicked();
            });
        },
    );
    if apply {
        let [lu, lp, au, ap, ins] = v;
        let _ = app.run("units", json!({ "lunits": lu, "luprec": lp, "aunits": au, "auprec": ap, "insunits": ins }));
    }
    if apply || cancel || !*open {
        ctx.data_mut(|d| d.remove::<[i64; 5]>(id));
        *open = false;
    } else {
        ctx.data_mut(|d| d.insert_temp(id, v));
    }
}

fn about(ctx: &egui::Context, open: &mut bool) {
    let t = Tokens::get();
    let tab_id = egui::Id::new("about_tab");
    egui::Window::new(crate::tl!("About UbotCAD"))
        .id(egui::Id::new("About UbotCAD"))
        .open(open)
        .default_size(vec2(640.0, 420.0))
        .collapsible(false)
        .show(ctx, |ui| {
            let mut tab = ui.data_mut(|d| d.get_temp::<u8>(tab_id)).unwrap_or(0);
            ui.horizontal(|ui| {
                for (i, l) in [crate::tl!("About"), crate::tl!("Contributors"), crate::tl!("Models")].iter().enumerate() {
                    if ui.selectable_label(tab == i as u8, crate::i18n::t(l)).clicked() {
                        tab = i as u8;
                    }
                }
            });
            ui.data_mut(|d| d.insert_temp(tab_id, tab));
            ui.separator();
            match tab {
                1 => crate::credits::contributors_ui(ui),
                2 => crate::credits::models_ui(ui),
                _ => {
                    ui.heading(crate::tl!("UbotCAD"));
                    ui.label(crate::tf!("Version {version}", version = env!("CARGO_PKG_VERSION")));
                    ui.label(crate::tl!("Computer-aided design and drafting: an open-source, clean-room CAD application written in pure Rust."));
                    ui.add_space(6.0);
                    ui.label(RichText::new(crate::tl!("UbotCAD — based on CADCraft by the ArtCraft team.")).color(t.text_dim));
                    ui.hyperlink_to("ubotbuild.com", "https://ubotbuild.com/");
                    ui.add_space(6.0);
                    ui.label(RichText::new(crate::tl!("MIT OR Apache-2.0. Not affiliated with Autodesk, Inc.")).small().color(t.text_faint));
                }
            }
        });
}

fn commands(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut start = None;
    egui::Window::new(crate::tl!("Command Reference")).id(egui::Id::new("Command Reference")).open(open).default_size(vec2(640.0, 480.0)).show(
        ctx,
        |ui| {
            let id = ui.id().with("cmdfilter");
            let mut filter = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(crate::tl!("Filter"));
                ui.text_edit_singleline(&mut filter);
            });
            ui.data_mut(|d| d.insert_temp(id, filter.clone()));
            if ui.button(crate::tl!("Command Palette...")).clicked() {
                let _ = app.run("ui.palette", json!({"query": filter}));
            }
            let index = crate::palette::index::SearchIndex::build(app.language(), &crate::shortcuts::active(app));
            let hits = index.search(&filter, &app.session.recent_commands(8), &app.ws.effective.layout.favorites, 50);
            let entries: Vec<_> =
                if filter.trim().is_empty() { index.entries().collect() } else { hits.iter().filter_map(|hit| index.entry(hit)).collect() };
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("cmdref").striped(true).num_columns(3).show(ui, |ui| {
                    for entry in entries {
                        let available = entry.status == cadcraft_engine::cmd::registry::CommandStatus::Available;
                        let response = ui.add_enabled(available, egui::Link::new(entry.id.to_ascii_uppercase()));
                        if response.clicked() {
                            start = Some(entry.id);
                        }
                        ui.label(crate::i18n::t(entry.label_en));
                        ui.label(RichText::new(entry.aliases.join(", ").to_ascii_uppercase()).small());
                        if !available {
                            response.on_disabled_hover_text(crate::palette::unavailable_reason());
                        }
                        ui.end_row();
                    }
                });
            });
        },
    );
    if let Some(c) = start {
        app.ui.dialog = None;
        crate::menus::activate(app, c);
    }
}

fn blocks(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(crate::tl!("Blocks")).id(egui::Id::new("Blocks")).open(open).default_size(vec2(320.0, 360.0)).show(ctx, |ui| {
        blocks_body(ui, app);
    });
}

pub(crate) fn blocks_body(ui: &mut egui::Ui, app: &mut CadApp) {
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("blocks_body").auto_shrink([false, false]).show(ui, |ui| {
        let Ok(d) = app.session.doc() else { return };
        let mut names = d.blocks.keys().filter(|name| !name.starts_with('*')).peekable();
        if names.peek().is_none() {
            ui.label(crate::tl!("No blocks defined in this drawing."));
        }
        for name in names {
            if ui.button(name).on_hover_text(crate::tl!("Insert this block")).clicked() {
                pick = Some(name.clone());
            }
        }
    });
    if let Some(name) = pick {
        insert_block(app, &name);
    }
}

/// Start INSERT with `name` already given at its block name prompt, so the next prompt asks for
/// the insertion point.
fn insert_block(app: &mut CadApp, name: &str) {
    app.start("insert");
    if app.session.running.as_ref().is_none_or(|r| r.id != "insert") {
        return;
    }
    if let Err(e) = app.session.input(cadcraft_engine::Input::Text(name.to_string())) {
        app.session.echo(e.to_string());
    }
}

fn language(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(crate::tl!("Interface Language")).id(egui::Id::new("interface_language")).open(open).resizable(false).show(ctx, |ui| {
        egui::ComboBox::from_id_salt("interface_language_choice").selected_text(app.ui.interface_language.name()).show_ui(ui, |ui| {
            for preference in crate::i18n::Preference::ALL {
                if ui.selectable_label(app.ui.interface_language == preference, preference.name()).clicked() {
                    let _ = app.run("ui.language", serde_json::json!({"lang": preference.code()}));
                    ui.ctx().request_repaint();
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use cadcraft_engine::doc::EntityKind;
    use cadcraft_engine::geom::Vec2;
    use cadcraft_engine::{Input, Session};
    use serde_json::json;

    use crate::{CadApp, Services};

    #[test]
    fn picking_a_block_starts_insert_with_its_name() {
        let mut app = CadApp::new(Session::new(), Services::default());
        let c = app.session.execute("circle", &json!({"center": [0, 0], "radius": 1})).unwrap();
        let h = c["handle"].as_str().unwrap().to_string();
        app.session.execute("block", &json!({"name": "Door 2", "base": [0, 0], "handles": [h], "keep": "delete"})).unwrap();
        // A command already running is replaced, as when a command is typed.
        app.start("line");

        super::insert_block(&mut app, "Door 2");
        assert!(app.session.running.as_ref().is_some_and(|r| r.id == "insert"));
        assert_eq!(app.session.current_prompt().map(|p| p.message), Some("Specify insertion point".to_string()));

        app.session.input(Input::Point(Vec2::new(5.0, 6.0))).unwrap();
        // X scale, Y scale and rotation take their defaults.
        for _ in 0..3 {
            app.session.input(Input::Enter).unwrap();
        }
        assert!(app.session.running.is_none());
        let d = app.session.doc().unwrap();
        let mut ins = Vec::new();
        for e in d.model.iter() {
            if let EntityKind::Insert(i) = &e.kind {
                ins.push(i.clone());
            }
        }
        assert_eq!(ins.len(), 1);
        assert_eq!(ins[0].block, "Door 2");
        assert!((ins[0].insert.x - 5.0).abs() < 1e-9 && (ins[0].insert.y - 6.0).abs() < 1e-9);
    }

    #[test]
    fn picking_a_missing_block_keeps_asking_for_a_name() {
        let mut app = CadApp::new(Session::new(), Services::default());
        super::insert_block(&mut app, "Nope");
        assert!(app.session.running.as_ref().is_some_and(|r| r.id == "insert"));
        assert_eq!(app.session.current_prompt().map(|p| p.message), Some("Enter block name".to_string()));
    }
}
