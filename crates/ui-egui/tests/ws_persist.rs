//! Workspace persistence in the running app: commits, snapshots, startup recovery, layout undo and
//! the `ui.workspace.*` commands. Storage is a `MemoryStore`; the hosts' own stores have their tests.

use cadcraft_engine::Session;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::i18n::Preference;
use cadcraft_ui_egui::theme::ThemePref;
use cadcraft_ui_egui::wsprofile::SaveStatus;
use cadcraft_ui_egui::wsprofile::canonical::with_checksum;
use cadcraft_ui_egui::wsprofile::merge::Level;
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::{Dock, OwnerType, ProfileStatus, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::startup::{FALLBACK_MESSAGE, Fallback, RESTORED_MESSAGE, parse_set};
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, MAX_USER_PROFILES, MemoryStore, ProfileSet, ProfileStore};
use cadcraft_ui_egui::wsprofile::validate::{Mode, validate};
use cadcraft_ui_egui::{CadApp, Services};
use craft_ui::docking::Node;
use craft_ui::layout::SplitSize;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const SPEC_SAMPLE: &str = r#"{"schema_version":"1.0.0","profile_id":"uuid","revision":12,"base_id":"AUTOCAD_FAMILIAR","panels":[{"id":"properties","mode":"dock","side":"right","width_ratio":0.24}],"command_line":{"side":"bottom","height_ratio":0.15},"shortcuts":{"line":"L"}}"#;

/// User aliases live in one engine store per process; a test that sets them or counts layout undo
/// steps after an engine command holds this, or a parallel test's aliases show up as its own edit.
fn alias_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app
}

fn run(app: &mut CadApp, id: &str, params: Value) -> Value {
    app.run(id, params.clone()).unwrap_or_else(|error| panic!("{id} {params}: {error}"))
}

fn store(app: &CadApp) -> &dyn ProfileStore {
    app.ws.store.as_deref().unwrap()
}

fn current_set(app: &CadApp) -> ProfileSet {
    parse_set(&store(app).read_current().unwrap()).unwrap()
}

fn standard_toolbar_visible(profile: &WorkspaceProfile) -> bool {
    profile.layout.toolbars.iter().any(|toolbar| toolbar.id == "standard" && toolbar.visible)
}

fn user_patch(app: &CadApp) -> Value {
    app.ws.layers.iter().find(|layer| layer.level == Level::User).map(|layer| layer.patch.clone()).unwrap()
}

fn frame(app: &mut CadApp, ctx: &egui::Context, time: f64) {
    let raw = egui::RawInput { time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| app.logic(ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn app_without_store_keeps_todays_behaviour() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    assert!(app.ws.store.is_none());
    assert!(app.commit_workspace_now().is_err());
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    assert!(!app.ui.show_toolbar);
    // Layout undo works without storage too.
    run(&mut app, "ui.layout.undo", json!({}));
    assert!(app.ui.show_toolbar);
}

#[test]
fn session_layer_is_not_persisted() {
    let mut app = attached(MemoryStore::default());
    app.set_session_layer(json!({"command_line": {"visible": false}, "status_bar": {"visible": false}}));
    assert!(!app.ui.show_command_line && !app.ui.show_status_bar);
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    let user = current_set(&app).user.unwrap();
    assert!(user.layout.command_line.visible, "the session layer stays out of the stored profile");
    assert!(user.layout.status_bar.visible);
    assert!(!standard_toolbar_visible(&user), "the user's own change is stored");
    assert!(!app.ui.show_command_line, "the session layer still shows while the app runs");
}

#[test]
fn commit_writes_snapshot_and_keeps_20() {
    let mut app = attached(MemoryStore::default());
    let mut last = 0;
    for index in 0..25u64 {
        run(&mut app, "ui.cmdline.lines", json!({"lines": index % 12}));
        app.commit_workspace_now().unwrap();
        let revision = app.ws.user.revision;
        assert!(revision > last, "revision {revision} after {last}");
        last = revision;
    }
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
    let snapshots = store(&app).snapshots();
    assert_eq!(snapshots.len(), 20);
    assert_eq!(snapshots.last(), Some(&last));
    assert_eq!(current_set(&app).user.unwrap().revision, last);
    assert_eq!(store(&app).read_snapshot(last), store(&app).read_current());
}

#[test]
fn crash_recovery_ux04() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    let set = |pointer: &str, value: Value| ("ui.workspace.set", json!({"pointer": pointer, "value": value}));
    let steps: Vec<(&str, Value)> = vec![
        ("ui.toggle.toolbar", json!({"on": false})),
        ("ui.toggle.statusbar", json!({"on": false})),
        ("ui.cmdline.lines", json!({"lines": 7})),
        ("ui.dock", json!({"operation": "float", "panel": "layers", "rect": [200, 150, 320, 240]})),
        ("ui.dock", json!({"operation": "open", "panel": "externalReferences"})),
        ("ui.dock", json!({"operation": "move", "panel": "properties", "target": "toolSets", "zone": "bottom"})),
        set("/ui_scale", json!(1.25)),
        set("/shortcuts/line", json!("L")),
        set("/aliases/ln", json!("line")),
        set("/quick_access", json!(["qsave", "u"])),
        set("/favorites", json!(["circle"])),
        set("/status_bar/items/grid", json!(false)),
        set("/toolsets/tab", json!("Modeling")),
        set("/toolsets/hidden_tools", json!(["donut"])),
        set("/toolsets/collapsed_groups", json!(["Draw"])),
        set("/command_line/dock", json!("top")),
        set("/theme", json!("light")),
        set("/locale", json!("de")),
        set("/ribbon/collapsed", json!(true)),
        set("/toolbars/standard/locked", json!(true)),
    ];
    assert_eq!(steps.len(), 20);
    for (id, params) in steps {
        run(&mut app, id, params);
        app.commit_workspace_now().unwrap();
    }
    let root = app.ui.docking.layout.root.clone();
    // Killed: nothing runs on exit; a new app starts from what the store holds.
    let store = app.ws.store.take().unwrap();
    drop(app);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(store);

    assert!(app.ws.fallback.is_none());
    assert!(!app.ui.show_toolbar);
    assert!(!app.ui.show_status_bar);
    assert_eq!(app.ui.history_lines, 7);
    assert!(app.ui.docking.layout.floating.iter().any(|group| group.panels.contains(&Panel::Layers)));
    assert!(app.ui.docking.layout.contains(&Panel::ExternalRefs));
    assert_eq!(app.ui.docking.layout.root, root, "docked panels keep their places");
    let user = &app.ws.user;
    assert_eq!(user.ui_scale, 1.25);
    assert_eq!(user.layout.shortcuts.get("line").map(String::as_str), Some("L"));
    assert_eq!(user.layout.aliases.get("ln").map(String::as_str), Some("line"));
    assert_eq!(user.layout.quick_access, vec!["qsave".to_string(), "u".to_string()]);
    assert_eq!(user.layout.favorites, vec!["circle".to_string()]);
    assert_eq!(user.layout.status_bar.items.get("grid"), Some(&false));
    assert_eq!(app.ui.toolset_tab, "Modeling");
    assert_eq!(app.ui.hidden_tools, vec!["donut".to_string()]);
    assert_eq!(app.ui.collapsed_groups, vec!["Draw".to_string()]);
    assert_eq!(user.layout.command_line.dock, Dock::Top);
    assert_eq!(app.ui.theme, ThemePref::Light);
    assert_eq!(app.ui.interface_language.code(), "de");
    assert!(user.layout.ribbon.collapsed);
    assert!(user.layout.toolbars.iter().any(|toolbar| toolbar.id == "standard" && toolbar.locked && !toolbar.visible));
}

#[test]
fn corrupt_store_falls_back_ux15() {
    let shared = Shared::new(MemoryStore { current: Some("{".into()), ..MemoryStore::default() });
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    assert!(matches!(app.ws.fallback, Some(Fallback::Corrupt(_))), "{:?}", app.ws.fallback);
    assert_eq!(app.ws.user.base_id.as_deref(), Some("SIMPLE"));
    assert_eq!(app.ws.effective.layout, preset("SIMPLE").unwrap().layout);
    assert_eq!(shared.damaged(), vec![b"{".to_vec()]);
    assert!(!store(&app).snapshots().contains(&CORRUPT_SNAPSHOT), "the copy is kept beside the snapshots");
    assert_eq!(FALLBACK_MESSAGE, "Workspace could not be read; using SIMPLE. The damaged copy was kept.");
    assert_eq!(app.ws.message.as_deref(), Some(FALLBACK_MESSAGE));
    assert_eq!(app.status.as_ref().map(|(text, _)| text.as_str()), Some(FALLBACK_MESSAGE));
    assert_eq!(run(&mut app, "ui.workspace.status", json!({}))["message"], json!(FALLBACK_MESSAGE));
    // The app keeps working, and its next save keeps the damaged copy.
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    assert_eq!(shared.damaged(), vec![b"{".to_vec()]);
    assert!(!standard_toolbar_visible(&current_set(&app).user.unwrap()));

    let mut newer = preset("SIMPLE").unwrap();
    newer.schema_version = "2.0.0".into();
    let newer = serde_json::to_string(&json!({"active": "SIMPLE", "user": newer})).unwrap();
    let app = attached(MemoryStore { current: Some(newer), ..MemoryStore::default() });
    assert!(matches!(app.ws.fallback, Some(Fallback::NewerMajor(ref version)) if version == "2.0.0"), "{:?}", app.ws.fallback);

    let invalid = json!({"active": "SIMPLE", "user": {"schema_version": "1.0.0", "name": "x".repeat(2000)}}).to_string();
    let shared = Shared::new(MemoryStore { current: Some(invalid.clone()), ..MemoryStore::default() });
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    assert!(matches!(app.ws.fallback, Some(Fallback::Invalid(_))), "{:?}", app.ws.fallback);
    assert_eq!(shared.damaged(), vec![invalid.into_bytes()]);
    assert_eq!(app.ws.effective.layout, preset("SIMPLE").unwrap().layout);
}

fn three_committed_revisions() -> Shared {
    let shared = Shared::new(MemoryStore::default());
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    for (revision, lines) in [(1, 5), (2, 6), (3, 7)] {
        run(&mut app, "ui.cmdline.lines", json!({"lines": lines}));
        app.commit_workspace_now().unwrap();
        assert_eq!(app.ws.user.revision, revision);
    }
    shared
}

fn reattach(shared: &Shared) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    app
}

