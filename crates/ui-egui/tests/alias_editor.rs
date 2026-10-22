use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::{Session, cmd::resolve_alias};
use cadcraft_ui_egui::customize;
use cadcraft_ui_egui::customize::alias_tab::{AliasIssue, check_alias};
use cadcraft_ui_egui::wsprofile::schema::WorkspaceLayout;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn app() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
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
fn builtin_aliases_l_pl_co_run_their_commands_h07() {
    let _guard = test_lock();
    let _app = app();
    for (alias, command) in [
        ("L", "line"),
        ("PL", "pline"),
        ("C", "circle"),
        ("CO", "copy"),
        ("M", "move"),
        ("RO", "rotate"),
        ("TR", "trim"),
        ("EX", "extend"),
        ("O", "offset"),
        ("H", "hatch"),
        ("LA", "layer"),
        ("DI", "dist"),
        ("Z", "zoom"),
    ] {
        assert_eq!(resolve_alias(alias), command, "{alias}");
        assert_eq!(resolve_alias(&alias.to_ascii_lowercase()), command);
    }
}

#[test]
fn overriding_a_builtin_alias_requires_confirmation() {
    let _guard = test_lock();
    let mut app = app();
    assert!(check_alias(&app.ws.effective.layout, "L", "layer").contains(&AliasIssue::OverridesBuiltin { command: "line" }));
    let history = app.ws.history.undo_len();
    assert!(app.run("ui.alias.set", json!({"alias": "L", "command": "layer"})).unwrap_err().contains("confirm to replace"));
    assert_eq!(resolve_alias("L"), "line");
    assert_eq!(app.ws.history.undo_len(), history);
    app.run("ui.alias.set", json!({"alias": "l", "command": "layer", "confirm": true})).unwrap();
    assert_eq!(resolve_alias("L"), "layer");
    assert_eq!(app.ws.user.layout.aliases.get("L").map(String::as_str), Some("layer"));
    assert!(!app.ws.user.layout.aliases.contains_key("l"));
    app.run("ui.alias.reset", json!({})).unwrap();
    assert_eq!(resolve_alias("L"), "line");
    assert!(app.ws.user.layout.aliases.is_empty());
}

#[test]
fn alias_cannot_shadow_a_command_name() {
    let _guard = test_lock();
    let mut app = app();
    assert!(check_alias(&app.ws.effective.layout, "circle", "line").contains(&AliasIssue::IsCommandName { command: "circle" }));
    for confirm in [false, true] {
        assert!(app.run("ui.alias.set", json!({"alias": "circle", "command": "line", "confirm": confirm})).is_err());
    }
    assert_eq!(resolve_alias("circle"), "circle");
    assert!(app.ws.user.layout.aliases.is_empty());
}

#[test]
fn vietnamese_alias_is_rejected_with_a_reason() {
    let _guard = test_lock();
    let mut app = app();
    assert!(check_alias(&app.ws.effective.layout, "lớp", "layer").contains(&AliasIssue::NotAscii));
    let error = app.run("ui.alias.set", json!({"alias": "lớp", "command": "layer"})).unwrap_err();
    assert!(error.contains("letters A–Z"), "{error}");
    assert!(app.ws.user.layout.aliases.is_empty());
}

#[test]
fn aliases_export_and_import_round_trip() {
    let _guard = test_lock();
    let mut app = app();
    let aliases = json!({"LL": "line", "CC": "circle", "XX": "copy"});
    for (alias, command) in aliases.as_object().unwrap() {
        app.run("ui.alias.set", json!({"alias": alias, "command": command})).unwrap();
    }
    let exported = app.run("ui.alias.export", json!({})).unwrap();
    let text = exported["text"].as_str().unwrap();
    let mut file: Value = serde_json::from_str(text).unwrap();
    assert_eq!(file, json!({"aliases": aliases}));
    app.run("ui.alias.reset", json!({})).unwrap();
    let imported = app.run("ui.alias.import", json!({"text": text})).unwrap();
    assert_eq!(imported["imported"], 3);
    assert_eq!(imported["skipped"], 0);
    for (alias, command) in aliases.as_object().unwrap() {
        assert_eq!(resolve_alias(alias), command.as_str().unwrap());
    }
    file["aliases"]["bad alias"] = json!("line");
    app.run("ui.alias.reset", json!({})).unwrap();
    let imported = app.run("ui.alias.import", json!({"text": file.to_string()})).unwrap();
    assert_eq!(imported["imported"], 3);
    assert_eq!(imported["skipped"], 1);
    assert!(imported["message"].as_str().unwrap().contains("1 skipped"));
    assert_eq!(app.ws.user.layout.aliases.len(), 3);
}

#[test]
fn alias_page_is_keyboard_operable() {
    let _guard = test_lock();
    let mut app = app();
    app.run("ui.customize.open", json!({})).unwrap();
    let mut h = harness(app);
    for _ in 0..4 {
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Aliases").is_focused());
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Alias").focus();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Alias").type_text("LL");
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Alias command").is_focused());
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Alias command").type_text("line");
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(resolve_alias("LL"), "line");
    assert_eq!(h.state().ws.effective.layout.aliases.get("LL").map(String::as_str), Some("line"));
}

