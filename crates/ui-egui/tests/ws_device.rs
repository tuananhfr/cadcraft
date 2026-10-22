use cadcraft_engine::Session;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::wsprofile::canonical::{ChecksumCheck, verify_checksum, with_checksum};
use cadcraft_ui_egui::wsprofile::device::{DeviceState, split_geometry};
use cadcraft_ui_egui::wsprofile::merge::{Layer, Level, apply_patch, diff_patch, effective};
use cadcraft_ui_egui::wsprofile::startup::{parse_set, policy_layers, policy_problem};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use craft_ui::docking::Action;
use serde_json::{Value, json};

fn attached(current: Option<String>) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore { current, ..Default::default() }));
    app
}

fn run(app: &mut CadApp, command: &str, params: Value) -> Value {
    app.run(command, params).unwrap()
}

fn frame(app: &mut CadApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    let raw = egui::RawInput { time: Some(time), events, ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| app.logic(ui.ctx()));
    output.textures_delta.clear();
}

fn panel_geometry(patch: &Value, id: &str) -> Value {
    patch.get("panels").and_then(Value::as_array).unwrap().iter().find(|panel| panel.get("id") == Some(&json!(id))).unwrap().clone()
}

#[test]
fn device_layer_sits_between_user_and_session() {
    let layer = |level, height| Layer { level, patch: json!({"command_line": {"height_ratio": height}}), locks: Vec::new() };
    let mut layers = vec![layer(Level::Session, 0.4), layer(Level::Device, 0.3), layer(Level::User, 0.2)];
    assert_eq!(effective(&layers).layout.command_line.height_ratio, 0.4);
    layers.retain(|layer| layer.level != Level::Session);
    assert_eq!(effective(&layers).layout.command_line.height_ratio, 0.3);
    assert!(Level::User < Level::Device && Level::Device < Level::Session);
    assert_eq!(serde_json::to_value(Level::Device).unwrap(), json!("device"));
}

#[test]
fn dragging_a_panel_writes_device_not_user() {
    let mut app = attached(None);
    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "properties", "rect": [100, 100, 320, 240]}));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 1.0, vec![]);
    let at = egui::pos2(600.0, 400.0);
    let button =
        |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() };
    frame(&mut app, &ctx, 1.1, vec![egui::Event::PointerMoved(at), button(true)]);
    for (x, time) in [(200.0, 1.15), (240.0, 1.2), (280.0, 1.25)] {
        app.ui.docking.apply(Action::MoveFloating { panel: Panel::Properties, rect: [x, 150.0, 320.0, 240.0] }).unwrap();
        frame(&mut app, &ctx, time, vec![]);
    }
    frame(&mut app, &ctx, 1.3, vec![button(false)]);
    let user = &app.ws.layers.iter().find(|layer| layer.level == Level::User).unwrap().patch;
    assert!(panel_geometry(user, "properties").get("x_ratio").is_none());
    assert!(panel_geometry(&app.ws.device.layout_patch, "properties").get("x_ratio").is_some());
    assert!(user.get("dock_tree").is_none());
    assert!(app.ws.device.layout_patch.get("dock_tree").is_some());
    let store = app.ws.store.take().unwrap();
    let mut restarted = attached(None);
    restarted.attach_profile_store(store);
    assert!(restarted.ws.fallback.is_none());
    assert_eq!(restarted.ws.device.layout_patch, app.ws.device.layout_patch);
}