#[test]
fn corrupt_current_restores_last_valid_snapshot() {
    let _aliases = alias_lock();
    let shared = three_committed_revisions();
    let damaged = "{\"schema_version\":";
    shared.0.borrow_mut().current = Some(damaged.into());
    let mut app = reattach(&shared);
    assert!(matches!(app.ws.fallback, Some(Fallback::RestoredSnapshot { revision: 3, ref reason }) if !reason.is_empty()));
    assert_eq!(app.ws.user.revision, 3);
    assert_eq!(app.ws.user.layout.command_line.lines, 7);
    assert_eq!(app.ui.history_lines, 7);
    assert_eq!(shared.damaged(), vec![damaged.as_bytes().to_vec()]);
    assert_eq!(store(&app).read_current().as_deref(), Some(damaged), "startup waits for a commit before replacing current");
    let message = RESTORED_MESSAGE.replace("{revision}", "3");
    assert_eq!(app.ws.message.as_deref(), Some(message.as_str()));
    assert_eq!(app.status.as_ref().map(|(text, _)| text.as_str()), Some(message.as_str()));
    assert!(app.ws.autosave.pending(), "the restored set must replace the damaged current on the next save");
    assert_eq!(app.ws.status, SaveStatus::Unsaved);
    let status = run(&mut app, "ui.workspace.status", json!({}));
    assert_eq!(status["fallback"], json!("restored_snapshot"));
    assert_eq!(status["restored_revision"], json!(3));
}

#[test]
fn corrupt_snapshot_is_skipped_for_an_older_one() {
    let _aliases = alias_lock();
    let shared = three_committed_revisions();
    {
        let mut store = shared.0.borrow_mut();
        store.current = Some("{\"schema_version\":".into());
        store.snapshot_map.insert(3, "{".into());
    }
    let app = reattach(&shared);
    assert!(matches!(app.ws.fallback, Some(Fallback::RestoredSnapshot { revision: 2, .. })));
    assert_eq!(app.ws.user.revision, 2);
    assert_eq!(app.ws.user.layout.command_line.lines, 6);
    assert_eq!(app.ui.history_lines, 6);
    assert!(app.ws.message.as_ref().is_some_and(|message| message.contains("revision 2")));
    assert_eq!(store(&app).read_snapshot(3).as_deref(), Some("{"), "startup keeps the unusable snapshot");
}

#[test]
fn newer_major_still_falls_back_to_simple() {
    let _aliases = alias_lock();
    let shared = three_committed_revisions();
    let mut newer: Value = serde_json::from_str(&shared.read_current().unwrap()).unwrap();
    newer["schema_version"] = json!("2.0.0");
    // A genuine newer writer signs its set; an unsigned edit would read as corruption instead.
    let damaged = with_checksum(newer).to_string();
    shared.0.borrow_mut().current = Some(damaged.clone());
    let app = reattach(&shared);
    assert_eq!(app.ws.fallback, Some(Fallback::NewerMajor("2.0.0".into())));
    assert_eq!(app.ws.user.base_id.as_deref(), Some("SIMPLE"));
    assert_eq!(app.ws.effective.layout, preset("SIMPLE").unwrap().layout);
    assert_eq!(shared.damaged(), vec![damaged.into_bytes()]);
    assert!(!app.ws.autosave.pending());
    assert_eq!(app.workspace_status()["fallback"], json!("newer_major"));
    assert_eq!(app.workspace_status()["restored_revision"], Value::Null);
}

#[test]
fn restored_snapshot_is_rewritten_on_next_commit() {
    let _aliases = alias_lock();
    let shared = three_committed_revisions();
    let damaged = "{\"schema_version\":";
    shared.0.borrow_mut().current = Some(damaged.into());
    let mut app = reattach(&shared);
    assert!(matches!(app.ws.fallback, Some(Fallback::RestoredSnapshot { revision: 3, .. })));
    assert!(app.ws.autosave.pending());
    run(&mut app, "ui.cmdline.lines", json!({"lines": 8}));
    app.commit_workspace_now().unwrap();
    let user = current_set(&app).user.unwrap();
    assert_eq!(user.revision, 4);
    assert_eq!(user.layout.command_line.lines, 8);
    assert_eq!(store(&app).read_snapshot(4), store(&app).read_current());
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
    assert!(!app.ws.autosave.pending());
    assert_eq!(shared.damaged(), vec![damaged.as_bytes().to_vec()]);
    let app = reattach(&shared);
    assert!(app.ws.fallback.is_none());
    assert_eq!(app.ui.history_lines, 8);
}

#[test]
fn first_run_has_no_profile_yet() {
    let app = attached(MemoryStore::default());
    assert_eq!(app.ws.fallback, Some(Fallback::NoProfile));
    assert_eq!(app.ws.active, "SIMPLE");
    assert!(app.ws.message.is_none());
    assert!(store(&app).read_current().is_none(), "nothing is written before the user changes something");
}

#[test]
fn store_write_failure_keeps_running() {
    let mut app = attached(MemoryStore { fail_writes: true, ..MemoryStore::default() });
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    let layout = app.ws.user.layout.clone();
    let docking = app.ui.docking.layout.clone();
    assert!(app.commit_workspace_now().is_err());
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "{:?}", app.ws.status);
    assert_eq!(app.ws.user.layout, layout);
    assert_eq!(app.ui.docking.layout, docking);
    assert!(!app.ui.show_toolbar);
    assert!(app.status.as_ref().is_some_and(|(text, _)| text.starts_with("Could not save workspace")));

    // The autosave path reports the same way from a frame, without panicking.
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 10.0);
    run(&mut app, "ui.toggle.statusbar", json!({"on": false}));
    app.ws.status = SaveStatus::Unsaved;
    frame(&mut app, &ctx, 12.0);
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "{:?}", app.ws.status);
    assert!(!app.ui.show_status_bar);
}

#[test]
fn autosave_commits_from_the_frame_loop_after_the_debounce() {
    let mut app = attached(MemoryStore::default());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    frame(&mut app, &ctx, 1.2);
    assert!(store(&app).read_current().is_none(), "still inside the debounce");
    frame(&mut app, &ctx, 2.0);
    assert!(!standard_toolbar_visible(&current_set(&app).user.unwrap()));
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
}

#[test]
fn layout_undo_is_separate_ux() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    run(&mut app, "line", json!({"points": [[0, 0], [10, 0]]}));
    let undo_len = app.session.state().unwrap().undo.len();
    let drawing = app.session.doc().unwrap().clone();
    let before = app.ui.docking.layout.clone();

    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "layers", "rect": [100, 100, 300, 200]}));
    let floated = app.ui.docking.layout.clone();
    assert_ne!(floated, before);
    run(&mut app, "ui.layout.undo", json!({}));
    assert_eq!(app.ui.docking.layout, before);
    assert_eq!(app.session.state().unwrap().undo.len(), undo_len, "layout undo leaves the drawing's undo alone");
    assert_eq!(*app.session.doc().unwrap(), drawing);
    run(&mut app, "ui.layout.redo", json!({}));
    assert!(app.ui.docking.layout.floating.iter().any(|group| group.panels.contains(&Panel::Layers)));

    let docking = app.ui.docking.layout.clone();
    run(&mut app, "u", json!({}));
    assert_ne!(*app.session.doc().unwrap(), drawing, "the drawing's undo ran");
    assert_eq!(app.ui.docking.layout, docking, "and left the layout alone");

    run(&mut app, "ui.layout.undo", json!({}));
    assert!(app.run("ui.layout.undo", json!({})).is_err(), "nothing left to undo");
}

