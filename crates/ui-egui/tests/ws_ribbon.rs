//! W8a shortcuts, aliases, Quick Access and favorites.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::quick_access;
use cadcraft_ui_egui::shortcuts::{self, BindError};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, KeyboardShortcut, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

#[test]
fn shortcut_defaults_unchanged() {
    let _guard = test_lock();
    let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let pairs: &[(KeyboardShortcut, &str)] = &[
        (sc(cmd_shift, Key::A), "pickstyle"),
        (sc(cmd_shift, Key::Z), "redo"),
        (sc(cmd, Key::Z), "u"),
        (sc(cmd, Key::Y), "redo"),
        (sc(cmd, Key::N), "new"),
        (sc(cmd, Key::O), "ui.open"),
        (sc(cmd_shift, Key::S), "ui.saveas"),
        (sc(cmd, Key::S), "qsave"),
        (sc(cmd, Key::P), "plot"),
        (sc(cmd, Key::A), "selectall"),
        (sc(cmd_shift, Key::C), "copybase"),
        (sc(cmd, Key::C), "copyclip"),
        (sc(cmd, Key::X), "cutclip"),
        (sc(cmd_shift, Key::V), "pasteblock"),
        (sc(cmd, Key::V), "pasteclip"),
        (sc(cmd, Key::Num1), "ui.panel.properties"),
        (sc(cmd, Key::Num3), "ui.toggle.toolsets"),
        (sc(cmd, Key::Num9), "ui.toggle.cmdline"),
        (sc(Modifiers::NONE, Key::F1), "ui.dialog.commands"),
        (sc(Modifiers::NONE, Key::F2), "ui.toggle.history"),
        (sc(Modifiers::NONE, Key::F3), "osnap"),
        (sc(Modifiers::NONE, Key::F5), "isoplane"),
        (sc(cmd, Key::E), "isoplane"),
        (sc(Modifiers::NONE, Key::F7), "grid"),
        (sc(Modifiers::NONE, Key::F8), "ortho"),
        (sc(Modifiers::NONE, Key::F9), "snap"),
        (sc(Modifiers::NONE, Key::F10), "polar"),
        (sc(Modifiers::NONE, Key::F11), "otrack"),
        (sc(Modifiers::NONE, Key::F12), "dynmode"),
    ];
    let expected: BTreeMap<String, String> = pairs.iter().map(|(key, id)| (format!("{key:?}"), (*id).to_string())).collect();
    let actual: BTreeMap<String, String> = shortcuts::defaults()
        .iter()
        .flat_map(|(id, keys)| {
            keys.split(',').map(move |key| {
                let combo = shortcuts::parse(key.trim()).unwrap();
                (format!("{:?}", shortcuts::keyboard_shortcut(&combo)), id.clone())
            })
        })
        .collect();
    assert_eq!(actual, expected);
}

// The engine's user-alias store is process-wide, so these app restarts must not overlap.
fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app
}

fn restarted(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), policy: old.read_policy(), snapshot_map, ..MemoryStore::default() })
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn running(app: &CadApp) -> Option<&str> {
    app.session.running.as_ref().map(|command| command.id.as_str())
}

