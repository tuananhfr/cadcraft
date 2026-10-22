use std::collections::BTreeSet;

use super::*;
use crate::{CadApp, Services};
use cadcraft_engine::Session;
use serde_json::json;

fn placeholders(text: &str) -> BTreeSet<&str> {
    text.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(key, _)| key)).collect()
}

#[test]
fn catalogs_are_nfc() {
    for language in &LANGUAGES {
        assert!(unicode_normalization::is_nfc(language.source), "{} catalog is not NFC", language.code);
    }
    assert!(unicode_normalization::is_nfc(include_str!("vi_glossary.tsv")), "Vietnamese glossary is not NFC");
}

#[test]
fn catalog_is_valid_and_preserves_placeholders_and_ellipses() {
    for language in &LANGUAGES {
        let (catalog, errors) = parse(language.source);
        assert!(errors.is_empty(), "{}: {errors:?}", language.code);
        for ((context, key), value) in catalog {
            assert!(["", "menu", "ribbon", "@plural"].contains(&context), "unknown context: {context}");
            let forms: Vec<_> = if context == "@plural" { value.split('|').collect() } else { vec![value] };
            if context == "@plural" {
                assert_eq!(forms.len(), language.plural_forms, "{}: {key}", language.code);
            }
            for form in forms {
                assert_eq!(placeholders(key), placeholders(form), "{key}");
                for suffix in ["...", "…"] {
                    assert_eq!(key.ends_with(suffix), form.ends_with(suffix), "{key}");
                }
            }
        }
    }
    let (parsed, errors) = parse("broken\n\tOpen\tВідкрити\n\tOpen\tВідкрити\n\tEmpty\t\n");
    assert_eq!(parsed.len(), 1);
    assert_eq!(errors.len(), 3);
    assert_eq!(tr("uk", "a future untranslated label"), "a future untranslated label");
    assert_eq!(tr("unknown", "Open"), "Open");
}

/// Rows of the Vietnamese glossary: (context, English, Vietnamese); the note column is for reviewers.
fn glossary() -> Vec<(&'static str, &'static str, &'static str)> {
    let rows = include_str!("vi_glossary.tsv").lines().map(|line| line.trim_end_matches('\r'));
    rows.filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let cols: Vec<_> = line.split('\t').collect();
            assert_eq!(cols.len(), 4, "glossary rows need context, English, Vietnamese and a note: {line}");
            (cols[0], cols[1], cols[2])
        })
        .collect()
}

#[test]
fn glossary_is_valid_and_carries_the_uf_tab_table() {
    let rows = glossary();
    assert!(rows.len() >= 140, "{}", rows.len());
    for (en, vi) in [
        ("Home", "Vẽ"),
        ("Insert", "Chèn"),
        ("Annotate", "Ghi chú"),
        ("Layers", "Lớp bản vẽ"),
        ("View", "Hiển thị"),
        ("Layout", "Bố cục in"),
        ("Properties", "Thuộc tính"),
        ("Xref", "Tham chiếu ngoài"),
        ("Plot", "In bản vẽ"),
    ] {
        assert!(rows.contains(&("ribbon", en, vi)), "{en}");
    }
    let mut seen = BTreeSet::new();
    for (context, en, vi) in &rows {
        assert!(["", "menu", "ribbon"].contains(context), "unknown context: {context}");
        assert!(seen.insert((*context, *en)), "duplicate glossary row: {context} {en}");
        assert!(!vi.is_empty(), "{en}");
    }
}

/// Ukrainian is the reference translation and must cover everything. Other languages fall back
/// to English for what they don't cover yet; their gaps are reported, not enforced, so adding a
/// command doesn't require a row in every language.
const REQUIRED: &[&str] = &["uk"];
const REPORTED: &[&str] = &["pt-BR", "de", "es", "cs", "ru", "fr", "zh", "it", "ko", "sv", "ar", "vi"];

fn check(language: &str, what: &str, missing: Vec<String>) {
    if REQUIRED.contains(&language) {
        assert!(missing.is_empty(), "{language}: missing {what}: {missing:?}");
    } else if !missing.is_empty() {
        eprintln!("{language}: {} {what} fall back to English: {missing:?}", missing.len());
    }
}

#[test]
fn translations_cover_commands_menus_tools_and_snap_labels() {
    for language in REQUIRED.iter().chain(REPORTED) {
        check(language, "translations", missing_commands_menus_tools_and_snap_labels(language));
    }
}