#[test]
fn restore_creates_new_revision() {
    let mut app = attached(MemoryStore::default());
    for lines in 1..=10u64 {
        run(&mut app, "ui.cmdline.lines", json!({"lines": lines}));
        app.commit_workspace_now().unwrap();
    }
    assert_eq!(app.ws.user.revision, 10);
    let tenth = store(&app).read_snapshot(10);
    let result = run(&mut app, "ui.workspace.restore", json!({"revision": 3}));
    assert_eq!(result["revision"], json!(11));
    assert_eq!(app.ws.user.revision, 11);
    assert_eq!(app.ui.history_lines, 3);
    assert_eq!(store(&app).read_snapshot(10), tenth, "restoring never overwrites a snapshot");
    assert!(store(&app).snapshots().contains(&11));
    assert!(app.run("ui.workspace.restore", json!({"revision": 99})).is_err());
}

#[test]
fn save_as_and_switch_between_user_profiles() {
    let mut app = attached(MemoryStore::default());
    let arch = run(&mut app, "ui.workspace.saveas", json!({"name": "My Arch"}))["profile_id"].as_str().unwrap().to_string();
    run(&mut app, "ui.workspace.select", json!({"id": "SIMPLE"}));
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    let structure = run(&mut app, "ui.workspace.saveas", json!({"name": "My Struct"}))["profile_id"].as_str().unwrap().to_string();
    assert_ne!(arch, structure);

    let set = current_set(&app);
    assert_eq!(set.profiles.len(), 2);
    assert_eq!(set.active, structure);
    let choices = app.ws.choices();
    assert_eq!(choices.len(), PRESET_IDS.len() + 2);
    let ids: Vec<&str> = choices.iter().map(|choice| choice.id.as_str()).collect();
    assert_eq!(&ids[..PRESET_IDS.len()], PRESET_IDS);
    assert!(ids.contains(&arch.as_str()) && ids.contains(&structure.as_str()));

    run(&mut app, "ui.workspace.select", json!({"id": arch}));
    assert!(app.ui.show_toolbar);
    run(&mut app, "ui.workspace.select", json!({"id": structure}));
    assert!(!app.ui.show_toolbar);
    assert!(app.run("ui.workspace.saveas", json!({"name": "my arch"})).is_err(), "names are unique");
    assert!(app.run("ui.workspace.select", json!({"id": "LPC_COMPANY"})).is_err());

    for index in 2..MAX_USER_PROFILES {
        run(&mut app, "ui.workspace.saveas", json!({"name": format!("Workspace {index}")}));
    }
    assert_eq!(app.ws.profiles.len(), MAX_USER_PROFILES);
    let error = app.run("ui.workspace.saveas", json!({"name": "One too many"})).unwrap_err();
    assert!(error.contains("At most 20 saved workspaces"), "{error}");
}

#[test]
fn published_preset_is_immutable() {
    let mut app = attached(MemoryStore::default());
    let familiar = preset("AUTOCAD_FAMILIAR").unwrap();
    run(&mut app, "ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"}));
    assert_eq!(app.ws.effective.layout, familiar.layout);
    assert_eq!(user_patch(&app), json!({}));

    run(&mut app, "ui.toggle.statusbar", json!({"on": false}));
    assert_ne!(app.ws.effective.layout, familiar.layout);
    assert_eq!(user_patch(&app), json!({"status_bar": {"visible": false}}), "the change went into the user layer");
    let platform = app.ws.layers.iter().find(|layer| layer.level == Level::Platform).unwrap();
    assert_eq!(platform.patch, serde_json::to_value(&familiar.layout).unwrap());
    assert_eq!(preset("AUTOCAD_FAMILIAR"), Some(familiar.clone()));

    run(&mut app, "ui.workspace.reset", json!({}));
    assert_eq!(app.ws.effective.layout, familiar.layout);
    assert_eq!(user_patch(&app), json!({}));
    assert!(app.ui.show_status_bar);
    run(&mut app, "ui.layout.undo", json!({}));
    assert!(!app.ui.show_status_bar, "a reset can be undone");
}

#[test]
fn import_dry_run_lists_changes_without_applying() {
    let mut app = attached(MemoryStore::default());
    app.commit_workspace_now().unwrap();
    let first = store(&app).read_snapshot(1).unwrap();
    let layout = app.ws.user.layout.clone();
    let docking = app.ui.docking.layout.clone();

    let preview = run(&mut app, "ui.workspace.import", json!({"text": SPEC_SAMPLE, "dry_run": true}));
    let changes: Vec<&str> = preview["changes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(changes.contains(&"/shortcuts/line"), "{changes:?}");
    assert!(changes.iter().any(|pointer| pointer.starts_with("/panels/layers")), "{changes:?}");
    assert_eq!(app.ws.user.layout, layout);
    assert_eq!(app.ui.docking.layout, docking);
    assert_eq!(app.ws.user.revision, 1);

    run(&mut app, "ui.workspace.import", json!({"text": SPEC_SAMPLE}));
    assert_eq!(app.ws.user.layout.shortcuts.get("line").map(String::as_str), Some("L"));
    assert_eq!(app.ws.user.base_id.as_deref(), Some("AUTOCAD_FAMILIAR"));
    assert_eq!(app.ws.user.revision, 2);
    assert_eq!(store(&app).read_snapshot(1), Some(first), "the earlier revision is kept");

    let hostile = r#"{"schema_version":"1.0.0","layout":{"toolbars":[{"id":"x","meta":{"onload":"x"}}]}}"#;
    let layout = app.ws.user.layout.clone();
    assert!(app.run("ui.workspace.import", json!({"text": hostile})).is_err());
    assert_eq!(app.ws.user.layout, layout);
}

#[test]
fn export_has_no_secrets_and_round_trips() {
    let mut app = attached(MemoryStore::default());
    app.ui.recent_files = vec!["C:\\drawings\\secret.dwg".into(), "/home/me/plan.dxf".into()];
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "layers", "rect": [100, 100, 300, 200]}));
    let text = run(&mut app, "ui.workspace.export", json!({}))["text"].as_str().unwrap().to_string();
    assert!(validate(&text, Mode::Import).is_ok(), "{:?}", validate(&text, Mode::Import));
    for secret in ["secret.dwg", "plan.dxf", ":\\", "/home"] {
        assert!(!text.contains(secret), "{secret} leaked into the export");
    }
    let layout = app.ws.user.layout.clone();
    run(&mut app, "ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"}));
    assert_ne!(app.ws.user.layout, layout);
    run(&mut app, "ui.workspace.import", json!({"text": text}));
    assert_eq!(app.ws.user.layout, layout);
}

fn set_with_user(theme: Value, locale: Value) -> String {
    let mut user = serde_json::to_value(preset("SIMPLE").unwrap()).unwrap();
    user["profile_id"] = json!("user.SIMPLE");
    user["owner_type"] = json!("user");
    user["base_id"] = json!("SIMPLE");
    user["theme"] = theme;
    user["locale"] = locale;
    json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": user}).to_string()
}

fn app_with_prefs(current: String) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.ui.theme = ThemePref::Light;
    app.ui.interface_language = Preference::parse("de").unwrap();
    app.attach_profile_store(Box::new(MemoryStore { current: Some(current), ..MemoryStore::default() }));
    app
}

#[test]
fn theme_and_locale_follow_profile() {
    let mut app = app_with_prefs(set_with_user(json!("dark"), json!("uk")));
    assert_eq!(app.ui.theme, ThemePref::Dark);
    assert_eq!(app.ui.interface_language.code(), "uk");
    // A profile that names a theme keeps the user's later choice.
    run(&mut app, "ui.theme", json!({"theme": "light"}));
    assert!(app.ws.autosave.pending(), "the profile changed, so it is saved");
    app.commit_workspace_now().unwrap();
    assert_eq!(current_set(&app).user.unwrap().theme.as_deref(), Some("light"));

    let mut app = app_with_prefs(set_with_user(Value::Null, Value::Null));
    assert_eq!(app.ui.theme, ThemePref::Light, "no theme in the profile: the saved preference stays");
    assert_eq!(app.ui.interface_language.code(), "de");
    run(&mut app, "ui.theme", json!({"theme": "dark"}));
    app.commit_workspace_now().unwrap();
    assert_eq!(current_set(&app).user.unwrap().theme, None, "preferences keep the choice, not the profile");
}

#[test]
fn restoring_the_saved_options_keeps_an_unchanged_drawing_clean() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.sample", json!({}));
    let drawing = app.session.state().unwrap().doc.clone();
    let profile = app.profile_json();
    app.load_profile(&profile);
    let state = app.session.state().unwrap();
    assert!(!state.is_dirty(), "GRIDUNIT/SNAPUNIT equal to the drawing's own are not written back");
    assert!(std::sync::Arc::ptr_eq(&drawing, &state.doc));
}