#[test]
fn bind_rejects_reserved_and_reports_conflict() {
    let _guard = test_lock();
    let mut map = shortcuts::defaults();
    let before = map.clone();
    for key in ["Escape", "Esc", "Ctrl+Q", "Cmd+Q"] {
        assert_eq!(shortcuts::bind(&mut map, "line", key, false), Err(BindError::Reserved));
        assert_eq!(map, before);
    }
    assert_eq!(shortcuts::bind(&mut map, "line", "Ctrl+S", false), Err(BindError::Conflict { other: "qsave".into() }));
    assert_eq!(map, before);
    shortcuts::bind(&mut map, "line", "Ctrl+S", true).unwrap();
    assert_eq!(map.get("line").map(String::as_str), Some("Ctrl+S"));
    assert!(!map.contains_key("qsave"));
    assert_eq!(shortcuts::bind(&mut map, "line", "Ctrl+not-a-key", false), Err(BindError::Unparsable));
    let before = map.clone();
    for key in ["L", "1", "Delete", "Tab", "Space", "F13"] {
        assert_eq!(shortcuts::bind(&mut map, "line", key, false), Err(BindError::Unparsable), "{key}");
        assert_eq!(map, before);
    }
    for key in ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Shift+L", "Alt+L"] {
        let mut map = BTreeMap::new();
        shortcuts::bind(&mut map, "line", key, false).unwrap();
        assert_eq!(map.get("line").map(String::as_str), Some(key));
    }
    assert_eq!(shortcuts::format(&shortcuts::parse("Cmd+Shift+L").unwrap()), "Ctrl+Shift+L");
    assert_eq!(BindError::Reserved.to_string(), "Reserved key");
    assert_eq!(BindError::Conflict { other: "qsave".into() }.to_string(), "Already used by qsave");
    assert_eq!(BindError::Unparsable.to_string(), "Not a key combination");
}

#[test]
fn rebinding_persists_and_works() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.shortcut.bind", json!({"command": "line", "key": "Ctrl+Shift+L"})).unwrap();
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.shortcuts.get("line").map(String::as_str), Some("Ctrl+Shift+L"));
    let mut h = harness(restored);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::L);
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
}

#[test]
fn aliases_persist() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("aliasedit", json!({"set": {"LL": "line"}})).unwrap();
    assert_eq!(app.ws.user.layout.aliases.get("ll").map(String::as_str), Some("line"));
    assert!(!app.session.state().unwrap().is_dirty());
    cadcraft_engine::cmd::file::pgp::set_aliases(&[]);
    assert!(cadcraft_engine::cmd::file::pgp::aliases().is_empty());
    let mut restored = restarted(&mut app);
    assert_eq!(cadcraft_engine::cmd::file::pgp::aliases(), vec![("ll".to_string(), "line".to_string())]);
    assert!(restored.run("aliasedit", json!({"set": {"line": "circle"}})).is_err());
    assert_eq!(cadcraft_engine::cmd::resolve_alias("line"), "line");
    assert!(restored.cmdline("LL").is_none());
    assert_eq!(running(&restored), Some("line"));
}

#[test]
fn quick_access_pin_unpin_reorder() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    assert_eq!(app.ws.effective.layout.quick_access, ["new", "open", "qsave", "saveas", "plot", "u", "redo"]);
    for id in &app.ws.effective.layout.quick_access {
        assert!(cadcraft_engine::find_command(id).is_some(), "{id}");
    }
    app.run("ui.quickaccess.pin", json!({"id": "line"})).unwrap();
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.quickaccess.pin", json!({"id": "line"})).unwrap()["changed"], json!(false));
    assert_eq!(app.ws.history.undo_len(), steps);
    app.run("ui.quickaccess.move", json!({"id": "line", "to": 0})).unwrap();
    assert_eq!(app.ws.effective.layout.quick_access.first().map(String::as_str), Some("line"));
    app.run("ui.quickaccess.unpin", json!({"id": "open"})).unwrap();
    assert!(!app.ws.effective.layout.quick_access.iter().any(|id| id == "open"));
    let mut restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.quick_access, app.ws.effective.layout.quick_access);
    assert!(!restored.session.state().unwrap().is_dirty());
    restored.run("ui.toggle.menubar", json!({"on": true})).unwrap();
    restored.run("ui.toggle.toolsets", json!({"on": false})).unwrap();
    restored.run("ui.ribbon.collapse", json!({"collapsed": true})).unwrap();
    let mut h = harness(restored);
    h.get_by_label("Line").click();
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
}