#[derive(Clone, Copy, Debug)]
enum Section {
    Menus(&'static str),
    UiCommands,
    ToolSets,
    Snap,
    Literals(&'static str),
    Prompts,
    Ribbon,
}

const VI_DONE: &[Section] = &[
    Section::Menus("File"),
    Section::Menus("Edit"),
    Section::Menus("View"),
    Section::Menus("Insert"),
    Section::Menus("Format"),
    Section::Menus("Tools"),
    Section::Menus("Draw"),
    Section::Menus("Dimension"),
    Section::Menus("Modify"),
    Section::Menus("Window"),
    Section::Menus("Help"),
    Section::Menus(""),
    Section::UiCommands,
    Section::ToolSets,
    Section::Snap,
];

const MARKED_UI_SOURCES: &[(&str, &str)] = &[
    ("xrefs.rs", include_str!("../xrefs.rs")),
    ("canvas.rs", include_str!("../canvas.rs")),
    ("chrome.rs", include_str!("../chrome.rs")),
    ("cmdline.rs", include_str!("../cmdline.rs")),
    ("credits.rs", include_str!("../credits.rs")),
    ("palettes/quickcalc.rs", include_str!("../palettes/quickcalc.rs")),
    ("palettes/patterns.rs", include_str!("../palettes/patterns.rs")),
    ("dialogs.rs", include_str!("../dialogs.rs")),
    ("hatchdlg.rs", include_str!("../hatchdlg.rs")),
    ("mtext_editor.rs", include_str!("../mtext_editor.rs")),
    ("layers.rs", include_str!("../layers.rs")),
    ("canvas/compare.rs", include_str!("../canvas/compare.rs")),
    ("canvas/blockedit.rs", include_str!("../canvas/blockedit.rs")),
    ("bedit.rs", include_str!("../bedit.rs")),
    ("palettes.rs", include_str!("../palettes.rs")),
    ("parametric.rs", include_str!("../parametric.rs")),
    ("quick.rs", include_str!("../quick.rs")),
    ("vpscale.rs", include_str!("../vpscale.rs")),
    ("options/dialog.rs", include_str!("../options/dialog.rs")),
    ("wsprofile/picker.rs", include_str!("../wsprofile/picker.rs")),
    ("wsprofile/mod.rs", include_str!("../wsprofile/mod.rs")),
    ("wsprofile/sync.rs", include_str!("../wsprofile/sync.rs")),
    ("wsprofile/conflict.rs", include_str!("../wsprofile/conflict.rs")),
    ("wsprofile/tab_conflict.rs", include_str!("../wsprofile/tab_conflict.rs")),
    ("shortcuts.rs", include_str!("../shortcuts.rs")),
    ("quick_access.rs", include_str!("../quick_access.rs")),
    ("ribbon/mod.rs", include_str!("../ribbon/mod.rs")),
    ("ribbon/render.rs", include_str!("../ribbon/render.rs")),
    ("customize/mod.rs", include_str!("../customize/mod.rs")),
    ("customize/ribbon_tab.rs", include_str!("../customize/ribbon_tab.rs")),
    ("customize/toolbars_tab.rs", include_str!("../customize/toolbars_tab.rs")),
    ("customize/keyboard_tab.rs", include_str!("../customize/keyboard_tab.rs")),
    ("customize/quick_access_tab.rs", include_str!("../customize/quick_access_tab.rs")),
    ("toolbars/mod.rs", include_str!("../toolbars/mod.rs")),
    ("toolbars/render.rs", include_str!("../toolbars/render.rs")),
    ("toolbars/menu.rs", include_str!("../toolbars/menu.rs")),
];

fn menu_tops() -> BTreeSet<&'static str> {
    std::iter::once("")
        .chain(crate::menus::MENUS.iter().copied())
        .chain(cadcraft_engine::command_specs().iter().filter_map(|c| c.menu.first().copied()))
        .chain(crate::menus::UI_COMMANDS.iter().filter_map(|(_, _, path, _)| path.first().copied()))
        .collect()
}

fn section_sources(section: Section) -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    match section {
        Section::Menus(top) => {
            sources.extend(crate::menus::MENUS.iter().filter(|name| **name == top).map(|name| (*name).to_string()));
            for c in cadcraft_engine::command_specs() {
                if c.menu.first().copied().unwrap_or_default() == top {
                    sources.insert(c.label.to_string());
                    sources.extend(c.menu.iter().map(|name| (*name).to_string()));
                }
            }
            for (_, label, path, _) in crate::menus::UI_COMMANDS {
                if path.first().copied().unwrap_or_default() == top {
                    sources.insert((*label).to_string());
                    sources.extend(path.iter().map(|name| (*name).to_string()));
                }
            }
        }
        Section::UiCommands => {
            for (_, label, path, _) in crate::menus::UI_COMMANDS {
                sources.insert((*label).to_string());
                sources.extend(path.iter().map(|name| (*name).to_string()));
            }
        }
        Section::ToolSets => {
            for groups in [crate::palettes::DRAFTING, crate::palettes::MODELING] {
                for (name, large, small) in groups {
                    sources.insert((*name).to_string());
                    sources.extend(large.iter().chain(*small).map(|(_, _, label)| (*label).to_string()));
                }
            }
        }
        Section::Snap => sources.extend(cadcraft_engine::snap::mode::ALL.iter().map(|(_, name)| (*name).to_string())),
        Section::Literals(file) => {
            let (_, source) = MARKED_UI_SOURCES.iter().find(|(name, _)| *name == file).unwrap();
            for marker in ["crate::tl!(", "crate::tf!("] {
                for rest in source.split(marker).skip(1) {
                    // These macros require a quoted literal. Use JSON's string decoding for escapes.
                    let rest = rest.trim_start();
                    let mut escape = false;
                    let end = rest
                        .char_indices()
                        .skip(1)
                        .find_map(|(i, c)| {
                            if escape {
                                escape = false;
                                None
                            } else if c == '\\' {
                                escape = true;
                                None
                            } else if c == '"' {
                                Some(i + 1)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    let key: String = serde_json::from_str(&rest[..end]).unwrap();
                    // Coordinate axes, glyph buttons and product names are not translated.
                    if !["X", "Y", "B", "UbotCAD"].contains(&key.as_str()) {
                        sources.insert(key);
                    }
                }
            }
        }
        Section::Prompts => {}
        Section::Ribbon => {
            for id in crate::wsprofile::presets::PRESET_IDS {
                let profile = crate::wsprofile::presets::preset(id).unwrap();
                for tab in profile.layout.ribbon.tabs {
                    sources.insert(tab.label);
                    sources.extend(tab.groups.into_iter().map(|group| group.label));
                }
            }
        }
    }
    if !matches!(section, Section::Literals(_)) {
        sources.remove("");
    }
    sources
}

fn missing_commands_menus_tools_and_snap_labels(language: &str) -> Vec<String> {
    let catalog = catalog(language).unwrap();
    let mut sources = BTreeSet::new();
    for section in menu_tops().into_iter().map(Section::Menus).chain([Section::UiCommands, Section::ToolSets, Section::Snap]) {
        sources.extend(section_sources(section));
    }
    sources.into_iter().filter(|s| !catalog.contains_key(&("", s.as_str()))).collect()
}

#[test]
fn every_marked_ui_literal_has_a_translation() {
    for language in REQUIRED.iter().chain(REPORTED) {
        check(language, "UI translations", missing_marked_ui_literals(language).into_iter().collect());
    }
}

fn missing_marked_ui_literals(language: &str) -> BTreeSet<String> {
    let catalog = catalog(language).unwrap();
    MARKED_UI_SOURCES
        .iter()
        .flat_map(|(file, _)| section_sources(Section::Literals(file)))
        .filter(|key| !catalog.contains_key(&("", key.as_str())))
        .collect()
}

fn missing_vi_section(section: Section) -> BTreeSet<String> {
    let catalog = catalog("vi").unwrap();
    let context = if matches!(section, Section::Ribbon) { "ribbon" } else { "" };
    section_sources(section)
        .into_iter()
        .filter(|key| !catalog.contains_key(&(context, key.as_str())) && !catalog.contains_key(&("", key.as_str())))
        .collect()
}

#[test]
fn section_sources_keep_menu_and_literal_boundaries() {
    let file = section_sources(Section::Menus("File"));
    assert!(file.contains("File"));
    assert!(file.contains("New Drawing..."));
    assert!(!file.contains("Line"));
    assert!(!file.contains("Drawing as bytes"));
    let unplaced = section_sources(Section::Menus(""));
    assert!(unplaced.contains("Drawing as bytes"));
    assert!(!unplaced.contains("New Drawing..."));
    assert!(!unplaced.contains(""));
    assert!(section_sources(Section::Menus("Tools")).contains("Geometric Constraints"));
    assert!(section_sources(Section::Menus("Tools")).contains("Coincident"));
    assert!(section_sources(Section::UiCommands).contains("Calibrate 3D Mouse"));
    assert!(section_sources(Section::ToolSets).contains("Primitives"));
    assert!(section_sources(Section::Snap).contains("Endpoint"));
    let xrefs = section_sources(Section::Literals("xrefs.rs"));
    assert!(xrefs.contains("Attach DWG..."));
    assert!(xrefs.contains("References: {n}"));
    assert!(!xrefs.contains("New Drawing..."));
    assert!(section_sources(Section::Ribbon).contains("Home"));
    assert!(!missing_vi_section(Section::Ribbon).contains("Home"));
}

#[test]
fn vietnamese_sections_are_complete() {
    let mut missing = Vec::new();
    for section in VI_DONE {
        let keys = missing_vi_section(*section);
        if !keys.is_empty() {
            missing.push(format!("{section:?}: {keys:?}"));
        }
    }
    assert!(missing.is_empty(), "missing Vietnamese section translations:\n{}", missing.join("\n"));
}

#[test]
#[ignore]
fn dump_missing_vi() {
    let Some(directory) = std::env::var_os("I18N_DUMP_DIR") else {
        eprintln!("Vietnamese catalog dump skipped: I18N_DUMP_DIR is unset");
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let sections = menu_tops()
        .into_iter()
        .map(Section::Menus)
        .chain([Section::UiCommands, Section::ToolSets, Section::Snap])
        .chain(MARKED_UI_SOURCES.iter().map(|(file, _)| Section::Literals(file)))
        .chain([Section::Prompts, Section::Ribbon]);
    for section in sections {
        let name = match section {
            Section::Menus("") => "menus-unplaced".to_string(),
            Section::Menus(top) => format!("menus-{}", top.to_ascii_lowercase()),
            Section::UiCommands => "ui-commands".to_string(),
            Section::ToolSets => "tool-sets".to_string(),
            Section::Snap => "snap".to_string(),
            Section::Literals(file) => format!("literals-{}", file.replace(['/', '\\', '.'], "-")),
            Section::Prompts => "prompts".to_string(),
            Section::Ribbon => "ribbon".to_string(),
        };
        let rows: String = missing_vi_section(section).into_iter().map(|key| format!("\t{key}\t\n")).collect();
        std::fs::write(directory.join(format!("vi-missing-{name}.tsv")), rows).unwrap();
    }
}

#[test]
fn locale_and_saved_preference_resolution() {
    for tag in ["uk", "uk-UA", "uk_UA.UTF-8", "UK-ua", " uk-UA ", "uk_UA@euro"] {
        assert_eq!(locale_language(tag), Some("uk"), "{tag}");
    }
    for tag in ["de", "de-DE", "de_DE.UTF-8", "DE-at", "de-CH"] {
        assert_eq!(locale_language(tag), Some("de"), "{tag}");
    }
    for (tag, code) in [("ru", "ru"), ("ru-RU", "ru"), ("ru_RU.UTF-8", "ru"), ("fr", "fr"), ("fr-FR", "fr"), ("fr_CA.UTF-8", "fr")] {
        assert_eq!(locale_language(tag), Some(code), "{tag}");
    }
    for tag in ["ua", "ukulele", "pl-PL", "", "garbage", "💙"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    for tag in ["pt", "pt-BR", "pt_BR.UTF-8", "PT-br", "pt-PT"] {
        assert_eq!(locale_language(tag), Some("pt-BR"), "{tag}");
    }
    assert_eq!(locale_language("ptolemy"), None);
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "pt-BR".into()]), "pt-BR");
    assert_eq!(Preference::Auto.resolve(&["fr-FR".into(), "pt-BR".into()]), "fr");
    for tag in ["es", "es-ES", "es_MX.UTF-8", "ES-ar"] {
        assert_eq!(locale_language(tag), Some("es"), "{tag}");
    }
    assert_eq!(Preference::Auto.resolve(&["es-ES".into(), "pt-BR".into()]), "es");
    assert_eq!(Preference::parse("es"), Some(Preference::Spanish));
    assert_eq!(Preference::BrazilianPortuguese.resolve(&["uk-UA".into()]), "pt-BR");
    assert_eq!(Preference::parse("pt-BR"), Some(Preference::BrazilianPortuguese));
    let locales = vec!["pl-PL".into(), "uk-UA".into(), "en-US".into()];
    assert_eq!(Preference::Auto.resolve(&locales), "uk");
    assert_eq!(Preference::English.resolve(&locales), "en");
    assert_eq!(Preference::German.resolve(&locales), "de");
    assert_eq!(Preference::Auto.resolve(&["de-DE".into(), "en-US".into()]), "de");
    assert_eq!(Preference::Auto.resolve(&["en-US".into(), "uk-UA".into()]), "en");
    assert_eq!(Preference::Auto.resolve(&[]), "en");
    assert_eq!(Preference::restore(None), Preference::Auto);
    assert_eq!(Preference::restore(Some("obsolete")), Preference::Auto);
    for preference in Preference::ALL {
        assert_eq!(Preference::restore(Some(preference.code())), preference);
        let saved = serde_json::to_string(&preference).unwrap();
        assert_eq!(serde_json::from_str::<Preference>(&saved).unwrap(), preference);
    }
    assert!(serde_json::from_str::<Preference>("\"unsupported\"").is_err());
    assert_eq!(serde_json::from_value::<crate::UiState>(json!({})).unwrap().interface_language, Preference::Auto);
    // Saved with the host's other preferences; a bad saved value keeps the current choice.
    let mut a = CadApp::new(Session::new(), Services::default());
    a.ui.interface_language = Preference::Ukrainian;
    let mut b = CadApp::new(Session::new(), Services::default());
    b.load_prefs(&a.prefs_json());
    assert_eq!(b.ui.interface_language, Preference::Ukrainian);
    b.load_prefs(r#"{"interfaceLanguage":"klingon"}"#);
    assert_eq!(b.ui.interface_language, Preference::Ukrainian);
}

#[test]
fn plural_forms_and_templates_preserve_inserted_user_data() {
    with_language("uk", || {
        for (n, form) in [
            (0, 2),
            (1, 0),
            (2, 1),
            (4, 1),
            (5, 2),
            (11, 2),
            (12, 2),
            (14, 2),
            (21, 0),
            (22, 1),
            (25, 2),
            (101, 0),
            (111, 2),
            (114, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(plural_uk(n), form, "{n}");
        }
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 учасник");
        assert_eq!(count(22, "{n} contributor", "{n} contributors"), "22 учасники");
        assert_eq!(count(11, "{n} contributor", "{n} contributors"), "11 учасників");
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Поточний: {n} Open");
        assert_eq!(format("{b}/{a}", &[("a", "А".into()), ("b", "Б".into())]), "Б/А");
    });
    assert_eq!(t("Layers"), "Layers");
    with_language("de", || {
        assert_eq!(t("Layers"), "Layer");
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 Mitwirkender");
        assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 Mitwirkende");
        assert_eq!(count(11, "{n} contributor", "{n} contributors"), "11 Mitwirkende");
    });
    with_language("uk", || {
        assert_eq!(t("Layers"), "Шари");
        with_language("en", || assert_eq!(t("Layers"), "Layers"));
        assert_eq!(t("Layers"), "Шари");
    });
    assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 contributors");
}

#[test]
fn brazilian_portuguese_plurals_and_accents() {
    for (n, form) in [(0, 0), (1, 0), (2, 1), (5, 1), (21, 1), (u64::MAX, 1)] {
        assert_eq!(plural_pt(n), form, "{n}");
    }
    with_language("pt-BR", || {
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 colaborador");
        assert_eq!(count(3, "{n} contributor", "{n} contributors"), "3 colaboradores");
        assert_eq!(t("Layers"), "Camadas");
        assert_eq!(t("Dimension"), "Cota");
        assert_eq!(format("Height: {height}", &[("height", "{n}".into())]), "Altura: {n}");
    });
    assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 contributors");
}

#[test]
fn live_switch_to_brazilian_portuguese_translates_the_shell() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    let ctx = egui::Context::default();
    app.logic(&ctx);
    app.start("line");
    app.run("ui.language", json!({"lang":"pt-BR"})).unwrap();
    let text = render(&mut app, &ctx);
    assert!(text.contains("Propriedades"), "{text}");
    assert!(text.contains("Arquivo"), "{text}");
    assert_eq!(app.language(), "pt-BR");
    assert_eq!(t("Properties"), "Properties", "language must not leak out of the render scope");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> String {
    fn collect(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(s) => {
                text.push_str(s.galley.text());
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

fn render(app: &mut CadApp, ctx: &egui::Context) -> String {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
    // Windows fade in; a second pass makes their text visible without a time dependency.
    let mut text = String::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(input.clone(), |ui| app.ui(ui));
        text = painted_text(&output.shapes);
        output.textures_delta.clear();
    }
    text
}

#[test]
fn live_switch_translates_the_shell_without_mutating_drawing_or_automation() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    let before = render(&mut app, &ctx);
    assert!(before.contains("Properties"));
    let commands = crate::control::all_commands(&app);
    let (menu_request, _) = crate::ControlRequest::new("ui.menu.list", json!({}));
    let crate::control::Outcome::Done(menu_before) = crate::control::handle(&mut app, &ctx, &menu_request) else { panic!("expected menu reply") };
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    app.run("ui.language", json!({"lang":"uk"})).unwrap();
    let after = render(&mut app, &ctx);
    assert!(after.contains("Властивості"), "{after}");
    assert!(after.contains("Файл"), "{after}");
    assert!(after.contains("Укажіть першу точку"), "{after}");
    assert!(after.contains("Open"), "the drawing title must remain unchanged");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    let crate::control::Outcome::Done(menu_after) = crate::control::handle(&mut app, &ctx, &menu_request) else { panic!("expected menu reply") };
    assert_eq!(menu_before, menu_after);
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(t("Properties"), "Properties", "language must not leak out of the render scope");
    assert!(app.run("ui.language", json!({"lang":"bad"})).is_err());
    assert_eq!(app.language(), "uk");
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.language(), "uk");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
    // The menu tree used by the control channel keeps its original labels and IDs.
    assert_eq!(crate::menus::tree(&app).first().unwrap().0, "File");
}

#[test]
fn dialogs_render_in_ukrainian_and_keep_their_source_ids() {
    for (dialog, heading) in [
        ("dsettings", "Налаштування креслення"),
        ("about", "Про UbotCAD"),
        ("commands", "Довідник команд"),
        ("blocks", "Блоки"),
        ("layers", "Диспетчер властивостей шарів"),
        ("qselect", "Швидкий вибір"),
        ("parameters", "Диспетчер параметрів"),
        ("language", "Мова інтерфейсу"),
        ("hatch", "Штрихування та градієнт"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Ukrainian;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_every_german_special_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("de.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn default_fonts_cover_every_brazilian_portuguese_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("pt_br.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn default_fonts_cover_every_ukrainian_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    // Use only egui's bundled fonts, exactly as the web build does (no system-font loader).
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("uk.tsv").chars().filter(|c| ('\u{0400}'..='\u{04ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            // In egui 0.36, has_glyph can report a false negative for glyphs in the same
            // face as the replacement character. Check the family's actual character maps.
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn czech_locales_preferences_and_plural_forms() {
    for tag in ["cs", "cs-CZ", "cs_CZ.UTF-8", "CS-cz", " cs-CZ ", "cs_CZ@euro"] {
        assert_eq!(locale_language(tag), Some("cs"), "{tag}");
    }
    for tag in ["cz", "csharp", "csomething"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("cs"), Some(Preference::Czech));
    assert_eq!(Preference::Czech.name(), "Čeština");
    assert_eq!(Preference::Czech.resolve(&["en-US".into()]), "cs");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "cs-CZ".into(), "en-US".into()]), "cs");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Czech;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Czech);
    with_language("cs", || {
        for (n, form) in [(0, 2), (1, 0), (2, 1), (4, 1), (5, 2), (11, 2), (21, 2), (22, 2), (25, 2), (u64::MAX, 2)] {
            assert_eq!(plural_cs(n), form, "{n}");
            let contributor = ["přispěvatel", "přispěvatelé", "přispěvatelů"][form];
            let commit = ["commit", "commity", "commitů"][form];
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {contributor}"));
            assert_eq!(count(n, "{n} commit", "{n} commits"), format!("{n} {commit}"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Aktuální: {n} Open");
    });
}

#[test]
fn czech_menu_headings_keep_object_labels_and_other_catalogs() {
    with_language("cs", || {
        assert_eq!(menu("View"), "Zobrazení");
        assert_eq!(t("View"), "Pohled");
        assert_eq!(menu("Dimension"), "Kótování");
        assert_eq!(t("Dimension"), "Kóta");
        assert_eq!(menu("Draw"), "Kreslení");
        assert_eq!(menu("Modify"), "Modifikace");
        assert_eq!(menu("Edit"), "Úpravy");
        assert_eq!(menu("File"), "Soubor");
    });
    for language in ["en", "uk", "de", "es", "pt-BR", "unknown"] {
        for source in crate::menus::MENUS {
            assert_eq!(tr_menu(language, source), tr(language, source));
        }
    }
}

#[test]
fn live_czech_switch_preserves_drawing_command_and_saved_choice() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    app.run("ui.language", json!({"lang":"cs"})).unwrap();
    let text = render(&mut app, &ctx);
    for label in ["Vlastnosti", "Soubor", "Zobrazení", "Kreslení", "Kótování", "Určete první bod"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "cs");
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    assert_eq!(t("Properties"), "Properties");
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.language(), "cs");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn czech_dialogs_render_with_stable_ids() {
    for (dialog, heading) in [
        ("dsettings", "Nastavení kreslení"),
        ("about", "O aplikaci UbotCAD"),
        ("commands", "Přehled příkazů"),
        ("blocks", "Bloky"),
        ("layers", "Správce vlastností hladin"),
        ("qselect", "Rychlý výběr"),
        ("parameters", "Správce parametrů"),
        ("language", "Čeština"),
        ("options:preferences", "Uživatelské předvolby"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Czech;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_czech_letters_and_typographic_quotes() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = "ÁČĎÉĚÍŇÓŘŠŤÚŮÝŽáčďéěíňóřšťúůýž„“".chars().collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn russian_and_french_plurals() {
    with_language("ru", || {
        for (n, word) in [(1, "участник"), (2, "участника"), (5, "участников"), (21, "участник"), (22, "участника"), (11, "участников")]
        {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
    });
    with_language("fr", || {
        for (n, word) in [(0, "contributeur"), (1, "contributeur"), (2, "contributeurs"), (10, "contributeurs")] {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
    });
}

#[test]
fn italian_locales_plurals_and_accents() {
    for tag in ["it", "it-IT", "it_IT.UTF-8", "IT-ch", " it-CH ", "it_IT@euro"] {
        assert_eq!(locale_language(tag), Some("it"), "{tag}");
    }
    assert_eq!(locale_language("italic"), None);
    assert_eq!(Preference::parse("it"), Some(Preference::Italian));
    assert_eq!(Preference::Auto.resolve(&["it-IT".into(), "en-US".into()]), "it");
    assert_eq!(Preference::Italian.resolve(&["uk-UA".into()]), "it");
    with_language("it", || {
        for (n, word) in [(0, "collaboratori"), (1, "collaboratore"), (2, "collaboratori"), (21, "collaboratori")] {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
        assert_eq!(t("Draw"), "Disegna");
        assert_eq!(tr_menu("it", "Modify"), "Edita");
        assert_eq!(tr("it", "Modify"), "Modifica");
        assert_eq!(tr_menu("it", "View"), "Visualizza");
        assert_eq!(tr("it", "View"), "Vista");
    });
    // Italian accented letters (à è é ì ò ù and capitals) come from the bundled egui fonts.
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("it.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    assert!(letters.contains(&'à') && letters.contains(&'è'));
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

/// Chinese shows only with an installed CJK font (none is bundled); without one the interface
/// stays English instead of drawing boxes, and choosing it says why.
#[test]
fn chinese_needs_an_installed_cjk_font() {
    assert_eq!(locale_language("zh-CN"), Some("zh"));
    let mut app = CadApp::new(Session::new(), Services::default());
    let r = app.run("ui.language", json!({"lang": "zh"})).unwrap();
    assert_eq!(r["interfaceLanguage"], "zh");
    let font = crate::theme::cjk_font();
    assert_eq!(app.language(), if font.is_some() { "zh" } else { "en" });
    assert_eq!(r["shown"], app.language());
    assert!(crate::theme::can_show("uk") && crate::theme::can_show("en"));
    if font.is_none() {
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No Chinese font")));
    }
}

#[test]
fn hatch_dialog_extents_label_says_extents_in_ukrainian() {
    assert_eq!(tr("uk", "Default to boundary extents"), "Типово за габаритами контуру");
}

#[test]
fn korean_representative_translations_remain_available() {
    let ko = catalog("ko").unwrap();
    // Fixed regression examples; new source strings remain reported, not required for Korean.
    assert!(REPORTED.contains(&"ko"));
    for (context, source, expected) in [
        ("", "File", "파일"),
        ("menu", "View", "보기"),
        ("menu", "Draw", "그리기"),
        ("menu", "Dimension", "치수"),
        ("menu", "Modify", "수정"),
        ("", "Options...", "옵션..."),
        ("", "User Preferences", "사용자 기본 설정"),
        ("", "Interface Language", "인터페이스 언어"),
        ("", "Properties", "특성"),
        ("", "Degree", "도 기호"),
        ("", "Style: Standard", "스타일: Standard"),
        ("", "WCS ⌄", "WCS 선택"),
        ("@plural", "{n} contributor", "기여자 {n}명"),
        ("@plural", "{n} commit", "커밋 {n}개"),
    ] {
        assert_eq!(ko.get(&(context, source)).copied(), Some(expected), "{context}: {source}");
    }
    assert_eq!(tr("ko", "future untranslated label"), "future untranslated label");
}

#[test]
fn korean_locale_saved_preference_and_single_count_form() {
    for tag in ["ko", "ko-KR", "ko_KR.UTF-8", "KO-kr", " ko-KR ", "ko_KR@euro"] {
        assert_eq!(locale_language(tag), Some("ko"), "{tag}");
    }
    for tag in ["kr", "korean", "kotlin"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("ko"), Some(Preference::Korean));
    assert_eq!(Preference::Korean.resolve(&["en-US".into()]), "ko");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "ko-KR".into(), "en-US".into()]), "ko");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.system_languages = vec!["ko-KR".into()];
    app.run("ui.language", json!({"lang": "ko"})).unwrap();
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.system_languages = vec!["en-US".into()];
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Korean);
    assert_eq!(restored.language(), app.language());
    with_language("ko", || {
        for n in [0, 1, 2, 5, 11, 21, 101, u64::MAX] {
            assert_eq!(plural_single(n), 0);
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("기여자 {n}명"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "현재: {n} Open");
    });
}

#[test]
fn korean_live_switch_keeps_automation_and_drawing_and_falls_back_without_font() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    let result = app.run("ui.language", json!({"lang": "ko"})).unwrap();
    let supported = crate::theme::can_show("ko");
    assert_eq!(result["shown"], if supported { "ko" } else { "en" });
    assert_eq!(app.ui.interface_language, Preference::Korean);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(app.language()));
    let text = render(&mut app, &ctx);
    if supported {
        for label in ["특성", "파일", "첫 번째 점 지정"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
    } else {
        assert!(text.contains("Properties"));
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No font covers the Korean interface")));
        assert_eq!(Preference::Korean.name(), "Korean");
    }
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    assert_eq!(t("Properties"), "Properties");
    assert!(app.run("ui.language", json!({"lang": "bad"})).is_err());
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.ui.interface_language, Preference::Korean);
    // The general ui.set route uses the same saved preference and font installation.
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "zh"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    assert_eq!(app.ui.interface_language, Preference::Chinese);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(if crate::theme::can_show("zh") { "zh" } else { "en" }));
    render(&mut app, &ctx);
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "ko"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    assert_eq!(app.ui.interface_language, Preference::Korean);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(app.language()));
    let text = render(&mut app, &ctx);
    assert!(text.contains(if supported { "특성" } else { "Properties" }));
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "en"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some("en"));
    assert_eq!(app.ui.interface_language, Preference::English);
    assert!(render(&mut app, &ctx).contains("Properties"));
    assert_eq!(crate::menus::tree(&app).first().unwrap().0, "File");
}

#[test]
fn korean_dialogs_keep_source_ids() {
    if !crate::theme::can_show("ko") {
        return; // The fallback path is covered above; do not claim glyph rendering here.
    }
    for (dialog, source) in [
        ("dsettings", "Drafting Settings"),
        ("about", "About UbotCAD"),
        ("commands", "Command Reference"),
        ("blocks", "Blocks"),
        ("layers", "Layer Properties Manager"),
        ("qselect", "Quick Select"),
        ("parameters", "Parameters Manager"),
        ("language", "Interface Language"),
        ("options:preferences", "User Preferences"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Korean;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(tr("ko", source)), "{dialog}: {text}");
        if dialog == "language" {
            assert!(text.contains("한국어"), "{dialog}: {text}");
        }
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn korean_ui_fonts_cover_catalog_and_language_picker_in_both_families() {
    if !crate::theme::can_show("ko") {
        return;
    }
    let letters: BTreeSet<_> = include_str!("ko.tsv")
        .lines()
        .filter_map(|line| line.split('\t').nth(2))
        .flat_map(str::chars)
        .chain("한국어".chars())
        .filter(|c| !c.is_ascii())
        .collect();
    // The picker shows 한국어 in an English interface only when the CJK fonts loaded in every
    // language cover Hangul; the Korean interface itself always does, with a fallback font.
    let ctx = egui::Context::default();
    let english = crate::theme::cjk_covers_hangul();
    assert_eq!(with_language("en", || Preference::Korean.name()), if english { "한국어" } else { "Korean" });
    assert_eq!(with_language("ko", || Preference::Korean.name()), "한국어");
    crate::theme::install_fonts(&ctx, if english { "en" } else { "ko" });
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let missing = ctx.fonts_mut(|view| {
        let mut missing = Vec::new();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                if !font.characters().contains_key(c) {
                    missing.push(format!("{family:?}: {c} (U+{:04X})", u32::from(*c)));
                }
            }
        }
        missing
    });
    assert!(missing.is_empty(), "missing Korean UI glyphs: {}", missing.join(", "));
}

#[test]
fn swedish_locales_preferences_and_plural_forms() {
    for tag in ["sv", "sv-SE", "sv_SE.UTF-8", "SV-se", " sv-SE ", "sv-FI"] {
        assert_eq!(locale_language(tag), Some("sv"), "{tag}");
    }
    // `se` is Northern Sami and `swe` the three-letter code; neither picks Swedish.
    for tag in ["se", "swe", "svenska"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("sv"), Some(Preference::Swedish));
    assert_eq!(Preference::Swedish.name(), "Svenska");
    assert_eq!(Preference::Swedish.resolve(&["en-US".into()]), "sv");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "sv-SE".into(), "en-US".into()]), "sv");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Swedish;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Swedish);
    with_language("sv", || {
        for n in [0, 1, 2, 5, 21, u64::MAX] {
            // `bidragsgivare` is the same in the singular and the plural; `commit` takes an -s.
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} bidragsgivare"));
            let commit = if n == 1 { "commit" } else { "commits" };
            assert_eq!(count(n, "{n} commit", "{n} commits"), format!("{n} {commit}"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Aktuell: {n} Open");
    });
}

#[test]
fn swedish_menu_headings_keep_object_labels() {
    with_language("sv", || {
        assert_eq!(menu("Dimension"), "Måttsättning");
        assert_eq!(t("Dimension"), "Mått");
        assert_eq!(menu("View"), "Visa");
        assert_eq!(menu("Draw"), "Rita");
        assert_eq!(menu("Modify"), "Ändra");
        assert_eq!(menu("Edit"), "Redigera");
        assert_eq!(menu("File"), "Arkiv");
    });
}

#[test]
fn live_swedish_switch_preserves_drawing_command_and_saved_choice() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    app.run("ui.language", json!({"lang":"sv"})).unwrap();
    let text = render(&mut app, &ctx);
    for label in ["Egenskaper", "Arkiv", "Visa", "Rita", "Måttsättning", "Ange första punkten"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "sv");
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn swedish_dialogs_render_with_stable_ids() {
    for (dialog, heading) in [
        ("dsettings", "Ritinställningar"),
        ("about", "Om UbotCAD"),
        ("blocks", "Block"),
        ("layers", "Hantera lageregenskaper"),
        ("qselect", "Snabbmarkering"),
        ("parameters", "Hantera parametrar"),
        ("language", "Svenska"),
        ("options:preferences", "Användarinställningar"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Swedish;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_swedish_letters_and_quotes() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = "ÅÄÖÉåäöé”–".chars().collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn vietnamese_locales_preferences_and_single_count_form() {
    for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vn", " vi-VN "] {
        assert_eq!(locale_language(tag), Some("vi"), "{tag}");
    }
    for tag in ["vie", "vn"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("vi"), Some(Preference::Vietnamese));
    assert_eq!(Preference::Vietnamese.name(), "Tiếng Việt");
    assert_eq!(Preference::Vietnamese.resolve(&["en-US".into()]), "vi");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "vi-VN".into()]), "vi");
    assert_eq!(Preference::restore(Some("vi")), Preference::Vietnamese);
    assert_eq!(serde_json::to_string(&Preference::Vietnamese).unwrap(), "\"vi\"");
    assert_eq!(serde_json::from_str::<Preference>("\"vi\"").unwrap(), Preference::Vietnamese);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Vietnamese;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    with_language("vi", || {
        for n in [0, 1, 2, 5, 21, u64::MAX] {
            assert_eq!(plural_single(n), 0);
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} người đóng góp"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Hiện hành: {n} Open");
    });
}

#[test]
fn ribbon_tab_labels_follow_the_uf_table() {
    for (en, vi) in [
        ("Home", "Vẽ"),
        ("Insert", "Chèn"),
        ("Annotate", "Ghi chú"),
        ("Layers", "Lớp bản vẽ"),
        ("View", "Hiển thị"),
        ("Layout", "Bố cục in"),
        ("Properties", "Thuộc tính"),
        ("Xref", "Tham chiếu ngoài"),
        ("Plot", "In bản vẽ"),
    ] {
        assert_eq!(tr_ribbon("vi", en), vi, "{en}");
        assert_eq!(with_language("vi", || ribbon(en)), vi, "{en}");
    }
    assert_eq!(tr_ribbon("uk", "Home"), tr("uk", "Home"));
    assert_eq!(tr_ribbon("vi", "Open"), tr("vi", "Open"));
    assert_eq!(tr_ribbon("unknown", "Home"), "Home");
}

#[test]
fn live_vietnamese_switch_preserves_drawing_command_and_saved_choice() {
    if !crate::theme::can_show("vi") {
        eprintln!("Vietnamese rendering skipped: no covering UI font; fallback is tested separately");
        return;
    }
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    let menus = format!("{:?}", crate::menus::tree(&app));
    let drawing = app.session.state().unwrap().clone();
    app.run("ui.language", json!({"lang":"vi"})).unwrap();
    app.logic(&ctx);
    let text = render(&mut app, &ctx);
    for label in ["Thuộc tính", "Tệp", "Chỉ định điểm đầu"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "vi");
    assert_eq!(app.fonts_language, Some("vi"));
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    app.ui.dialog = Some("language".into());
    for language in ["en", "vi", "en"] {
        app.run("ui.language", json!({"lang":language})).unwrap();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(if language == "vi" { "Ngôn ngữ giao diện" } else { "Interface Language" }), "{text}");
        assert_eq!(app.ui.dialog.as_deref(), Some("language"));
        assert_eq!(crate::control::cmdline_state(&app), prompt);
        assert_eq!(crate::control::all_commands(&app), commands);
        assert_eq!(format!("{:?}", crate::menus::tree(&app)), menus);
        let state = app.session.state().unwrap();
        assert_eq!(state.title, "Open");
        assert!(std::sync::Arc::ptr_eq(&state.doc, &drawing.doc));
        assert_eq!(state.selection, drawing.selection);
        assert_eq!(state.views, drawing.views);
        assert_eq!(state.is_dirty(), drawing.is_dirty());
    }
    app.ui.dialog = None;
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn user_named_ribbon_tab_is_not_translated() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ws.effective.layout.ribbon = crate::wsprofile::presets::preset("AUTOCAD_FAMILIAR").unwrap().layout.ribbon;
    app.ws.effective.layout.ribbon.visible = true;
    let home = app.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().clone();
    let mut renamed = home.clone();
    renamed.label = "Bản vẽ kiến trúc".into();
    let mut custom = home.clone();
    custom.id = "custom_1".into();
    custom.label = "Home".into();
    app.ws.effective.layout.ribbon.tabs.push(crate::wsprofile::schema::RibbonTab {
        id: "custom_2".into(),
        label: "Bản vẽ kiến trúc".into(),
        ..Default::default()
    });
    let ctx = egui::Context::default();
    for language in ["vi", "en"] {
        with_language(language, || {
            assert_eq!(crate::ribbon::render::tab_label(&home), if language == "vi" { "Vẽ" } else { "Home" });
            assert_eq!(crate::ribbon::render::tab_label(&renamed), "Bản vẽ kiến trúc");
            renamed.label = "Insert".into();
            assert_eq!(crate::ribbon::render::tab_label(&renamed), "Insert");
            renamed.label = "Bản vẽ kiến trúc".into();
            assert_eq!(crate::ribbon::render::tab_label(&custom), "Home");
            let mut group = home.groups.iter().find(|group| group.id == "draw").unwrap().clone();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), if language == "vi" { "Vẽ" } else { "Draw" });
            group.label = "Insert".into();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), "Insert");
            group.id = "custom_1".into();
            group.label = "Draw".into();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), "Draw");
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| crate::ribbon::render::show(&mut app, ui));
            assert!(painted_text(&output.shapes).contains("Bản vẽ kiến trúc"));
            output.textures_delta.clear();
        });
    }
}