#[test]
fn profile_save_never_dirties_drawing_ux13() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.sample", json!({}));
    let sample = app.session.doc().unwrap().clone();
    assert!(!app.session.state().unwrap().is_dirty());
    for id in ["AUTOCAD_FAMILIAR", "ARCHITECTURE", "SIMPLE"] {
        run(&mut app, "ui.workspace.select", json!({"id": id}));
        app.commit_workspace_now().unwrap();
    }
    assert!(!app.session.state().unwrap().is_dirty());
    assert_eq!(*app.session.doc().unwrap(), sample);

    run(&mut app, "layer.current", json!({"name": "0"}));
    let drawing = app.session.doc().unwrap().clone();
    let dirty = app.session.state().unwrap().is_dirty();
    let space = app.session.state().unwrap().space.clone();
    let entities = drawing.model.len();
    for id in ["MEP_FIRE", "QS_QA", "SIMPLE"] {
        run(&mut app, "ui.workspace.select", json!({"id": id}));
        app.commit_workspace_now().unwrap();
    }
    let state = app.session.state().unwrap();
    assert_eq!(state.is_dirty(), dirty);
    assert_eq!(state.space, space);
    assert_eq!(*app.session.doc().unwrap(), drawing, "layer, UCS, views and entities are untouched");
    assert_eq!(app.session.doc().unwrap().header.str("CLAYER", ""), "0");
    assert_eq!(app.session.doc().unwrap().model.len(), entities);
}

#[test]
fn locked_pointers_refuse_user_changes() {
    let policy =
        json!({"layers": [{"level": "company", "patch": {"shortcuts": {"line": "LL"}}, "locks": ["/shortcuts/line", "/toolbars/standard"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    assert_eq!(app.ws.effective.layout.shortcuts.get("line").map(String::as_str), Some("LL"));
    let error = app.run("ui.workspace.set", json!({"pointer": "/shortcuts/line", "value": "L"})).unwrap_err();
    assert!(error.contains("Locked by company policy"), "{error}");
    // A change made in the interface is turned back.
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    assert!(app.ui.show_toolbar);
    assert_eq!(app.ws.message.as_deref(), Some("Locked by company policy"));
}

#[test]
fn unknown_ids_survive_capture_and_are_reported() {
    let mut user = serde_json::to_value(preset("SIMPLE").unwrap()).unwrap();
    user["base_id"] = json!("SIMPLE");
    user["layout"]["panels"].as_array_mut().unwrap().push(json!({"id": "sheetSet", "mode": "dock", "side": "left"}));
    user["layout"]["quick_access"] = json!(["qsave", "futurecommand"]);
    let current = json!({"active": "SIMPLE", "user": user}).to_string();
    let mut app = attached(MemoryStore { current: Some(current), ..MemoryStore::default() });
    assert_eq!(app.ws.report.unknown_panels, vec!["sheetSet".to_string()]);
    assert_eq!(app.ws.report.unknown_commands, vec!["futurecommand".to_string()]);
    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "layers", "rect": [100, 100, 300, 200]}));
    app.commit_workspace_now().unwrap();
    let stored = current_set(&app).user.unwrap();
    assert!(stored.layout.panels.iter().any(|panel| panel.id == "sheetSet"));
    assert!(stored.layout.quick_access.contains(&"futurecommand".to_string()));
}

#[test]
fn every_preset_applies_and_reads_back_unchanged() {
    let mut app = attached(MemoryStore::default());
    for id in PRESET_IDS {
        run(&mut app, "ui.workspace.select", json!({"id": id}));
        assert!(app.ui.docking.validate().is_ok(), "{id}");
        assert!(app.ui.docking.layout.contains(&Panel::Canvas), "{id}");
        assert_eq!(app.ws.user.layout, preset(id).unwrap().layout, "{id}: selecting must not record an edit");
        let status = run(&mut app, "ui.workspace.status", json!({}));
        assert_eq!(status["active"], json!(id));
    }
}

/// A new app over a copy of what `app` stored, with `policy` as the host's policy file.
fn restarted(app: &CadApp, policy: Option<Value>) -> CadApp {
    let old = store(app);
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), snapshot_map, policy: policy.map(|policy| policy.to_string()), ..MemoryStore::default() })
}

fn left_column_holds(app: &CadApp, panel: Panel) -> bool {
    fn holds(node: &Node<Panel>, panel: Panel) -> bool {
        match node {
            Node::Split { first, second, .. } => holds(first, panel) || holds(second, panel),
            Node::Tabs { panels, .. } => panels.contains(&panel),
            Node::Stack { entries } => entries.iter().any(|entry| entry.panel == panel),
        }
    }
    match &app.ui.docking.layout.root {
        Some(Node::Split { first, .. }) => !holds(first, Panel::Canvas) && holds(first, panel),
        _ => false,
    }
}

fn root_size(app: &CadApp) -> SplitSize {
    match &app.ui.docking.layout.root {
        Some(Node::Split { size, .. }) => *size,
        other => panic!("the root is not a split: {other:?}"),
    }
}

#[test]
fn policy_change_reaches_committed_user() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toggle.statusbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    // The company later changes two fields; the user changed one of them, and nothing is locked.
    let policy = json!({"layers": [{"level": "company", "patch": {"command_line": {"lines": 9}, "status_bar": {"visible": true}}}]});
    let app = restarted(&app, Some(policy));
    assert!(app.ws.fallback.is_none(), "{:?}", app.ws.fallback);
    assert_eq!(app.ws.effective.layout.command_line.lines, 9, "the policy change shows through");
    assert_eq!(app.ui.history_lines, 9);
    assert!(!app.ws.effective.layout.status_bar.visible, "the user's own change still wins");
    assert!(!app.ui.show_status_bar);
}

#[test]
fn locked_panel_cannot_be_moved_by_dragging() {
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/panels/layers"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    assert!(!left_column_holds(&app, Panel::Layers));
    run(&mut app, "ui.dock", json!({"operation": "move", "panel": "layers", "target": "toolSets", "zone": "bottom"}));
    assert_eq!(app.ws.message.as_deref(), Some("Locked by company policy"));
    assert!(!left_column_holds(&app, Panel::Layers), "the locked panel goes back");
    assert!(app.ui.docking.layout.contains(&Panel::Layers));
    app.commit_workspace_now().unwrap();
    let app = restarted(&app, Some(policy));
    assert!(!left_column_holds(&app, Panel::Layers), "and stays back after a restart");
    assert!(app.ui.docking.layout.contains(&Panel::Layers));
}

#[test]
fn session_hidden_panel_not_persisted_via_dock_tree() {
    let mut app = attached(MemoryStore::default());
    app.set_session_layer(json!({"panels": [{"id": "layers", "visible": false, "mode": "hidden"}]}));
    assert!(!app.ui.docking.layout.contains(&Panel::Layers));
    run(&mut app, "ui.dock", json!({"operation": "move", "panel": "properties", "target": "toolSets", "zone": "bottom"}));
    assert!(left_column_holds(&app, Panel::Properties), "the user's own move is kept");
    assert!(!app.ui.docking.layout.contains(&Panel::Layers), "the session layer still hides the panel");
    app.commit_workspace_now().unwrap();
    let app = restarted(&app, None);
    assert!(app.ui.docking.layout.contains(&Panel::Layers), "the session's hidden panel was not stored");
    assert!(left_column_holds(&app, Panel::Properties));
}

#[test]
fn session_layer_hides_a_panel_the_users_dock_tree_shows() {
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.dock", json!({"operation": "move", "panel": "properties", "target": "toolSets", "zone": "bottom"}));
    assert!(app.ws.user.layout.dock_tree.is_some());
    app.set_session_layer(json!({"panels": [{"id": "layers", "visible": false, "mode": "hidden"}]}));
    assert!(!app.ui.docking.layout.contains(&Panel::Layers));
}

#[test]
fn dock_sizes_are_stored_as_fractions_of_the_window() {
    let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let large = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2560.0, 1600.0));
    let mut app = attached(MemoryStore::default());
    app.ws.area = small;
    app.apply_workspace();
    run(&mut app, "ui.dock", json!({"operation": "resizeSplit", "path": [], "size": {"FixedFirst": 256.0}}));
    app.commit_workspace_now().unwrap();
    assert_eq!(root_size(&app), SplitSize::FixedFirst(256.0));
    app.ws.area = large;
    app.apply_workspace();
    assert_eq!(root_size(&app), SplitSize::FixedFirst(512.0), "the column keeps its share of a larger window");
    let mut app = restarted(&app, None);
    assert_eq!(root_size(&app), SplitSize::FixedFirst(256.0));

    // A tree written before sizes were fractions keeps its points.
    let mut legacy = cadcraft_ui_egui::docking::Workspace::default();
    legacy.layout.root = app.ui.docking.layout.root.clone();
    if let Some(Node::Split { size, .. }) = &mut legacy.layout.root {
        *size = SplitSize::FixedFirst(300.0);
    }
    let mut tree = serde_json::to_value(&legacy).unwrap();
    tree["rects"] = json!("ratio");
    let mut layout = app.ws.user.layout.clone();
    layout.dock_tree = Some(tree);
    cadcraft_ui_egui::wsprofile::apply::apply(&mut app, &layout, large);
    assert_eq!(root_size(&app), SplitSize::FixedFirst(300.0));
}