#[test]
fn favorites_rank_first_in_search() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let baseline: Vec<_> = quick_access::search(&app, "atch").into_iter().map(|entry| entry.id).collect();
    assert!(baseline.iter().any(|id| id == "HATCHEDIT"));
    assert_ne!(baseline.first().map(String::as_str), Some("HATCHEDIT"));
    app.run("ui.favorite.add", json!({"id": "hatchedit"})).unwrap();
    app.run("ui.favorite.add", json!({"id": "hatchedit"})).unwrap();
    assert_eq!(app.ws.user.layout.favorites, ["hatchedit"]);
    let ranked: Vec<_> = quick_access::search(&app, "atch").into_iter().map(|entry| entry.id).collect();
    assert_eq!(ranked.first().map(String::as_str), Some("HATCHEDIT"));
    assert_ne!(ranked, baseline);
    assert!(quick_access::search(&app, "workspace").iter().any(|entry| entry.id.starts_with("UI.WORKSPACE.")), "UI commands are searchable");
    let mut restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.favorites, ["hatchedit"]);
    assert_eq!(quick_access::search(&restored, "atch").into_iter().map(|entry| entry.id).collect::<Vec<_>>(), ranked);
    restored.run("ui.favorite.remove", json!({"id": "hatchedit"})).unwrap();
    assert_eq!(quick_access::search(&restored, "atch").into_iter().map(|entry| entry.id).collect::<Vec<_>>(), baseline);
    assert!(restored.ws.effective.layout.favorites.is_empty());
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn unbinding_and_replacing_defaults_survive_restart() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.shortcut.bind", json!({"command": "line", "key": "Ctrl+S", "replace": true})).unwrap();
    let restored = restarted(&mut app);
    assert_eq!(shortcuts::active(&restored).get("qsave").map(String::as_str), Some(""));
    let mut h = harness(restored);
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
    h.key_press(Key::Escape);
    h.run_steps(3);
    h.state_mut().run("ui.shortcut.unbind", json!({"command": "line"})).unwrap();
    let restored = restarted(h.state_mut());
    let mut h = harness(restored);
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    assert_eq!(running(h.state()), None);
    assert!(h.state().ui.dialog.is_none());
}

#[test]
fn profile_edits_obey_policy_and_layout_undo() {
    let _guard = test_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/shortcuts/qsave", "/quick_access", "/favorites", "/aliases"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    for (command, params) in [
        ("ui.shortcut.bind", json!({"command": "line", "key": "Ctrl+S", "replace": true})),
        ("ui.shortcut.unbind", json!({"command": "qsave"})),
        ("ui.quickaccess.pin", json!({"id": "line"})),
        ("ui.favorite.add", json!({"id": "hatch"})),
        ("aliasedit", json!({"set": {"LL": "line"}})),
    ] {
        assert!(app.run(command, params).unwrap_err().contains("Locked by company policy"), "{command}");
    }
    assert!(cadcraft_engine::cmd::file::pgp::aliases().is_empty());
    assert!(app.ws.user.layout.aliases.is_empty());
    let before = app.ws.user.layout.clone();
    app.run("ui.shortcut.bind", json!({"command": "line", "key": "Ctrl+Shift+L"})).unwrap();
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout.shortcuts.get("line").map(String::as_str), Some("Ctrl+Shift+L"));
    assert!(!app.session.state().unwrap().is_dirty());
}

#[test]
fn quick_access_unknown_id_remains_a_placeholder() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
    app.run("ui.quickaccess.pin", json!({"id": "sheetset"})).unwrap();
    app.run("ui.quickaccess.move", json!({"id": "sheetset", "to": 0})).unwrap();
    let restored = restarted(&mut app);
    let h = harness(restored);
    assert!(h.get_by_label("Sheet Set — not available in this version").accesskit_node().is_disabled());
    assert_eq!(h.state().ws.effective.layout.quick_access.first().map(String::as_str), Some("sheetset"));
}

