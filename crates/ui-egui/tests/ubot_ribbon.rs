//! The U-BOT tab of the UbotBuild workspace: real commands only, translated labels, and profiles
//! saved before the tab existed.

use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_engine::cmd::registry::{CommandStatus, Target, status_on};
use cadcraft_ui_egui::i18n::{self, tr, tr_ribbon};
use cadcraft_ui_egui::ribbon;
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::RibbonTab;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, toolbars};
use egui::{accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const GROUPS: [(&str, &str, [&str; 2]); 4] = [
    ("layout", "Box Layout", ["ubfill", "ubzone"]),
    ("replace", "Replace Boxes", ["ubreplace", "tubab"]),
    ("quantity", "Box Quantities", ["ubcount", "ublegend"]),
    ("check", "Check", ["ubcheck", "ublib"]),
];

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.ui.start_tab = false;
    app
}

fn ubotbuild() -> CadApp {
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "UBOTBUILD"})).unwrap();
    app
}

fn ubot_tab() -> RibbonTab {
    preset("UBOTBUILD").unwrap().layout.ribbon.tabs.into_iter().find(|tab| tab.id == "ubot").unwrap()
}

fn standard_tab_ids(app: &CadApp) -> Vec<String> {
    ribbon::visible_tabs(app).into_iter().filter(|tab| tab.contextual.is_none()).map(|tab| tab.id).collect()
}

fn harness(app: CadApp, language: &'static str) -> Harness<'static, CadApp> {
    let mut harness = Harness::builder()
        .with_size(vec2(1366.0, 768.0))
        .build_ui_state(move |ui, app: &mut CadApp| i18n::with_language(language, || ribbon::render::show(app, ui)), app);
    harness.run_steps(3);
    harness
}

#[test]
fn ubot_tab_lists_only_registered_commands() {
    let _guard = test_lock();
    let tab = ubot_tab();
    assert_eq!((tab.label.as_str(), tab.order, tab.visible, tab.contextual.as_deref()), ("U-BOT", 1, true, None));
    let groups: Vec<_> = tab
        .groups
        .iter()
        .map(|group| (group.id.as_str(), group.label.as_str(), group.commands.iter().map(String::as_str).collect::<Vec<_>>()))
        .collect();
    assert_eq!(groups, GROUPS.map(|(id, label, commands)| (id, label, commands.to_vec())));
    assert!(tab.groups.iter().all(|group| group.large == Some(2)), "every U-BOT button is a large tile");
    for command in tab.groups.iter().flat_map(|group| &group.commands) {
        assert!(toolbars::is_known(command), "{command}");
        assert!(cadcraft_engine::find_command(command).is_some(), "{command}");
        assert_eq!(status_on(command, Target::Desktop), CommandStatus::Available, "{command}");
    }
    for id in PRESET_IDS.iter().filter(|id| **id != "UBOTBUILD") {
        assert!(preset(id).unwrap().layout.ribbon.tabs.iter().all(|tab| tab.id != "ubot"), "{id}");
    }

    let app = ubotbuild();
    assert_eq!(
        standard_tab_ids(&app),
        ["home", "ubot", "draw", "modify", "annotate", "layers", "insert", "properties", "view", "layout", "output", "tools"]
    );
    let mut harness = harness(app, "en");
    harness.get_by_role_and_label(Role::Tab, "U-BOT").click();
    harness.run_steps(3);
    for (_, group, commands) in GROUPS {
        harness.get_by_label(group);
        for command in commands {
            let label = cadcraft_engine::find_command(command).unwrap().label;
            assert!(harness.query_all_by_role_and_label(Role::Button, label).next().is_some(), "{command}: {label}");
        }
    }
}

#[test]
fn ubot_tab_labels_translated_vi_uk() {
    let _guard = test_lock();
    let tab = ubot_tab();
    for (language, expected) in
        [("vi", ["U-BOT", "Bố trí", "Thay hộp", "Thống kê", "Kiểm tra"]), ("uk", ["U-BOT", "Розкладка", "Замінити бокси", "Обсяги", "Перевірка"])]
    {
        let labels: Vec<&str> =
            std::iter::once(tr_ribbon(language, &tab.label)).chain(tab.groups.iter().map(|group| tr_ribbon(language, &group.label))).collect();
        assert_eq!(labels, expected, "{language}");
        for command in tab.groups.iter().flat_map(|group| &group.commands) {
            let label = cadcraft_engine::find_command(command).unwrap().label;
            assert_ne!(tr(language, label), label, "{language}: {command}");
        }
    }

    let mut harness = harness(ubotbuild(), "vi");
    harness.get_by_role_and_label(Role::Tab, "U-BOT").click();
    harness.run_steps(3);
    for group in ["Bố trí", "Thay hộp", "Thống kê", "Kiểm tra"] {
        harness.get_by_label(group);
    }
    let fill = harness.query_all_by_role_and_label(Role::Button, "Rải hộp U-BOT").next();
    assert!(fill.is_some_and(|button| !button.accesskit_node().is_disabled()));
}

#[test]
fn ubot_tab_reaches_a_profile_saved_before_it() {
    let _guard = test_lock();
    let mut app = ubotbuild();
    app.run("ui.ribbon.tab.visible", json!({"tab": "view", "visible": false})).unwrap();
    app.commit_workspace_now().unwrap();
    let saved = app.ws.store.as_deref().unwrap().read_current().unwrap();
    // A build without the tab writes the same patch: it names only the tabs the user changed.
    let value: Value = serde_json::from_str(&saved).unwrap();
    assert!(!value.to_string().contains("\"ubot\""), "the stored patch must not carry the preset's tab list");

    let restarted = attached(MemoryStore { current: Some(saved), ..MemoryStore::default() });
    let ids = standard_tab_ids(&restarted);
    assert!(!ids.iter().any(|id| id == "view"), "the user's own change is kept");
    assert_eq!(ids.get(..2), Some(&["home".to_string(), "ubot".to_string()][..]));
}