#[test]
fn set_value_cannot_brick_restart() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.workspace.set", json!({"pointer": "/aliases/ln", "value": "line"}));
    app.commit_workspace_now().unwrap();
    let deep = (0..40).map(|level| format!("/n{level}")).collect::<String>();
    for (pointer, value) in [("/aliases/mytoken", json!("line")), ("/aliases/long", json!("x".repeat(2000))), (deep.as_str(), json!(1))] {
        assert!(app.run("ui.workspace.set", json!({"pointer": pointer, "value": value})).is_err(), "{pointer} was accepted");
    }
    assert!(app.ws.user.layout.aliases.keys().all(|key| key == "ln"));
    // Whatever puts it there, a profile startup would refuse is never written.
    app.ws.user.layout.aliases.insert("apiKey".into(), "line".into());
    let saved = store(&app).read_current();
    assert!(app.commit_workspace_now().is_err());
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "{:?}", app.ws.status);
    assert_eq!(store(&app).read_current(), saved);

    let app = restarted(&app, None);
    assert!(app.ws.fallback.is_none(), "{:?}", app.ws.fallback);
    assert_eq!(app.ws.user.layout.aliases.get("ln").map(String::as_str), Some("line"));
}

/// Snapshots can be refused while the current set can still be written.
struct SnapshotsRefused {
    inner: MemoryStore,
    refuse: std::rc::Rc<std::cell::Cell<bool>>,
}

impl ProfileStore for SnapshotsRefused {
    fn read_current(&self) -> Option<String> {
        self.inner.read_current()
    }
    fn write_current(&mut self, json: &str) -> Result<(), String> {
        self.inner.write_current(json)
    }
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        if self.refuse.get() {
            return Err("snapshots are refused".into());
        }
        self.inner.push_snapshot(revision, json)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.inner.snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.inner.read_snapshot(revision)
    }
}

#[test]
fn a_damaged_set_that_was_not_kept_is_not_overwritten() {
    let refuse = std::rc::Rc::new(std::cell::Cell::new(true));
    let damaged = SnapshotsRefused { inner: MemoryStore { current: Some("{".into()), ..MemoryStore::default() }, refuse: refuse.clone() };
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(damaged));
    assert!(matches!(app.ws.fallback, Some(Fallback::Corrupt(_))));
    let message = app.ws.message.clone().unwrap_or_default();
    assert_ne!(message, FALLBACK_MESSAGE, "the damaged copy was not kept, so the message must not say so");
    assert!(message.contains("could not be kept"), "{message}");
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    assert!(app.commit_workspace_now().is_err());
    assert_eq!(store(&app).read_current().as_deref(), Some("{"), "the only copy is not overwritten");
    refuse.set(false);
    app.commit_workspace_now().unwrap();
    assert_eq!(store(&app).read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
    assert!(!standard_toolbar_visible(&current_set(&app).user.unwrap()));
}

#[test]
fn an_unreadable_policy_is_reported() {
    for policy in ["not json", r#"{"layers": 5}"#, r#"{"layers": [{"level": "user", "patch": {}}]}"#] {
        let app = attached(MemoryStore { policy: Some(policy.into()), ..MemoryStore::default() });
        let message = app.ws.message.clone().unwrap_or_default();
        assert!(message.contains("policy"), "{policy}: {message:?}");
    }
    let app = attached(MemoryStore { policy: Some(json!({"layers": []}).to_string()), ..MemoryStore::default() });
    assert!(app.ws.message.is_none());
}

#[test]
fn restore_keeps_profiles_saved_since() {
    let mut app = attached(MemoryStore::default());
    let first = run(&mut app, "ui.workspace.saveas", json!({"name": "First"}))["profile_id"].as_str().unwrap().to_string();
    let revision = app.ws.user.revision;
    let second = run(&mut app, "ui.workspace.saveas", json!({"name": "Second"}))["profile_id"].as_str().unwrap().to_string();
    run(&mut app, "ui.workspace.restore", json!({"revision": revision}));
    let ids: Vec<&str> = app.ws.profiles.iter().map(|profile| profile.profile_id.as_str()).collect();
    assert!(ids.contains(&first.as_str()) && ids.contains(&second.as_str()), "{ids:?}");
    assert_eq!(current_set(&app).profiles.len(), 2);
}

fn frame_with(app: &mut CadApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    let raw = egui::RawInput { time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| app.logic(ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn a_drag_is_one_undo_step_and_is_saved_on_release() {
    let mut app = attached(MemoryStore::default());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    let at = egui::pos2(600.0, 400.0);
    let button =
        |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() };
    frame_with(&mut app, &ctx, 1.1, vec![egui::Event::PointerMoved(at), button(true)]);
    for (step, time) in [(240.0, 1.15), (260.0, 1.2), (280.0, 1.25)] {
        if let Some(Node::Split { size, .. }) = &mut app.ui.docking.layout.root {
            *size = SplitSize::FixedFirst(step);
        }
        frame_with(&mut app, &ctx, time, vec![]);
    }
    assert!(store(&app).read_current().is_none(), "nothing is saved mid-drag");
    frame_with(&mut app, &ctx, 1.3, vec![button(false)]);
    assert_eq!(app.ws.history.undo_len(), 1, "the whole drag is one layout undo step");
    assert!(store(&app).read_current().is_some(), "the release saves at once, without the debounce");
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
}

/// A `MemoryStore` the test can still look into after the app took it.
#[derive(Clone)]
struct Shared(std::rc::Rc<std::cell::RefCell<MemoryStore>>, std::rc::Rc<std::cell::Cell<bool>>);

impl Shared {
    fn new(store: MemoryStore) -> Self {
        Self(std::rc::Rc::new(std::cell::RefCell::new(store)), std::rc::Rc::new(std::cell::Cell::new(false)))
    }

    fn damaged(&self) -> Vec<Vec<u8>> {
        self.0.borrow().damaged.values().cloned().collect()
    }
}

impl ProfileStore for Shared {
    fn take_external_change(&mut self) -> bool {
        self.1.replace(false)
    }

    fn read_current(&self) -> Option<String> {
        self.0.borrow().read_current()
    }
    fn write_current(&mut self, json: &str) -> Result<(), String> {
        self.0.borrow_mut().write_current(json)
    }
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        self.0.borrow_mut().push_snapshot(revision, json)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.0.borrow().snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.0.borrow().read_snapshot(revision)
    }
    fn read_policy(&self) -> Option<String> {
        self.0.borrow().read_policy()
    }
    fn keep_damaged(&mut self, raw: &[u8]) -> Result<String, String> {
        self.0.borrow_mut().keep_damaged(raw)
    }
}

#[test]
fn view_toggles_survive_restart() {
    let mut app = attached(MemoryStore::default());
    assert!(app.ui.show_viewcube && app.ui.show_ucs_icon);
    run(&mut app, "ui.toggle.viewcube", json!({"on": false}));
    run(&mut app, "ui.toggle.ucsicon", json!({"on": false}));
    app.ui.show_layer_list = true;
    app.commit_workspace_now().unwrap();
    let view = &current_set(&app).user.unwrap().layout.view;
    assert!(!view.viewcube && !view.ucs_icon && view.layer_list);
    let app = restarted(&app, None);
    assert!(!app.ui.show_viewcube, "the viewcube stays hidden");
    assert!(!app.ui.show_ucs_icon, "the UCS icon stays hidden");
    assert!(app.ui.show_layer_list);
    assert!(app.ui.show_file_tabs && app.ui.properties_all, "untouched switches keep their defaults");

    // A policy lock covers the view switches like any other field.
    let policy = json!({"layers": [{"level": "company", "patch": {"view": {"viewcube": true}}, "locks": ["/view/viewcube"]}]});
    let mut app = restarted(&app, Some(policy));
    assert!(app.ui.show_viewcube);
    run(&mut app, "ui.toggle.viewcube", json!({"on": false}));
    assert!(app.ui.show_viewcube, "the locked switch is turned back");
    assert!(!app.ui.show_ucs_icon);
}

/// Fails the next `failures` writes, then stores normally.
struct Flaky {
    inner: MemoryStore,
    failures: u32,
}

impl Flaky {
    fn fail(&mut self) -> Result<(), String> {
        if self.failures > 0 {
            self.failures -= 1;
            return Err("storage is busy".into());
        }
        Ok(())
    }
}

impl ProfileStore for Flaky {
    fn read_current(&self) -> Option<String> {
        self.inner.read_current()
    }
    fn write_current(&mut self, json: &str) -> Result<(), String> {
        self.fail()?;
        self.inner.write_current(json)
    }
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        self.inner.push_snapshot(revision, json)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.inner.snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.inner.read_snapshot(revision)
    }
}

#[test]
fn failed_commit_is_retried() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(Flaky { inner: MemoryStore::default(), failures: 1 }));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0);
    assert!(!app.ws.needs_flush());
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    assert!(app.ws.needs_flush());
    frame(&mut app, &ctx, 2.0);
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "{:?}", app.ws.status);
    assert!(app.ws.needs_flush(), "the change is still only in memory");
    assert!(app.ws.autosave.pending(), "another attempt is armed");
    frame(&mut app, &ctx, 4.0);
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)), "the retry waits");
    frame(&mut app, &ctx, 7.5);
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
    assert!(!app.ws.needs_flush());
    assert!(!standard_toolbar_visible(&current_set(&app).user.unwrap()));
}