#[test]
fn shortcut_bind_rejects_unknown_commands() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let before = app.ws.user.layout.clone();
    for key in ["Ctrl+Shift+L", "Escape"] {
        assert_eq!(app.run("ui.shortcut.bind", json!({"command": "not.a.command", "key": key})).unwrap_err(), "Unknown command: not.a.command");
        assert_eq!(app.ws.user.layout, before);
    }
    assert_eq!(app.run("ui.shortcut.bind", json!({"command": "line", "key": "L"})).unwrap_err(), "Not a key combination");
    assert_eq!(app.ws.user.layout, before);
    assert_eq!(app.ws.history.undo_len(), 0);
}

#[test]
fn imported_reserved_shortcuts_are_ignored() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let mut profile = app.ws.user.clone();
    profile.layout.shortcuts = [
        ("line".into(), "Escape".into()),
        ("circle".into(), "Ctrl+Q".into()),
        ("arc".into(), "Esc, Ctrl+Shift+L".into()),
        ("pline".into(), "Cmd+Q".into()),
    ]
    .into_iter()
    .collect();
    app.run("ui.workspace.import", json!({"text": serde_json::to_string(&profile).unwrap()})).unwrap();
    for id in ["line", "circle", "pline"] {
        assert!(!app.ws.effective.layout.shortcuts.contains_key(id));
        assert!(!shortcuts::active(&app).contains_key(id));
    }
    assert_eq!(app.ws.effective.layout.shortcuts.get("arc").map(String::as_str), Some("Ctrl+Shift+L"));
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.shortcuts, app.ws.effective.layout.shortcuts);
    let mut h = harness(restored);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::L);
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("arc"));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(running(h.state()), None);
}

#[test]
fn empty_quick_access_draws_nothing() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    assert!(app.ws.effective.layout.quick_access.is_empty());
    let mut h = Harness::builder().build_ui_state(|ui, app: &mut CadApp| quick_access::show(app, ui), app);
    h.run_steps(3);
    for label in ["Search commands...", "Workspace: Simple", "Open (Ctrl+O)", "More Quick Access commands"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn quick_access_labels_include_shortcuts() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h = Harness::builder().build_ui_state(|ui, app: &mut CadApp| quick_access::show(app, ui), app);
    h.run_steps(3);
    assert!(h.query_by_label("Save").is_none());
    assert!(h.query_by_label("Save (Ctrl+S)").is_some() || h.query_by_label("Save (⌘S)").is_some());
}

#[test]
fn custom_tab_and_group_order_survive_restart_ux03() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let added = app.run("ui.ribbon.tab.add", json!({"label": "My Tab"})).unwrap();
    let id = added["id"].as_str().unwrap();
    app.run("ui.ribbon.group.move", json!({"tab": "home", "group": "modify", "to": 0})).unwrap();
    app.run("ui.ribbon.tab.move", json!({"tab": id, "to": 1})).unwrap();
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.ribbon, app.ws.effective.layout.ribbon);
    let tabs = &restored.ws.effective.layout.ribbon.tabs;
    assert_eq!(tabs.iter().find(|tab| tab.id == id).unwrap().label, "My Tab");
    assert_eq!(tabs.iter().find(|tab| tab.id == "home").unwrap().groups.first().unwrap().id, "modify");
    assert_eq!(cadcraft_ui_egui::ribbon::visible_tabs(&restored).get(1).unwrap().id, id);
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn contextual_dimension_tab() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let result = app.run("dimlinear", json!({"p1": [0, 0], "p2": [10, 0], "at": [5, 3]})).unwrap();
    let handle = cadcraft_engine::doc::Handle::parse_hex(result["handle"].as_str().unwrap()).unwrap();
    app.session.set_selection(vec![handle]);
    assert_eq!(cadcraft_ui_egui::ribbon::contextual::active_contexts(&app.session), ["dimension"]);
    let mut h =
        Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::ribbon::render::show(app, ui), app);
    h.run_steps(3);
    assert!(h.query_by_label("Dimension").is_some());
    h.state_mut().session.set_selection(Vec::new());
    h.run_steps(3);
    assert!(cadcraft_ui_egui::ribbon::contextual::active_contexts(&h.state().session).is_empty());
    assert!(h.query_by_label("Dimension").is_none());
    let line = h.state_mut().run("line", json!({"points": [[0, 0], [2, 2]]})).unwrap();
    let line_handle = cadcraft_engine::doc::Handle::parse_hex(line["handles"][0].as_str().unwrap()).unwrap();
    h.state_mut().session.set_selection(vec![handle, line_handle]);
    h.run_steps(3);
    assert!(h.query_by_label("Dimension").is_none(), "mixed selections do not open a context");
}

