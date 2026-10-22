use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::customize;
use cadcraft_ui_egui::menus::{self, Entry, UI_COMMANDS};
use cadcraft_ui_egui::palette::index::SearchIndex;
use cadcraft_ui_egui::shortcuts::{self, BROWSER_RESERVED, BindError, KeyPlatform, WEB_ALTERNATES};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn app_for(platform: KeyPlatform) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    // Blank overrides let native kittest exercise the web map without retaining desktop defaults.
    let mut overrides: BTreeMap<_, _> = shortcuts::defaults_for(KeyPlatform::Desktop).into_keys().map(|id| (id, String::new())).collect();
    overrides.extend(shortcuts::defaults_for(platform));
    app.run("ui.workspace.set", json!({"pointer": "/shortcuts", "value": overrides})).unwrap();
    app
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            if app.ui.dialog.as_deref() == Some(customize::DIALOG) {
                let mut open = true;
                customize::dialog(app, ui.ctx(), &mut open);
                if !open {
                    app.ui.dialog = None;
                }
            }
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn web_alternates_avoid_browser_keys_h09() {
    let _guard = test_lock();
    let desktop = shortcuts::defaults_for(KeyPlatform::Desktop);
    let web = shortcuts::defaults_for(KeyPlatform::Web);
    let mut seen = BTreeMap::new();
    for (command, keys) in WEB_ALTERNATES.iter().copied().chain(web.iter().map(|(id, keys)| (id.as_str(), keys.as_str()))) {
        assert!(!shortcuts::is_browser_reserved(keys), "{command}: {keys}");
        for key in keys.split(',').map(str::trim) {
            let combo = shortcuts::parse(key).unwrap();
            for reserved in BROWSER_RESERVED {
                assert_ne!(Some(combo.clone()), shortcuts::parse(reserved), "{command}: {key}");
            }
        }
    }
    for (command, keys) in &web {
        for key in keys.split(',').map(str::trim) {
            let normalized = shortcuts::format(&shortcuts::parse(key).unwrap());
            assert!(seen.insert(normalized, command).is_none(), "duplicate web shortcut: {key}");
        }
    }
    assert!(!web.contains_key("ui.palette"));
    assert_eq!(desktop.get("ui.palette").map(String::as_str), Some("Ctrl+Shift+P"));
    for command in ["otrack", "dynmode"] {
        assert!(!web.contains_key(command), "{command} uses the status bar on the web");
    }
    let mut app = app_for(KeyPlatform::Web);
    for key in [
        "Ctrl+L",
        "Ctrl+T",
        "Ctrl+W",
        "Ctrl+R",
        "F5",
        "F11",
        "Shift+F10",
        "Ctrl+Shift+C",
        "Ctrl+Shift+P",
        "Ctrl+Shift+N",
        "Ctrl+Shift+T",
        "Ctrl+Shift+W",
        "Ctrl+Tab",
        "Ctrl+Shift+Tab",
        "Ctrl+Shift+I",
        "Alt+F4",
    ] {
        assert!(BROWSER_RESERVED.contains(&key), "missing browser key: {key}");
        assert!(shortcuts::is_browser_reserved(key), "{key}");
        assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Web, key), None, "{key}");
        app.run("ui.workspace.set", json!({"pointer": "/shortcuts/line", "value": key})).unwrap();
        assert!(!shortcuts::active_for(KeyPlatform::Web, &app).contains_key("line"), "{key}");
        let mut map = web.clone();
        let before = map.clone();
        for replace in [false, true] {
            assert_eq!(shortcuts::bind_for(KeyPlatform::Web, &mut map, "line", key, replace), Err(BindError::BrowserReserved), "{key}");
            assert_eq!(map, before);
        }
    }
    for (command, key) in [
        ("ui.open", "Ctrl+O"),
        ("qsave", "Ctrl+S"),
        ("plot", "Ctrl+P"),
        ("u", "Ctrl+Z"),
        ("selectall", "Ctrl+A"),
        ("copyclip", "Ctrl+C"),
        ("pasteclip", "Ctrl+V"),
        ("cutclip", "Ctrl+X"),
        ("ortho", "F8"),
        ("snap", "F9"),
    ] {
        assert_eq!(web.get(command).map(String::as_str), Some(key), "{command}");
    }
    assert!(web.get("redo").unwrap().split(',').any(|key| key.trim() == "Ctrl+Y"));
}