#[test]
fn second_damaged_start_keeps_both_copies() {
    let shared = Shared::new(MemoryStore { current: Some("{".into()), ..MemoryStore::default() });
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    assert_eq!(app.ws.message.as_deref(), Some(FALLBACK_MESSAGE));
    drop(app);
    // Damaged again, differently, before anything was committed.
    shared.0.borrow_mut().current = Some("[1".into());
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(shared.clone()));
    assert!(matches!(app.ws.fallback, Some(Fallback::Corrupt(_))));
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    let mut kept = shared.damaged();
    kept.sort();
    assert_eq!(kept, vec![b"[1".to_vec(), b"{".to_vec()], "neither damaged copy replaced the other");
    assert!(!store(&app).snapshots().contains(&CORRUPT_SNAPSHOT));
    // The same damage seen twice is kept once.
    assert_eq!(shared.0.borrow_mut().keep_damaged(b"{"), Ok("damaged-1".to_string()));
}

fn two_profiles() -> MemoryStore {
    let mut a = preset("SIMPLE").unwrap();
    a.profile_id = "user.A".into();
    a.base_id = Some("SIMPLE".into());
    a.name = "A".into();
    a.owner_type = OwnerType::User;
    a.status = ProfileStatus::Draft;
    let mut b = preset("AUTOCAD_FAMILIAR").unwrap();
    b.profile_id = "user.B".into();
    b.base_id = Some("AUTOCAD_FAMILIAR".into());
    b.name = "B".into();
    b.owner_type = OwnerType::User;
    b.status = ProfileStatus::Draft;
    b.layout.status_bar.visible = false;
    let set = ProfileSet { active: a.profile_id.clone(), user: Some(a.clone()), profiles: vec![a, b], ..Default::default() };
    MemoryStore { current: Some(serde_json::to_string(&set).unwrap()), ..Default::default() }
}

#[test]
fn restore_across_profiles_undo_preserves_identity_and_saved_layouts() {
    let _aliases = alias_lock();
    let mut app = attached(two_profiles());
    app.commit_workspace_now().unwrap();
    let a = app.ws.user.clone();
    run(&mut app, "ui.workspace.select", json!({"id": "user.B"}));
    let b = app.ws.user.clone();
    run(&mut app, "ui.workspace.restore", json!({"revision": a.revision}));
    let restored_revision = app.ws.user.revision;
    assert!(restored_revision > b.revision);
    assert_eq!(app.ws.active, "user.A");
    run(&mut app, "ui.layout.undo", json!({}));
    app.commit_workspace_now().unwrap();
    assert!(app.ws.user.revision > restored_revision);
    let again = restarted(&app, None);
    assert_eq!(again.ws.active, "user.B");
    assert_eq!(again.ws.user.profile_id, b.profile_id);
    assert_eq!(again.ws.user.base_id, b.base_id);
    assert_eq!(again.ws.user.layout, b.layout);
    let saved = current_set(&app);
    assert_eq!(saved.profiles.iter().find(|profile| profile.profile_id == a.profile_id).unwrap().layout, a.layout);
    assert_eq!(saved.profiles.iter().find(|profile| profile.profile_id == b.profile_id).unwrap().layout, b.layout);
    let undone_revision = app.ws.user.revision;
    run(&mut app, "ui.layout.redo", json!({}));
    app.commit_workspace_now().unwrap();
    assert!(app.ws.user.revision > undone_revision);
    let again = restarted(&app, None);
    assert_eq!(again.ws.active, "user.A");
    assert_eq!(again.ws.user.profile_id, a.profile_id);
    assert_eq!(again.ws.user.base_id, a.base_id);
    assert_eq!(again.ws.user.layout, a.layout);
    assert_eq!(again.ws.profiles.iter().find(|profile| profile.profile_id == b.profile_id).unwrap().layout, b.layout);
}

#[test]
fn import_base_change_undo_restores_identity_and_preferences() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.25}));
    app.commit_workspace_now().unwrap();
    let before = app.ws.user.clone();
    run(&mut app, "ui.workspace.import", json!({"text": SPEC_SAMPLE}));
    let imported = app.ws.user.clone();
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
    assert_eq!(imported.base_id.as_deref(), Some("AUTOCAD_FAMILIAR"));
    run(&mut app, "ui.layout.undo", json!({}));
    app.commit_workspace_now().unwrap();
    let again = restarted(&app, None);
    assert_eq!(again.ws.active, "SIMPLE");
    assert_eq!(again.ws.user.profile_id, before.profile_id);
    assert_eq!(again.ws.user.base_id, before.base_id);
    assert_eq!(again.ws.user.layout, before.layout);
    assert_eq!(again.ws.user.ui_scale, before.ui_scale);
    run(&mut app, "ui.layout.redo", json!({}));
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
    assert_eq!(app.ws.user.base_id, imported.base_id);
    assert_eq!(app.ws.user.layout, imported.layout);
    assert_eq!(app.ws.user.ui_scale, imported.ui_scale);
}

#[test]
fn save_as_undo_restores_source_identity_before_earlier_edits() {
    let _aliases = alias_lock();
    let mut app = attached(two_profiles());
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    let copy = run(&mut app, "ui.workspace.saveas", json!({"name": "Copy"}))["profile_id"].as_str().unwrap().to_string();
    assert_eq!(app.ws.active, copy);
    run(&mut app, "ui.layout.undo", json!({}));
    assert_eq!(app.ws.active, "user.A");
    assert_eq!(app.ws.user.profile_id, "user.A");
    assert!(!app.ui.show_toolbar);
    run(&mut app, "ui.layout.undo", json!({}));
    app.commit_workspace_now().unwrap();
    assert!(app.ui.show_toolbar);
    let again = restarted(&app, None);
    assert_eq!(again.ws.active, "user.A");
    assert!(!standard_toolbar_visible(again.ws.profiles.iter().find(|profile| profile.profile_id == copy).unwrap()));
    run(&mut app, "ui.layout.redo", json!({}));
    run(&mut app, "ui.layout.redo", json!({}));
    assert_eq!(app.ws.active, copy);
    assert_eq!(app.ws.user.profile_id, copy);
    assert!(!app.ui.show_toolbar);
}

struct RefuseOutgoingOnce {
    inner: MemoryStore,
    snapshot: bool,
    armed: std::rc::Rc<std::cell::Cell<bool>>,
}

impl ProfileStore for RefuseOutgoingOnce {
    fn read_current(&self) -> Option<String> {
        self.inner.read_current()
    }
    fn write_current(&mut self, text: &str) -> Result<(), String> {
        if !self.snapshot && self.armed.replace(false) {
            return Err("current write refused".into());
        }
        self.inner.write_current(text)
    }
    fn push_snapshot(&mut self, revision: u64, text: &str) -> Result<(), String> {
        if self.snapshot && self.armed.replace(false) {
            return Err("snapshot write refused".into());
        }
        self.inner.push_snapshot(revision, text)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.inner.snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.inner.read_snapshot(revision)
    }
}

#[test]
fn failed_outgoing_save_blocks_workspace_switch() {
    let _aliases = alias_lock();
    for snapshot in [true, false] {
        for action in ["select", "saveas", "restore", "import", "undo", "redo"] {
            let armed = std::rc::Rc::new(std::cell::Cell::new(false));
            let mut app = CadApp::new(Session::new(), Services::default());
            app.skip_first_run_picker = true;
            app.attach_profile_store(Box::new(RefuseOutgoingOnce { inner: two_profiles(), snapshot, armed: armed.clone() }));
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, 1.0);
            app.commit_workspace_now().unwrap();
            let revision = app.ws.user.revision;
            if matches!(action, "undo" | "redo") {
                run(&mut app, "ui.workspace.select", json!({"id": "user.B"}));
                if action == "redo" {
                    run(&mut app, "ui.layout.undo", json!({}));
                }
            }
            run(&mut app, "ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.25}));
            let before = app.ws.user.clone();
            let active = app.ws.active.clone();
            let profiles = app.ws.profiles.clone();
            let history = (app.ws.history.undo_len(), app.ws.history.redo_len());
            let current = store(&app).read_current();
            armed.set(true);
            let (command, params) = match action {
                "select" => ("ui.workspace.select", json!({"id": "user.B"})),
                "saveas" => ("ui.workspace.saveas", json!({"name": "Copy"})),
                "restore" => ("ui.workspace.restore", json!({"revision": revision})),
                "import" => ("ui.workspace.import", json!({"text": SPEC_SAMPLE})),
                "undo" => ("ui.layout.undo", json!({})),
                _ => ("ui.layout.redo", json!({})),
            };
            assert!(app.run(command, params).is_err(), "{action}, snapshot={snapshot}");
            assert_eq!(app.ws.active, active);
            assert_eq!(app.ws.user, before);
            assert_eq!(app.ws.profiles, profiles);
            assert_eq!((app.ws.history.undo_len(), app.ws.history.redo_len()), history);
            assert_eq!(store(&app).read_current(), current);
            assert!(matches!(app.ws.status, SaveStatus::Failed(_)));
            assert!(app.status.as_ref().is_some_and(|(text, _)| text.contains("write refused")));
            assert!(app.ws.autosave.pending());
            assert_eq!(app.ws.autosave.wait_secs(), 5.0);
            frame(&mut app, &ctx, 4.0);
            assert!(app.ws.autosave.pending(), "the retry still waits");
            frame(&mut app, &ctx, 7.0);
            assert_eq!(app.ws.status, SaveStatus::SavedLocally);
            assert!(!app.ws.needs_flush());
            let again = restarted(&app, None);
            assert_eq!(again.ws.active, active);
            assert_eq!(again.ws.user.profile_id, before.profile_id);
            assert_eq!(again.ws.user.layout, before.layout);
            assert_eq!(again.ws.user.ui_scale, 1.25);
        }
    }
}