#[test]
fn ribbon_buttons_run_real_commands() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    for id in app.ws.effective.layout.ribbon.tabs.iter().flat_map(|tab| &tab.groups).flat_map(|group| &group.commands) {
        assert!(cadcraft_engine::find_command(id).is_some() || cadcraft_ui_egui::menus::is_ui_command(id), "{id}");
    }
    let label = cadcraft_engine::find_command("line").unwrap().label;
    let mut h = harness(app);
    h.get_by_label(label).click();
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
}

fn keyboard_activate(h: &mut Harness<'_, CadApp>, label: &str, reverse: bool) {
    for _ in 0..180 {
        if h.query_all_by_label(label).any(|node| node.is_focused()) {
            h.key_press(Key::Enter);
            h.run_steps(3);
            return;
        }
        if reverse {
            h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        } else {
            h.key_press(Key::Tab);
        }
        h.run_steps(2);
    }
    panic!("keyboard could not reach {label}");
}

fn customize_harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            if app.ui.dialog.as_deref() == Some(cadcraft_ui_egui::customize::DIALOG) {
                let mut open = true;
                cadcraft_ui_egui::customize::dialog(app, ui.ctx(), &mut open);
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
fn customize_keyboard_only_ux14() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let preset = app.ws.effective.layout.clone();
    app.run("ui.shortcut.bind", json!({"command": "ui.customize.open", "key": "Ctrl+Shift+F12"})).unwrap();
    let mut h = customize_harness(app);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F12);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(cadcraft_ui_egui::customize::DIALOG));
    keyboard_activate(&mut h, "Add Tab", false);
    assert!(h.state().ws.effective.layout.ribbon.tabs.iter().any(|tab| tab.label == "New Tab"));
    keyboard_activate(&mut h, "Home", true);
    let before =
        h.query_all_by_role(egui::accesskit::Role::Button).find(|node| node.is_focused()).unwrap().accesskit_node().label().unwrap().to_string();
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    let after =
        h.query_all_by_role(egui::accesskit::Role::Button).find(|node| node.is_focused()).unwrap().accesskit_node().label().unwrap().to_string();
    assert_ne!(after, before, "Down moves focus to another row");
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert!(h.get_by_label(&before).is_focused(), "Up returns focus to the previous row");
    keyboard_activate(&mut h, "Group Down", false);
    assert_eq!(h.state().ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().groups.first().unwrap().id, "modify");
    keyboard_activate(&mut h, "Reset", true);
    assert_eq!(h.state().ws.effective.layout, preset);
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_edits_obey_policy_and_layout_undo() {
    let _guard = test_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/ribbon"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    assert!(app.run("ui.ribbon.tab.add", json!({"label": "Blocked"})).unwrap_err().contains("Locked by company policy"));
    assert_eq!(app.ws.history.undo_len(), 0);
    let mut app = attached(MemoryStore::default());
    let before = app.ws.effective.layout.ribbon.clone();
    app.run("ui.ribbon.tab.add", json!({"label": "Allowed"})).unwrap();
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert!(app.ws.effective.layout.ribbon.tabs.iter().any(|tab| tab.label == "Allowed"));
}

#[test]
fn ribbon_collapses_groups_and_scrolls_at_narrow_widths() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(480.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::ribbon::render::show(app, ui), app);
    h.run_steps(3);
    assert!(h.query_by_label("Next ribbon groups").is_some(), "collapsed groups still have a scroll control");
    h.get_by_label("Draw").click();
    h.run_steps(3);
    h.get_by_label(cadcraft_engine::find_command("line").unwrap().label).click();
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
}