#[test]
fn vietnamese_falls_back_to_english_without_a_covering_font() {
    let mut app = CadApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    let result = app.run("ui.language", json!({"lang":"vi"})).unwrap();
    app.logic(&ctx);
    let supported = crate::theme::can_show("vi");
    assert_eq!(app.language(), if supported { "vi" } else { "en" });
    assert_eq!(result["shown"], app.language());
    assert_eq!(app.ui.interface_language, Preference::Vietnamese);
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    let text = render(&mut app, &ctx);
    if supported {
        assert!(text.contains("Thuộc tính"), "{text}");
    } else {
        assert!(text.contains("Properties"), "{text}");
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No font covers the Vietnamese interface")));
    }
}

#[test]
fn vietnamese_ui_fonts_cover_catalog_when_shown() {
    if !crate::theme::can_show("vi") {
        eprintln!("Vietnamese glyph coverage skipped: no covering UI font");
        return;
    }
    let letters: BTreeSet<_> = include_str!("vi.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(2))
        .flat_map(str::chars)
        .chain("Tiếng Việt".chars())
        .filter(|c| !c.is_ascii())
        .collect();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, "vi");
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let missing = ctx.fonts_mut(|view| {
        let mut missing = Vec::new();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                if !font.characters().contains_key(c) {
                    missing.push(format!("{family:?}: {c} (U+{:04X})", u32::from(*c)));
                }
            }
        }
        missing
    });
    assert!(missing.is_empty(), "missing Vietnamese UI glyphs: {}", missing.join(", "));
}

