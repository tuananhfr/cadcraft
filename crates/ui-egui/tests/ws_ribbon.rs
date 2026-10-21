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
use egui_kittest::kittest::Queryable;
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
    assert_eq!(app.ws.effective.layout.quick_access, quick_access::DEFAULT_ITEMS);
    for id in quick_access::DEFAULT_ITEMS {
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
    let mut h = harness(restored);
    h.get_by_label("Line").click();
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("line"));
}

#[test]
fn favorites_rank_first_in_search() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.favorite.add", json!({"id": "hatch"})).unwrap();
    app.run("ui.favorite.add", json!({"id": "hatch"})).unwrap();
    assert_eq!(app.ws.user.layout.favorites, ["hatch"]);
    assert_eq!(quick_access::search(&app, "h").first().map(|entry| entry.id.as_str()), Some("HATCH"));
    assert!(quick_access::search(&app, "workspace").iter().any(|entry| entry.id.starts_with("UI.WORKSPACE.")), "UI commands are searchable");
    let mut restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.favorites, ["hatch"]);
    assert_eq!(quick_access::search(&restored, "h").first().map(|entry| entry.id.as_str()), Some("HATCH"));
    restored.run("ui.favorite.remove", json!({"id": "hatch"})).unwrap();
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