#[test]
fn ribbon_context_menu_pins_and_removes_through_layout_commands() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::ribbon::render::show(app, ui), app);
    h.run_steps(3);
    let label = cadcraft_engine::find_command("line").unwrap().label;
    h.get_by_label(label).click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Add to Quick Access").click();
    h.run_steps(3);
    assert!(h.state().ws.effective.layout.quick_access.iter().any(|id| id == "line"));
    h.get_by_label(label).click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Remove from Ribbon").click();
    h.run_steps(3);
    assert!(
        !h.state()
            .ws
            .effective
            .layout
            .ribbon
            .tabs
            .iter()
            .find(|tab| tab.id == "home")
            .unwrap()
            .groups
            .first()
            .unwrap()
            .commands
            .iter()
            .any(|id| id == "line")
    );
    assert_eq!(running(h.state()), None);
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_home_groups_fit_or_scroll_at_1366() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let commands: Vec<_> = app
        .ws
        .effective
        .layout
        .ribbon
        .tabs
        .iter()
        .find(|tab| tab.id == "home")
        .unwrap()
        .groups
        .iter()
        .flat_map(|group| group.commands.clone())
        .collect();
    let panel_id = egui::Id::new("ribbon_test_rect");
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        move |ui, app: &mut CadApp| {
            let before = ui.available_rect_before_wrap();
            cadcraft_ui_egui::ribbon::render::show(app, ui);
            let bottom = ui.available_rect_before_wrap().top();
            ui.ctx().data_mut(|data| data.insert_temp(panel_id, egui::Rect::from_min_max(before.min, egui::pos2(before.right(), bottom))));
        },
        app,
    );
    h.run_steps(3);
    let panel = h.ctx.data(|data| data.get_temp::<egui::Rect>(panel_id)).unwrap();
    assert_eq!(panel.height(), 142.0);
    let scroll = h.query_by_label("Previous ribbon groups").is_some() && h.query_by_label("Next ribbon groups").is_some();
    let buttons: Vec<_> =
        h.query_all_by_role(egui::accesskit::Role::Button).map(|node| (node.accesskit_node().label().unwrap().to_string(), node.rect())).collect();
    for (index, (label, rect)) in buttons.iter().enumerate() {
        assert!(panel.contains_rect(*rect), "{label} is outside the ribbon: {rect:?} vs {panel:?}");
        for (other_label, other_rect) in buttons.iter().skip(index + 1) {
            assert!(!rect.intersects(*other_rect), "{label} overlaps {other_label}: {rect:?} vs {other_rect:?}");
        }
    }
    for id in commands {
        let label = cadcraft_engine::find_command(&id).unwrap().label;
        assert!(h.query_all_by_label(label).any(|node| panel.contains_rect(node.rect())) || scroll, "{id} is neither reachable nor scrollable");
    }
}

#[test]
fn ribbon_edit_keeps_other_tab_orders() {
    let _guard = test_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/ribbon/tabs/ctx_dimension"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let orders: BTreeMap<_, _> = app.ws.effective.layout.ribbon.tabs.iter().map(|tab| (tab.id.clone(), tab.order)).collect();
    assert_eq!(orders.get("ctx_dimension"), Some(&100));
    assert_eq!(orders.get("ctx_hatch"), Some(&101));
    app.run("ui.ribbon.collapse", json!({"collapsed": true})).unwrap();
    app.run("ui.ribbon.tab.rename", json!({"tab": "home", "label": "My Home"})).unwrap();
    for tab in &app.ws.effective.layout.ribbon.tabs {
        assert_eq!(Some(&tab.order), orders.get(&tab.id), "{} was renumbered", tab.id);
    }
    assert_eq!(app.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().label, "My Home");
    let added = app.run("ui.ribbon.tab.add", json!({"label": "Policy-safe tab"})).unwrap();
    let id = added["id"].as_str().unwrap();
    app.run("ui.ribbon.tab.move", json!({"tab": id, "to": 1})).unwrap();
    assert_eq!(cadcraft_ui_egui::ribbon::visible_tabs(&app).get(1).unwrap().id, id);
    for tab in app.ws.effective.layout.ribbon.tabs.iter().filter(|tab| tab.contextual.is_some() || tab.id == "home") {
        assert_eq!(Some(&tab.order), orders.get(&tab.id), "{} was renumbered by an unrelated move", tab.id);
    }
    app.run("ui.ribbon.tab.remove", json!({"tab": id})).unwrap();
    for tab in app.ws.effective.layout.ribbon.tabs.iter().filter(|tab| tab.contextual.is_some()) {
        assert_eq!(Some(&tab.order), orders.get(&tab.id), "{} was renumbered by an unrelated removal", tab.id);
    }
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.ribbon, app.ws.effective.layout.ribbon);
    assert!(!app.session.state().unwrap().is_dirty());
}