#[test]
fn ui_scale_lives_on_the_device() {
    let mut app = attached(None);
    run(&mut app, "ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.25}));
    assert_eq!(app.ws.device.ui_scale, 1.25);
    assert!(serde_json::to_value(&app.ws.user).unwrap().get("ui_scale").is_none());
    run(&mut app, "ui.workspace.set", json!({"scope": "device", "pointer": "/command_line/height_ratio", "value": 0.3}));
    assert_eq!(app.ws.device.layout_patch.pointer("/command_line/height_ratio").unwrap().as_f64().unwrap() as f32, 0.3);
    assert_eq!(app.ws.effective.layout.command_line.height_ratio, 0.3);
    app.commit_workspace_now().unwrap();
    let raw = app.ws.store.as_ref().unwrap().read_current().unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(verify_checksum(&value), ChecksumCheck::Valid);
    assert_eq!(value.pointer("/deviceGeometry/uiScale"), Some(&json!(1.25)));
    assert!(value.pointer("/user/uiScale").is_none());
    assert!(value.pointer("/user/layoutPatch/commandLine/heightRatio").is_none());
    let set = parse_set(&raw).unwrap();
    assert_eq!(set.device.unwrap().ui_scale, 1.25);
}

#[test]
fn policy_cannot_target_the_device_level() {
    let policy = json!({"layers": [
        {"level": "device", "patch": {"command_line": {"height_ratio": 0.8}}, "locks": ["/command_line"]},
        {"level": "company", "patch": {"command_line": {"lines": 7}}}
    ]})
    .to_string();
    let layers = policy_layers(&policy);
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].level, Level::Company);
    assert!(policy_problem(&policy).is_some());
    let mut app = attached(None);
    app.attach_profile_store(Box::new(MemoryStore { policy: Some(policy), ..Default::default() }));
    assert_eq!(app.ws.effective.layout.command_line.lines, 7);
    assert_eq!(app.ws.effective.layout.command_line.height_ratio, 0.1);
    assert!(app.ws.effective.locked.is_empty());
}

fn assert_portable(value: &Value) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                assert!(
                    ![
                        "ui_scale",
                        "uiScale",
                        "device",
                        "deviceGeometry",
                        "x_ratio",
                        "xRatio",
                        "y_ratio",
                        "yRatio",
                        "width_ratio",
                        "widthRatio",
                        "height_ratio",
                        "heightRatio",
                        "dock_tree",
                        "dockTree"
                    ]
                    .contains(&key.as_str()),
                    "export includes {key}"
                );
                assert_portable(child);
            }
        }
        Value::Array(items) => items.iter().for_each(assert_portable),
        _ => {}
    }
}

#[test]
fn export_has_no_device_geometry() {
    let mut app = attached(None);
    run(&mut app, "ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.5}));
    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "layers", "rect": [200, 150, 320, 240]}));
    let exported = run(&mut app, "ui.workspace.export", json!({}));
    let value: Value = serde_json::from_str(exported["text"].as_str().unwrap()).unwrap();
    assert_portable(&value);
    assert_eq!(panel_geometry(&value["layout"], "layers").get("mode"), Some(&json!("float")));
    assert!(exported["ignored"].as_array().unwrap().iter().any(|pointer| pointer.as_str() == Some("/layout/panels/layers/x_ratio")));
}