fn picker_only(app: CadApp) -> Harness<'static, CadApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            let mut open = true;
            if app.ui.dialog.as_deref() == Some("workspace_picker") {
                cadcraft_ui_egui::wsprofile::picker::dialog(app, ui.ctx(), &mut open);
            }
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn click_simple(harness: &mut Harness<'static, CadApp>) {
    harness.query_all_by_label("Simple").find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().click();
    harness.run_steps(4);
}

#[test]
fn picker_reselect_active_preserves_committed_overrides() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    let before = app.ws.user.clone();
    let stored = store(&app).read_current();
    let history = (app.ws.history.undo_len(), app.ws.history.redo_len());
    run(&mut app, "ui.workspace.select", Value::Null);
    let mut harness = picker_only(app);
    click_simple(&mut harness);
    let app = harness.state_mut();
    assert_eq!(app.ui.dialog, None);
    assert!(!app.ui.show_toolbar);
    assert_eq!(app.ws.user, before);
    assert_eq!(store(app).read_current(), stored);
    assert_eq!((app.ws.history.undo_len(), app.ws.history.redo_len()), history);
    run(app, "ui.workspace.select", json!({"id": "SIMPLE"}));
    assert_eq!(app.ws.user, before);
    assert_eq!(store(app).read_current(), stored);
}

#[test]
fn select_active_saved_profile_preserves_pending_edit() {
    let _aliases = alias_lock();
    let mut app = attached(two_profiles());
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    let before = app.ws.user.clone();
    let stored = store(&app).read_current();
    let autosave = app.ws.autosave.clone();
    let history = (app.ws.history.undo_len(), app.ws.history.redo_len());
    run(&mut app, "ui.workspace.select", json!({"id": "user.A"}));
    assert_eq!(app.ws.user, before);
    assert_eq!(app.ws.active, "user.A");
    assert_eq!(app.ws.autosave, autosave);
    assert_eq!(store(&app).read_current(), stored);
    assert_eq!((app.ws.history.undo_len(), app.ws.history.redo_len()), history);
    app.commit_workspace_now().unwrap();
    assert!(!restarted(&app, None).ui.show_toolbar);
}

#[test]
fn picker_select_active_first_run_records_choice() {
    let _aliases = alias_lock();
    let mut app = attached(MemoryStore::default());
    run(&mut app, "ui.workspace.select", Value::Null);
    let mut harness = picker_only(app);
    click_simple(&mut harness);
    let app = harness.state();
    assert_eq!(app.ui.dialog, None);
    assert_eq!(app.ws.active, "SIMPLE");
    assert!(!app.ws.first_run);
    assert_eq!(app.ws.history.undo_len(), 0);
    let again = restarted(app, None);
    assert_eq!(again.ws.active, "SIMPLE");
    assert!(!again.ws.first_run);
    assert!(again.ws.fallback.is_none());
}

#[test]
fn dock_tree_cannot_override_locked_panel_geometry() {
    use cadcraft_ui_egui::wsprofile::conflict::Resolution;
    use cadcraft_ui_egui::wsprofile::sync::MemoryRemote;
    use craft_ui::docking::{Action, Placement, Zone};

    let _aliases = alias_lock();
    for field in ["width_ratio", "height_ratio", "side", "tab_group", "float_width", "float_height", "x_ratio", "y_ratio", "whole_panel"] {
        for route in ["set", "import", "restore", "sync"] {
            let floats = matches!(field, "float_width" | "float_height" | "x_ratio" | "y_ratio");
            let lock_field = match field {
                "float_width" => "width_ratio",
                "float_height" => "height_ratio",
                other => other,
            };
            let pointer = if field == "whole_panel" { "/panels/properties".to_string() } else { format!("/panels/properties/{lock_field}") };
            let panel = if floats {
                json!({"id": "properties", "mode": "float", "side": "floating", "x_ratio": 0.1, "y_ratio": 0.1,
                    "width_ratio": 0.25, "height_ratio": 0.3})
            } else {
                json!({"id": "properties", "width_ratio": 0.21, "height_ratio": 0.6})
            };
            let policy = json!({"layers": [{"level": "company", "patch": {"panels": [panel]}, "locks": [pointer]}]});
            let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..Default::default() });
            app.commit_workspace_now().unwrap();
            let expected = app.ui.docking.layout.clone();
            let mut malicious = app.ui.docking.clone();
            let action = match field {
                "width_ratio" | "whole_panel" => Action::ResizeSplit { path: vec![true], size: SplitSize::FixedSecond(640.0) },
                "height_ratio" => Action::ResizeStack { panel: Panel::Layers, height: Some(640.0) },
                "side" => Action::Move { panel: Panel::Properties, anchor: Panel::ToolSets, placement: Placement::Split(Zone::Bottom) },
                // Layers and Properties share a stack, not a tab group; tabbing Properties onto Layers changes nothing.
                "tab_group" => Action::Move { panel: Panel::Properties, anchor: Panel::ToolSets, placement: Placement::Tab { before: None } },
                "float_width" => Action::MoveFloating { panel: Panel::Properties, rect: [128.0, 80.0, 640.0, 240.0] },
                "float_height" => Action::MoveFloating { panel: Panel::Properties, rect: [128.0, 80.0, 320.0, 400.0] },
                "x_ratio" => Action::MoveFloating { panel: Panel::Properties, rect: [640.0, 80.0, 320.0, 240.0] },
                _ => Action::MoveFloating { panel: Panel::Properties, rect: [128.0, 400.0, 320.0, 240.0] },
            };
            malicious.apply(action).unwrap();
            assert_ne!(malicious.layout, expected, "{field}");
            let mut tree = serde_json::to_value(&malicious).unwrap();
            cadcraft_ui_egui::wsprofile::merge::strip_nulls(&mut tree);
            let mut incoming = app.ws.user.clone();
            incoming.layout.dock_tree = Some(tree.clone());
            match route {
                "set" => {
                    run(&mut app, "ui.workspace.set", json!({"pointer": "/dock_tree", "value": tree}));
                }
                "import" => {
                    run(&mut app, "ui.workspace.import", json!({"text": serde_json::to_string(&incoming).unwrap()}));
                }
                "restore" => {
                    let set = ProfileSet { active: app.ws.active.clone(), user: Some(incoming), ..Default::default() };
                    app.ws.store.as_mut().unwrap().push_snapshot(100, &serde_json::to_string(&set).unwrap()).unwrap();
                    run(&mut app, "ui.workspace.restore", json!({"revision": 100}));
                }
                _ => {
                    incoming.etag = Some("server:2".into());
                    let remote = MemoryRemote {
                        profiles: std::collections::BTreeMap::from([(incoming.profile_id.clone(), incoming)]),
                        online: true,
                        ..Default::default()
                    };
                    app.attach_remote(Box::new(remote));
                    app.sync_now();
                    assert_eq!(app.ws.status, SaveStatus::Conflict);
                    app.resolve_conflict(Resolution::UseServer);
                    assert_eq!(app.ws.status, SaveStatus::Synced);
                }
            }
            assert_eq!(app.ui.docking.layout, expected, "{route}: {field} must respect policy");
            app.commit_workspace_now().unwrap();
            let again = restarted(&app, Some(policy));
            assert!(again.ws.fallback.is_none(), "{route}: {field}: {:?}", again.ws.fallback);
            assert_eq!(again.ui.docking.layout, expected, "{route}: {field} after restart");
        }
    }
}

#[test]
fn dock_tree_preserves_unlocked_geometry_with_panel_lock() {
    use craft_ui::docking::Action;

    let _aliases = alias_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/panels/properties/width_ratio"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..Default::default() });
    let mut tree = app.ui.docking.clone();
    tree.apply(Action::ResizeSplit { path: vec![], size: SplitSize::FixedFirst(320.0) }).unwrap();
    tree.apply(Action::ResizeStack { panel: Panel::Layers, height: Some(240.0) }).unwrap();
    let expected = tree.layout.clone();
    let mut value = serde_json::to_value(tree).unwrap();
    cadcraft_ui_egui::wsprofile::merge::strip_nulls(&mut value);
    run(&mut app, "ui.workspace.set", json!({"pointer": "/dock_tree", "value": value}));
    assert_eq!(app.ui.docking.layout, expected, "a width lock leaves other dimensions editable");
    app.commit_workspace_now().unwrap();
    let again = restarted(&app, Some(policy));
    assert_eq!(again.ui.docking.layout, expected);
}