#[test]
fn minimize_ribbon_keyboard_toggle_persists() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = customize_harness(app);
    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
    for _ in 0..30 {
        if h.get_by_label("Minimize ribbon").is_focused() {
            break;
        }
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert!(h.get_by_label("Minimize ribbon").is_focused());
    h.key_press(Key::Space);
    h.run_steps(3);
    assert!(h.state().ws.effective.layout.ribbon.collapsed);
    let mut restored = restarted(h.state_mut());
    assert!(restored.ws.effective.layout.ribbon.collapsed);
    restored.run("ui.customize.open", json!({})).unwrap();
    let mut h = customize_harness(restored);
    for _ in 0..30 {
        if h.get_by_label("Minimize ribbon").is_focused() {
            break;
        }
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert!(h.get_by_label("Minimize ribbon").is_focused());
    h.key_press(Key::Space);
    h.run_steps(3);
    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
    assert!(!restarted(h.state_mut()).ws.effective.layout.ribbon.collapsed);
}

#[test]
fn customize_floating_toolbar_cannot_reorder_or_teleport() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.toolbar.float", json!({"id": "draw", "x_ratio": 0.3, "y_ratio": 0.4})).unwrap();
    let floating = app.ws.effective.layout.toolbars.iter().find(|bar| bar.id == "draw").unwrap().clone();
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = customize_harness(app);
    h.get_by_label("Toolbars").click();
    h.run_steps(3);
    h.get_by_label("Draw").click();
    h.run_steps(3);
    for label in ["Up", "Down"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} is unavailable for a floating toolbar");
    }
    assert!(h.query_by_label("Show").is_some());
    assert!(h.query_by_label("Hide").is_some());
    assert_eq!(h.state().ws.effective.layout.toolbars.iter().find(|bar| bar.id == "draw").unwrap(), &floating);
    h.get_by_label("Standard").click();
    h.run_steps(3);
    for label in ["Up", "Down"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "a single docked toolbar is at both list ends");
    }
}

#[test]
fn ribbon_alias_command_is_available() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.run("ui.workspace.set", json!({"pointer": "/ribbon/tabs/home/groups", "value": [{"id": "alias", "label": "Aliases", "commands": []}]}))
        .unwrap();
    app.run("ui.ribbon.button.add", json!({"tab": "home", "group": "alias", "id": "qnew"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::ribbon::render::show(app, ui), app);
    h.run_steps(3);
    assert!(!h.get_by_label("Qnew").accesskit_node().is_disabled());
    assert!(h.query_by_label("Qnew — not available in this version").is_none());
}

#[test]
fn window_minimize_ribbon_is_checkable() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::menus::menu_bar(app, ui), app);
    h.run_steps(3);
    h.get_by_label("Window").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Minimize Ribbon").click();
    h.run_steps(3);
    assert!(h.state().ws.effective.layout.ribbon.collapsed);
    h.get_by_label("Window").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Minimize Ribbon").click();
    h.run_steps(3);
    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
}
