use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_engine::cmd::registry::{CommandStatus, Target, status_on};
use cadcraft_engine::doc::Handle;
use cadcraft_ui_egui::ribbon::{self, contextual::active_contexts};
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::RibbonTab;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, toolbars};
use egui::{Id, accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.ui.start_tab = false;
    app
}

fn block_reference(app: &mut CadApp, name: &str) -> Handle {
    let line = app.run("line", json!({"points": [[0, 0], [1, 0]]})).unwrap()["handles"][0].clone();
    let block = app.run("block", json!({"name": name, "base": [0, 0], "handles": [line]})).unwrap();
    Handle::parse_hex(block["insert"].as_str().unwrap()).unwrap()
}

fn block_tab(app: &CadApp) -> RibbonTab {
    ribbon::visible_tabs(app).into_iter().find(|tab| tab.id == "ctx_block").unwrap()
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut harness = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(|ui, app: &mut CadApp| ribbon::render::show(app, ui), app);
    harness.run_steps(3);
    harness
}

#[test]
fn selecting_a_block_shows_the_block_tab() {
    let _guard = test_lock();
    let mut app = attached();
    let handle = block_reference(&mut app, "A");
    assert!(!ribbon::visible_tabs(&app).iter().any(|tab| tab.id == "ctx_block"));
    let mut harness = harness(app);
    harness.state_mut().session.set_selection(vec![handle]);
    harness.run_steps(3);
    assert_eq!(active_contexts(&harness.state().session), ["block"]);
    let block_button = harness.get_by_role_and_label(Role::Button, "Block");
    assert!(!block_button.accesskit_node().is_disabled());
    let block_tab_id = block_button.accesskit_node().locate().0;
    assert_eq!(harness.ctx.data(|data| data.get_temp::<String>(Id::new("cc_ribbon_active"))).as_deref(), Some("ctx_block"));
    harness.get_by_role_and_label(Role::Button, "Replace Block");
    harness.get_by_role_and_label(Role::Button, "Home").click();
    harness.run_steps(3);
    assert_eq!(harness.ctx.data(|data| data.get_temp::<String>(Id::new("cc_ribbon_active"))).as_deref(), Some("home"));
    let line = harness.state_mut().run("line", json!({"points": [[2, 0], [3, 0]]})).unwrap();
    let line = Handle::parse_hex(line["handles"][0].as_str().unwrap()).unwrap();
    harness.state_mut().session.set_selection(vec![handle, line]);
    harness.run_steps(3);
    assert!(active_contexts(&harness.state().session).is_empty());
    assert!(!ribbon::visible_tabs(harness.state()).iter().any(|tab| tab.id == "ctx_block"));
    assert!(harness.query_all_by_role_and_label(Role::Button, "Block").all(|node| node.accesskit_node().locate().0 != block_tab_id));
    harness.state_mut().session.set_selection(vec![handle]);
    harness.run_steps(3);
    assert_eq!(harness.ctx.data(|data| data.get_temp::<String>(Id::new("cc_ribbon_active"))).as_deref(), Some("ctx_block"));
    harness.state_mut().session.set_selection(Vec::new());
    harness.run_steps(3);
    assert!(active_contexts(&harness.state().session).is_empty());
    assert!(harness.query_all_by_role_and_label(Role::Button, "Block").all(|node| node.accesskit_node().locate().0 != block_tab_id));
}

#[test]
fn u_bot_block_is_handled_like_any_block() {
    let _guard = test_lock();
    let mut app = attached();
    let normal = block_reference(&mut app, "A");
    let u_bot = block_reference(&mut app, "U-BOT");
    app.session.set_selection(vec![normal]);
    let expected = block_tab(&app);
    app.session.set_selection(vec![u_bot]);
    assert_eq!(active_contexts(&app.session), ["block"]);
    assert_eq!(block_tab(&app), expected);
    let mut harness = harness(app);
    harness.get_by_role_and_label(Role::Button, "Replace Block");
    assert_eq!(harness.ctx.data(|data| data.get_temp::<String>(Id::new("cc_ribbon_active"))).as_deref(), Some("ctx_block"));
    harness.state_mut().session.set_selection(vec![normal, u_bot]);
    harness.run_steps(3);
    assert_eq!(block_tab(harness.state()), expected);
}

#[test]
fn every_block_tab_button_is_a_real_command() {
    let _guard = test_lock();
    for id in PRESET_IDS {
        let ribbon = preset(id).unwrap().layout.ribbon;
        if id == &"SIMPLE" {
            assert!(ribbon.tabs.is_empty());
            continue;
        }
        let tab = ribbon.tabs.iter().find(|tab| tab.id == "ctx_block").unwrap();
        assert_eq!(tab.contextual.as_deref(), Some("block"));
        assert_eq!(tab.label, "Block");
        assert!(tab.visible);
        let groups: Vec<_> =
            tab.groups.iter().map(|group| (group.label.as_str(), group.commands.iter().map(String::as_str).collect::<Vec<_>>())).collect();
        assert_eq!(
            groups,
            [
                ("Edit", vec!["bedit", "refedit", "explode", "attsync", "battman"]),
                ("Count", vec!["count", "countlist"]),
                ("Replace", vec!["blockreplace"]),
                ("Check", vec!["list", "selectsimilar", "audit"]),
            ]
        );
        for command in tab.groups.iter().flat_map(|group| &group.commands) {
            assert!(toolbars::is_known(command), "{id}: {command}");
            assert!(cadcraft_engine::find_command(command).is_some(), "{id}: {command}");
            assert_eq!(status_on(command, Target::Desktop), CommandStatus::Available, "{id}: {command}");
        }
    }
}