#[test]
fn alias_validation_is_strict_and_duplicate_users_require_confirmation() {
    let _guard = test_lock();
    let mut app = app();
    let layout = WorkspaceLayout::default();
    assert!(check_alias(&layout, "", "line").contains(&AliasIssue::Empty));
    assert!(check_alias(&layout, "ABCDEFGHIJKLMNOPQ", "line").contains(&AliasIssue::TooLong));
    for alias in ["has space", "a.b", " a", "a ", "a/b", "a\n"] {
        assert!(check_alias(&layout, alias, "line").contains(&AliasIssue::NotAscii), "{alias}");
    }
    app.run("ui.alias.set", json!({"alias": "ab_1-", "command": "cad.draw.line"})).unwrap();
    assert_eq!(resolve_alias("AB_1-"), "line");
    assert!(check_alias(&app.ws.effective.layout, "ab_1-", "circle").contains(&AliasIssue::DuplicateUser { command: "line".into() }));
    assert!(app.run("ui.alias.set", json!({"alias": "ab_1-", "command": "circle"})).is_err());
    app.run("ui.alias.set", json!({"alias": "ab_1-", "command": "circle", "confirm": true})).unwrap();
    assert_eq!(resolve_alias("ab_1-"), "circle");
    assert!(app.run("ui.alias.set", json!({"alias": "unknown", "command": "not_a_command"})).is_err());
    app.run("ui.alias.remove", json!({"alias": "ab_1-"})).unwrap();
    assert!(app.ws.effective.layout.aliases.is_empty());
    assert_eq!(resolve_alias("ab_1-"), "ab_1-");
}

#[test]
fn alias_import_skips_conflicts_unless_confirmed_and_rejects_hostile_data() {
    let _guard = test_lock();
    let mut app = app();
    let text = json!({"aliases": {"L": "layer", "LL": "line", "bad space": "line", "ZZZ": 12, "XYZ": "unknown"}}).to_string();
    let result = app.run("ui.alias.import", json!({"text": text})).unwrap();
    assert_eq!(result["imported"], 1);
    assert_eq!(result["skipped"], 4);
    assert_eq!(resolve_alias("L"), "line");
    let result = app.run("ui.alias.import", json!({"text": text, "confirm": true})).unwrap();
    assert_eq!(result["imported"], 2);
    assert_eq!(result["skipped"], 3);
    assert_eq!(resolve_alias("L"), "layer");
    let before = app.ws.user.layout.clone();
    for text in ["{broken".to_string(), "[]".into(), "x".repeat(256 * 1024 + 1)] {
        assert!(app.run("ui.alias.import", json!({"text": text})).is_err());
        assert_eq!(app.ws.user.layout, before);
    }
}

#[test]
fn alias_changes_persist_and_customize_undo_keeps_the_drawing() {
    let _guard = test_lock();
    let mut app = app();
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let drawing = app.session.doc().unwrap().clone();
    app.run("ui.customize.open", json!({})).unwrap();
    app.run("ui.alias.set", json!({"alias": "LL", "command": "line"})).unwrap();
    app.commit_workspace_now().unwrap();
    let stored = app.ws.store.as_deref().unwrap().read_current();
    let mut h = harness(app);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert!(h.state().ws.effective.layout.aliases.is_empty());
    assert_eq!(h.state().session.doc().unwrap(), &drawing);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run_steps(3);
    assert_eq!(resolve_alias("LL"), "line");
    assert_eq!(h.state().session.doc().unwrap(), &drawing);
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.attach_profile_store(Box::new(MemoryStore { current: stored, ..MemoryStore::default() }));
    assert_eq!(restored.ws.effective.layout.aliases.get("LL").map(String::as_str), Some("line"));
    assert_eq!(resolve_alias("LL"), "line");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn alias_export_uses_save_service_only_for_interactive_calls() {
    let _guard = test_lock();
    let mut app = app();
    app.services.pick_save = Some(Box::new(|_| panic!("programmatic export opened a picker")));
    assert!(app.run("ui.alias.export", json!({})).is_ok());
    let path = std::env::temp_dir().join(format!("cadcraft-alias-editor-{}.json", std::process::id()));
    let filename = path.to_string_lossy().into_owned();
    app.services.pick_save = Some(Box::new(move |name| {
        assert_eq!(name, "Aliases.json");
        Some(filename.clone())
    }));
    app.run("ui.alias.set", json!({"alias": "LL", "command": "line"})).unwrap();
    app.run("ui.alias.export", Value::Null).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({"aliases": {"LL": "line"}}));
    app.run("ui.alias.reset", json!({})).unwrap();
    app.run("ui.alias.import", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(resolve_alias("LL"), "line");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn command_ids() {
    for id in ["ui.alias.set", "ui.alias.remove", "ui.alias.reset", "ui.alias.export", "ui.alias.import"] {
        assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|entry| entry.0 == id), "{id}");
    }
}

#[test]
fn engine_alias_edits_and_removals_keep_profile_keys_in_sync() {
    let _guard = test_lock();
    let mut app = app();
    app.run("ui.alias.set", json!({"alias": "LL", "command": "line"})).unwrap();
    app.run("aliasedit", json!({"set": {"ll": "circle"}})).unwrap();
    assert_eq!(app.ws.user.layout.aliases.get("LL").map(String::as_str), Some("circle"));
    assert_eq!(app.ws.user.layout.aliases.len(), 1);
    assert_eq!(resolve_alias("ll"), "circle");
    app.run("aliasedit", json!({"remove": ["ll"]})).unwrap();
    assert!(app.ws.user.layout.aliases.is_empty());
    assert_eq!(resolve_alias("ll"), "ll");
}