#[test]
fn glossary_is_approved_before_bulk_rows() {
    let rows = include_str!("vi.tsv").lines().filter(|line| !line.is_empty() && !line.starts_with('#')).count();
    if rows > 60 {
        let status = include_str!("vi_glossary.tsv").lines().find(|line| line.starts_with("# status:"));
        assert!(status.is_some_and(|line| line.starts_with("# status: approved ")), "bulk Vietnamese translation needs an approved glossary");
    }
}

#[test]
fn vietnamese_catalog_follows_the_glossary() {
    let (catalog, errors) = parse(include_str!("vi.tsv"));
    assert!(errors.is_empty(), "{errors:?}");
    let glossary = glossary();
    for (context, key, value) in &glossary {
        if let Some(translated) = catalog.get(&(*context, *key)) {
            assert_eq!(translated, value, "{context}: {key}");
        }
    }
    for ((context, key), value) in &catalog {
        if *context == "ribbon" {
            assert!(glossary.contains(&(*context, *key, *value)), "unknown ribbon translation: {key}");
        }
    }
    for (context, key, value) in glossary.iter().filter(|(context, key, _)| {
        *context == "ribbon" && ["Home", "Insert", "Annotate", "Layers", "View", "Layout", "Properties", "Xref", "Plot"].contains(key)
    }) {
        assert_eq!(catalog.get(&(*context, *key)), Some(value), "missing U-F ribbon row: {key}");
    }
}