#[test]
fn dock_tree_locked_geometry_tolerates_rounding() {
    use craft_ui::docking::Action;

    let _aliases = alias_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/panels/properties/width_ratio"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..Default::default() });
    let mut tree = app.ui.docking.clone();
    let width = (0.21 + 0.0000005) * app.ws.area.width();
    tree.apply(Action::ResizeSplit { path: vec![true], size: SplitSize::FixedSecond(width) }).unwrap();
    let expected = tree.layout.clone();
    let mut value = serde_json::to_value(tree).unwrap();
    cadcraft_ui_egui::wsprofile::merge::strip_nulls(&mut value);
    run(&mut app, "ui.workspace.set", json!({"pointer": "/dock_tree", "value": value}));
    assert_eq!(app.ui.docking.layout, expected, "normalized rounding within tolerance must keep the tree");
}

fn two_workspace_writers() -> (Shared, CadApp, CadApp) {
    let shared = Shared::new(MemoryStore::default());
    let mut a = reattach(&shared);
    a.commit_workspace_now().unwrap();
    let b = reattach(&shared);
    assert_eq!(b.ws.known_revision, a.ws.known_revision);
    (shared, a, b)
}

fn conflicting_workspace_writers() -> (Shared, CadApp, CadApp) {
    let (shared, mut a, mut b) = two_workspace_writers();
    run(&mut a, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 5}));
    a.commit_workspace_now().unwrap();
    run(&mut b, "ui.cmdline.lines", json!({"lines": 7}));
    assert!(b.commit_workspace_now().unwrap_err().contains("changed in another tab"));
    (shared, a, b)
}

#[test]
fn second_writer_detects_conflict_and_does_not_overwrite() {
    let _aliases = alias_lock();
    let (shared, a, mut b) = conflicting_workspace_writers();
    assert_eq!(b.ws.status, SaveStatus::TabConflict);
    assert_eq!(b.ws.status.as_str(), "tab_conflict");
    assert_eq!(b.ws.status.label(), "Changed in another tab");
    assert_eq!(b.ui.dialog.as_deref(), Some("workspace_tab_conflict"));
    assert!(!b.ws.autosave.pending());
    assert!(b.ws.needs_flush(), "the unresolved layout is still local");
    assert_eq!(current_set(&b).user.unwrap().layout.command_line.lines, 5);
    assert_eq!(b.ws.user.layout.command_line.lines, 7);
    assert!(b.ws.known_revision < a.ws.known_revision);
    let before = shared.0.borrow().clone();
    let ctx = egui::Context::default();
    shared.1.set(true);
    for time in [1.0, 10.0, 100.0] {
        frame(&mut b, &ctx, time);
    }
    assert_eq!(b.ws.status, SaveStatus::TabConflict);
    assert!(!b.ws.autosave.pending());
    assert_eq!(shared.0.borrow().current, before.current);
    assert_eq!(shared.0.borrow().snapshot_map, before.snapshot_map);
}

#[test]
fn resolve_keep_mine_overwrites_after_confirm() {
    let _aliases = alias_lock();
    let (_, a, mut b) = conflicting_workspace_writers();
    run(&mut b, "ui.workspace.resolve", json!({"keep": "mine"}));
    let user = current_set(&b).user.unwrap();
    assert_eq!(user.layout.command_line.lines, 7);
    assert!(user.revision > a.ws.known_revision);
    assert_eq!(b.ws.known_revision, user.revision);
    assert_eq!(b.ws.status, SaveStatus::SavedLocally);
    assert!(!b.ws.autosave.pending());
    assert!(b.ui.dialog.is_none());
}

#[test]
fn resolve_keep_theirs_adopts_the_stored_set() {
    let _aliases = alias_lock();
    let (shared, a, mut b) = conflicting_workspace_writers();
    let before = shared.0.borrow().clone();
    run(&mut b, "ui.workspace.resolve", json!({"keep": "theirs"}));
    assert_eq!(b.ws.user.layout.command_line.lines, 5);
    assert_eq!(b.ws.known_revision, a.ws.known_revision);
    assert_eq!(b.ws.status, SaveStatus::SavedLocally);
    assert!(!b.ws.autosave.pending());
    assert!(b.ui.dialog.is_none());
    assert_eq!(shared.0.borrow().current, before.current);
    assert_eq!(shared.0.borrow().snapshot_map, before.snapshot_map);
}

#[test]
fn changed_elsewhere_without_local_edits_adopts_silently() {
    let _aliases = alias_lock();
    let (shared, mut a, mut b) = two_workspace_writers();
    b.set_session_layer(json!({"status_bar": {"visible": false}}));
    run(&mut a, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 5}));
    a.commit_workspace_now().unwrap();
    let before = shared.0.borrow().clone();
    shared.1.set(true);
    frame(&mut b, &egui::Context::default(), 1.0);
    assert_eq!(b.ws.user.layout.command_line.lines, 5);
    assert_eq!(b.ws.known_revision, a.ws.known_revision);
    assert_eq!(b.ws.message.as_deref(), Some("Updated from another tab"));
    assert!(b.ui.dialog.is_none(), "adoption does not open a dialog");
    assert!(!b.ui.show_status_bar, "session overrides survive adoption");
    assert!(!b.ws.autosave.pending());
    assert!(!shared.1.get());
    assert_eq!(shared.0.borrow().current, before.current);
    assert_eq!(shared.0.borrow().snapshot_map, before.snapshot_map);
    b.ui.dialog = Some("units".into());
    run(&mut a, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 6}));
    a.commit_workspace_now().unwrap();
    shared.1.set(true);
    frame(&mut b, &egui::Context::default(), 2.0);
    assert_eq!(b.ws.user.layout.command_line.lines, 6);
    assert_eq!(b.ui.dialog.as_deref(), Some("units"), "the current dialog stays open");
}

#[test]
fn programmatic_commit_conflict_returns_error_without_dialog() {
    let _aliases = alias_lock();
    let (shared, mut a, mut b) = two_workspace_writers();
    run(&mut a, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 5}));
    a.commit_workspace_now().unwrap();
    let before = shared.0.borrow().clone();
    let error = b.run("ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 7})).unwrap_err();
    assert!(error.contains("changed in another tab"), "{error}");
    assert_eq!(b.ws.status, SaveStatus::TabConflict);
    assert!(b.ui.dialog.is_none());
    assert!(!b.ws.autosave.pending());
    for command in ["ui.workspace.select", "ui.workspace.saveas", "ui.workspace.import", "ui.workspace.restore"] {
        let params = match command {
            "ui.workspace.select" => json!({"id": "AUTOCAD_FAMILIAR"}),
            "ui.workspace.saveas" => json!({"name": "Mine"}),
            "ui.workspace.import" => json!({"text": SPEC_SAMPLE}),
            _ => json!({"revision": a.ws.known_revision}),
        };
        let error = b.run(command, params).unwrap_err();
        assert!(error.contains("changed in another tab"), "{command}: {error}");
        assert!(b.ui.dialog.is_none());
    }
    assert_eq!(shared.0.borrow().current, before.current);
    assert_eq!(shared.0.borrow().snapshot_map, before.snapshot_map);
}

#[test]
fn external_change_with_pending_edits_waits_for_commit() {
    let _aliases = alias_lock();
    let (shared, mut a, mut b) = two_workspace_writers();
    run(&mut b, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 7}));
    run(&mut a, "ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 5}));
    a.commit_workspace_now().unwrap();
    shared.1.set(true);
    let ctx = egui::Context::default();
    frame(&mut b, &ctx, 0.1);
    assert_eq!(b.ws.user.layout.command_line.lines, 7);
    assert!(b.ui.dialog.is_none());
    frame(&mut b, &ctx, 1.0);
    assert_eq!(b.ws.status, SaveStatus::TabConflict);
    assert_eq!(b.ui.dialog.as_deref(), Some("workspace_tab_conflict"));
    assert!(!b.ws.autosave.pending());
    assert_eq!(current_set(&b).user.unwrap().layout.command_line.lines, 5);
}

#[test]
fn stored_revision_reads_only_bounded_unsigned_revision() {
    use cadcraft_ui_egui::wsprofile::startup::stored_revision;

    assert_eq!(stored_revision(r#"{"user":{"revision":42,"layout":"invalid"}}"#), Some(42));
    assert_eq!(stored_revision(r#"{"user":{"revision":18446744073709551615}}"#), Some(u64::MAX));
    for raw in ["{", "[]", "{}", r#"{"user":null}"#, r#"{"user":{"revision":-1}}"#, r#"{"user":{"revision":"1"}}"#] {
        assert_eq!(stored_revision(raw), None, "{raw}");
    }
    assert_eq!(stored_revision(&" ".repeat(cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES + 1)), None);
}

#[test]
fn workspace_resolution_requires_a_valid_choice_and_conflict() {
    let _aliases = alias_lock();
    let (_, _, mut b) = two_workspace_writers();
    for params in [Value::Null, json!({}), json!({"keep": "both"}), json!({"keep": "mine"}), json!({"keep": "theirs"})] {
        assert!(b.run("ui.workspace.resolve", params).is_err());
        assert!(b.ui.dialog.is_none());
    }
    assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|(id, label, menu, shortcut)| {
        *id == "ui.workspace.resolve" && *label == "Resolve Workspace Conflict" && menu.is_empty() && shortcut.is_none()
    }));
}