#[test]
fn import_keeps_this_devices_geometry() {
    let mut app = attached(None);
    run(&mut app, "ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.25}));
    run(&mut app, "ui.dock", json!({"operation": "float", "panel": "properties", "rect": [120, 160, 320, 240]}));
    app.commit_workspace_now().unwrap();
    let device = app.ws.device.clone();
    let incoming = json!({
        "schema_version": "1.0.0", "ui_scale": 2.0,
        "deviceGeometry": {"ui_scale": 2.0},
        "layout": {
            "command_line": {"lines": 7, "height_ratio": 0.9},
            "panels": [{"id": "properties", "mode": "float", "x_ratio": 0.9, "y_ratio": 0.9}],
            "dock_tree": {"other_screen": true}
        }
    })
    .to_string();
    for dry_run in [true, false] {
        let result = run(&mut app, "ui.workspace.import", json!({"text": incoming, "dry_run": dry_run}));
        assert_eq!(app.ws.device, device);
        let ignored = result["ignored"].as_array().unwrap();
        for pointer in ["/ui_scale", "/deviceGeometry", "/layout/command_line/height_ratio", "/layout/panels/properties/x_ratio"] {
            assert!(ignored.contains(&json!(pointer)), "{result}");
        }
        assert!(!result["changes"].as_array().unwrap().contains(&json!("/ui_scale")));
    }
    assert_eq!(app.ws.user.layout.command_line.lines, 7);
}

#[test]
fn legacy_scale_and_geometry_move_to_device_before_a_signed_commit() {
    let profile = json!({
        "schema_version": "1.0.0", "profile_id": "user.legacy", "base_id": "SIMPLE", "ui_scale": 1.5,
        "layout_patch": {"command_line": {"lines": 7, "height_ratio": 0.3}}
    });
    let saved = json!({"schema_version": "1.0.0", "active": "user.legacy", "profiles": [profile.clone()]});
    let working = json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": profile.clone()});
    for value in [profile, saved, working] {
        let mut app = attached(Some(with_checksum(value).to_string()));
        assert!(app.ws.fallback.is_none(), "{:?}", app.ws.fallback);
        assert_eq!(app.ws.device.ui_scale, 1.5);
        assert_eq!(app.ws.effective.layout.command_line.height_ratio, 0.3);
        assert_eq!(app.ws.user.layout.command_line.lines, 7);
        assert!(serde_json::to_value(&app.ws.user).unwrap().get("ui_scale").is_none());
        app.commit_workspace_now().unwrap();
        let store = app.ws.store.take().unwrap();
        app.attach_profile_store(store);
        assert!(app.ws.fallback.is_none());
        assert_eq!(app.ws.device.ui_scale, 1.5);
        assert_eq!(app.ws.effective.layout.command_line.height_ratio, 0.3);
    }
}

#[test]
fn split_geometry_preserves_keyed_identity_and_user_choices() {
    let (user, device) = split_geometry(json!({
        "panels": [{"id": "layers", "mode": "float", "visible": false, "x_ratio": 0.4, "width_ratio": 0.3}],
        "toolbars": [{"id": "draw", "y_ratio": 0.2}], "command_line": {"height_ratio": 0.3, "lines": 7},
        "dock_tree": {"layout": {}}, "aliases": {"x_ratio": "line"}
    }));
    assert_eq!(
        user,
        json!({
            "panels": [{"id": "layers", "mode": "float", "visible": false}],
            "toolbars": [{"id": "draw"}], "command_line": {"lines": 7}, "aliases": {"x_ratio": "line"}
        })
    );
    assert_eq!(
        device,
        json!({
            "panels": [{"id": "layers", "x_ratio": 0.4, "width_ratio": 0.3}],
            "toolbars": [{"id": "draw", "y_ratio": 0.2}], "command_line": {"height_ratio": 0.3}, "dock_tree": {"layout": {}}
        })
    );
    assert_eq!(DeviceState::default().ui_scale, 1.0);
    let base = cadcraft_ui_egui::wsprofile::schema::WorkspaceLayout::default();
    let mut reordered = base.clone();
    reordered.panels.reverse();
    reordered.panels.first_mut().unwrap().x_ratio = 0.4;
    let (user, device) = split_geometry(diff_patch(&base, &reordered));
    assert_eq!(apply_patch(&apply_patch(&base, &user), &device), reordered);
}

#[test]
fn legacy_fixtures_keep_device_preferences() {
    for text in [include_str!("fixtures/workspace/aged-5y.json"), include_str!("fixtures/workspace/legacy-v1.json")] {
        let mut app = attached(Some(text.into()));
        assert!(app.ws.fallback.is_none(), "{:?}", app.ws.fallback);
        if text.contains("ui_scale") {
            assert_eq!(app.ws.device.ui_scale, 1.25);
        } else {
            let value: Value = serde_json::from_str(text).unwrap();
            assert_eq!(app.ws.device.layout_patch.get("dock_tree"), value.pointer("/workspace/docking"));
        }
        app.commit_workspace_now().unwrap();
        let store = app.ws.store.take().unwrap();
        app.attach_profile_store(store);
        assert!(app.ws.fallback.is_none());
    }
}