#[test]
fn properties_and_command_line_have_web_keys() {
    let _guard = test_lock();
    let web = shortcuts::defaults_for(KeyPlatform::Web);
    let desktop = shortcuts::defaults_for(KeyPlatform::Desktop);
    let app = app_for(KeyPlatform::Web);
    for (command, web_key, desktop_key) in
        [("ui.panel.properties", "Ctrl+Shift+1", "Ctrl+1"), ("ui.toggle.cmdline", "Ctrl+Shift+9", "Ctrl+9"), ("ui.dialog.commands", "Shift+F1", "F1")]
    {
        assert_eq!(web.get(command).map(String::as_str), Some(web_key));
        assert_eq!(shortcuts::active_for(KeyPlatform::Web, &app).get(command).map(String::as_str), Some(web_key));
        assert_eq!(desktop.get(command).map(String::as_str), Some(desktop_key));
        assert!(UI_COMMANDS.iter().any(|entry| entry.0 == command));
        let mut map = BTreeMap::new();
        shortcuts::bind_for(KeyPlatform::Web, &mut map, command, web_key, false).unwrap();
        assert_eq!(map.get(command).map(String::as_str), Some(web_key));
    }
    let mut h = harness(app);
    let visible = h.state().ui.show_command_line;
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Num9);
    h.run_steps(3);
    assert_eq!(h.state().ui.show_command_line, !visible);
    h.key_press_modifiers(Modifiers::SHIFT, Key::F1);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("commands"));
}

fn menu_ids(entries: &[Entry], ids: &mut BTreeSet<String>) {
    for entry in entries {
        match entry {
            Entry::Item { id, .. } => {
                ids.insert(id.clone());
            }
            Entry::Sub { children, .. } => menu_ids(children, ids),
        }
    }
}

#[test]
fn web_commands_keep_a_way_in() {
    let _guard = test_lock();
    let app = app_for(KeyPlatform::Web);
    let desktop = shortcuts::defaults_for(KeyPlatform::Desktop);
    let web = shortcuts::defaults_for(KeyPlatform::Web);
    let index = SearchIndex::build("en", &web);
    let mut ids = BTreeSet::new();
    for (_, entries) in menus::tree(&app) {
        menu_ids(&entries, &mut ids);
    }
    assert!(ids.contains("ui.dialog.commands"));
    assert!(ids.contains("ui.palette"));
    for command in desktop.keys().filter(|id| !web.contains_key(*id)) {
        let in_ui = UI_COMMANDS.iter().any(|entry| entry.0 == command.as_str());
        // Drafting toggles without menu paths are also listed in the Help menu's Command Reference.
        let in_reference =
            index.search(command, &[], &[], 1).first().and_then(|hit| index.entry(hit)).is_some_and(|entry| entry.id == command.as_str());
        assert!(in_ui || ids.contains(command) || in_reference, "{command} lost its web entry point");
    }
}

#[test]
fn ctrl_z_in_customize_undoes_layout_on_both_platforms_tc19() {
    let _guard = test_lock();
    for platform in [KeyPlatform::Desktop, KeyPlatform::Web] {
        let mut app = app_for(platform);
        app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
        let drawing = app.session.doc().unwrap().clone();
        app.run("ui.customize.open", json!({})).unwrap();
        let before = app.ws.effective.layout.ribbon.clone();
        app.run("ui.ribbon.group.move", json!({"tab": "home", "group": "draw", "to": 1})).unwrap();
        let reordered = app.ws.effective.layout.ribbon.clone();
        assert_ne!(reordered, before);
        let mut h = harness(app);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Ribbon").focus();
        h.run_steps(2);
        assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Ribbon").is_focused());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon, before, "{platform:?}");
        assert_eq!(h.state().ws.history.redo_len(), 1, "{platform:?}");
        assert_eq!(h.state().session.doc().unwrap(), &drawing, "{platform:?}");
        assert_eq!(h.state().session.doc().unwrap().model.len(), 1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon, reordered, "{platform:?}");
        assert_eq!(h.state().ws.history.redo_len(), 0, "{platform:?}");
        assert_eq!(h.state().session.doc().unwrap(), &drawing, "{platform:?}");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon, before, "{platform:?}");
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon, reordered, "{platform:?}");
        assert_eq!(h.state().session.doc().unwrap(), &drawing, "{platform:?}");
    }
}
