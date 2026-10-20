//! Workspace persistence in the running app: commits, snapshots, startup recovery, layout undo and
//! the `ui.workspace.*` commands. Storage is a `MemoryStore`; the hosts' own stores have their tests.

use cadcraft_engine::Session;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::i18n::Preference;
use cadcraft_ui_egui::theme::ThemePref;
use cadcraft_ui_egui::wsprofile::SaveStatus;
use cadcraft_ui_egui::wsprofile::merge::Level;
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::{Dock, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::startup::{FALLBACK_MESSAGE, Fallback};
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, MAX_USER_PROFILES, MemoryStore, ProfileSet, ProfileStore};
use cadcraft_ui_egui::wsprofile::validate::{Mode, validate};
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};

const SPEC_SAMPLE: &str = r#"{"schema_version":"1.0.0","profile_id":"uuid","revision":12,"base_id":"AUTOCAD_FAMILIAR","panels":[{"id":"properties","mode":"dock","side":"right","width_ratio":0.24}],"command_line":{"side":"bottom","height_ratio":0.15},"shortcuts":{"line":"L"}}"#;

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
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
    serde_json::from_str(&store(app).read_current().unwrap()).unwrap()
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
    let mut app = attached(MemoryStore { current: Some("{".into()), ..MemoryStore::default() });
    assert!(matches!(app.ws.fallback, Some(Fallback::Corrupt(_))), "{:?}", app.ws.fallback);
    assert_eq!(app.ws.user.base_id.as_deref(), Some("SIMPLE"));
    assert_eq!(app.ws.effective.layout, preset("SIMPLE").unwrap().layout);
    assert_eq!(store(&app).read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
    assert_eq!(FALLBACK_MESSAGE, "Workspace could not be read; using SIMPLE. The damaged copy was kept.");
    assert_eq!(app.ws.message.as_deref(), Some(FALLBACK_MESSAGE));
    assert_eq!(app.status.as_ref().map(|(text, _)| text.as_str()), Some(FALLBACK_MESSAGE));
    assert_eq!(run(&mut app, "ui.workspace.status", json!({}))["message"], json!(FALLBACK_MESSAGE));
    // The app keeps working, and its next save keeps the damaged copy.
    run(&mut app, "ui.toggle.toolbar", json!({"on": false}));
    app.commit_workspace_now().unwrap();
    assert_eq!(store(&app).read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
    assert!(!standard_toolbar_visible(&current_set(&app).user.unwrap()));

    let mut newer = preset("SIMPLE").unwrap();
    newer.schema_version = "2.0.0".into();
    let newer = serde_json::to_string(&json!({"active": "SIMPLE", "user": newer})).unwrap();
    let app = attached(MemoryStore { current: Some(newer), ..MemoryStore::default() });
    assert!(matches!(app.ws.fallback, Some(Fallback::NewerMajor(ref version)) if version == "2.0.0"), "{:?}", app.ws.fallback);

    let invalid = json!({"active": "SIMPLE", "user": {"schema_version": "1.0.0", "name": "x".repeat(2000)}}).to_string();
    let app = attached(MemoryStore { current: Some(invalid.clone()), ..MemoryStore::default() });
    assert!(matches!(app.ws.fallback, Some(Fallback::Invalid(_))), "{:?}", app.ws.fallback);
    assert_eq!(store(&app).read_snapshot(CORRUPT_SNAPSHOT), Some(invalid));
    assert_eq!(app.ws.effective.layout, preset("SIMPLE").unwrap().layout);
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
