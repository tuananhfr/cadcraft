use cadcraft_engine::Session;
use cadcraft_engine::hash::sha256_hex;
use cadcraft_ui_egui::wsprofile::canonical::{CHECKSUM_KEY, ChecksumCheck, canonical_json, checksum_of, verify_checksum, with_checksum};
use cadcraft_ui_egui::wsprofile::startup::{Fallback, fresh_user, parse_set, restore};
use cadcraft_ui_egui::wsprofile::store::{MAX_SET_BYTES, MemoryStore, ProfileSet, ProfileStore};
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};

#[test]
fn canonical_json_sorts_keys_at_every_depth() {
    let value: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":null}}"#).unwrap();
    assert_eq!(canonical_json(&value), r#"{"a":{"c":null,"d":[{"y":2,"z":1}]},"b":1}"#);
}

#[test]
fn checksum_ignores_its_own_field_and_key_order() {
    let first: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":null}}"#).unwrap();
    let second: Value = serde_json::from_str(r#"{"a":{"c":null,"d":[{"y":2,"z":1}]},"b":1}"#).unwrap();
    let checksum = checksum_of(&first);
    assert_eq!(checksum, checksum_of(&second));
    assert_eq!(checksum, format!("sha256:{}", sha256_hex(canonical_json(&first).as_bytes())));

    let mut signed = with_checksum(first);
    assert_eq!(signed.get(CHECKSUM_KEY).and_then(Value::as_str), Some(checksum.as_str()));
    assert_eq!(verify_checksum(&signed), ChecksumCheck::Valid);
    assert_eq!(checksum_of(&signed), checksum);
    assert_eq!(with_checksum(signed.clone()), signed);
    signed[CHECKSUM_KEY] = json!("edited");
    assert_eq!(checksum_of(&signed), checksum);
    assert_eq!(verify_checksum(&signed), ChecksumCheck::Mismatch { stored: "edited".into(), actual: checksum });
}

#[test]
fn checksum_mismatch_is_corrupt() {
    let mut value = with_checksum(serde_json::to_value(ProfileSet::default()).unwrap());
    assert!(parse_set(&value.to_string()).is_ok());
    value["active"] = json!("ARCHITECTURE");
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));

    value["schema_version"] = json!("99.0.0");
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));
    let signed_newer = with_checksum(value);
    assert!(matches!(parse_set(&signed_newer.to_string()), Err(Fallback::NewerMajor(version)) if version == "99.0.0"));
}

#[test]
fn set_without_checksum_still_reads() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/workspace/aged-1y.json")).unwrap();
    let value = json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": fixture});
    assert_eq!(verify_checksum(&value), ChecksumCheck::Missing);
    let set = parse_set(&value.to_string()).unwrap();
    let user = set.user.unwrap();
    assert_eq!(set.active, "SIMPLE");
    assert_eq!(user.profile_id, "user.aged-1y");
    assert_eq!(user.revision, 4);
    assert_eq!(user.schema_version, 3);
    assert!(parse_set(include_str!("fixtures/workspace/aged-1y.json")).is_ok());
    assert!(parse_set(include_str!("fixtures/workspace/legacy-v1.json")).is_ok());
}

#[test]
fn canonical_json_preserves_array_order_strings_and_numbers() {
    let value: Value = serde_json::from_str(r#"{"z":[true,false,null,1.0,-2,1e30],"a":"quote\" newline\n slash\\ Tiếng Việt"}"#).unwrap();
    let text = canonical_json(&value);
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), value);
    assert_eq!(canonical_json(&json!([])), "[]");
    assert_eq!(canonical_json(&json!({})), "{}");
    assert_eq!(canonical_json(&json!([2, 1])), "[2,1]");
    assert_eq!(canonical_json(&json!(1.0)), "1.0");
}

#[test]
fn checksum_includes_nested_checksum_fields() {
    let first = json!({"nested": {"checksum": "first"}});
    let second = json!({"nested": {"checksum": "second"}});
    assert_ne!(checksum_of(&first), checksum_of(&second));
}

#[test]
fn malformed_checksum_is_a_mismatch() {
    for stored in [Value::Null, json!(false), json!(12), json!([]), json!({}), json!("sha256:bad")] {
        let value = json!({"active": "SIMPLE", "checksum": stored});
        assert!(matches!(verify_checksum(&value), ChecksumCheck::Mismatch { .. }));
        assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));
    }
}

#[test]
fn commit_writes_valid_checksums_for_current_and_snapshot() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    for _ in 0..2 {
        app.commit_workspace_now().unwrap();
        let store = app.ws.store.as_deref().unwrap();
        let raw = store.read_current().unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(verify_checksum(&value), ChecksumCheck::Valid);
        assert!(value.get("schema_version").is_none());
        assert_eq!(value["schemaVersion"], json!(3));
        let set = parse_set(&raw).unwrap();
        assert!(!set.extra.contains_key(CHECKSUM_KEY));
        assert_eq!(store.read_snapshot(set.user.unwrap().revision), Some(raw));
    }
}

#[test]
fn checksum_damage_restores_the_last_valid_snapshot() {
    let mut user = fresh_user("SIMPLE", &[]);
    user.revision = 7;
    let value = with_checksum(json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": user}));
    let mut damaged = value.clone();
    damaged["active"] = json!("ARCHITECTURE");
    let raw = damaged.to_string();
    let mut store = MemoryStore { current: Some(raw.clone()), ..MemoryStore::default() };
    store.push_snapshot(7, &value.to_string()).unwrap();
    store.push_snapshot(8, &raw).unwrap();

    let restored = restore(&mut store);
    assert!(matches!(restored.fallback, Some(Fallback::RestoredSnapshot { revision: 7, reason }) if reason.contains("checksum")));
    assert_eq!(restored.user.revision, 7);
    assert_eq!(restored.set.active, "SIMPLE");
    assert_eq!(store.read_current(), Some(raw.clone()));
    assert!(store.damaged.values().any(|bytes| bytes == raw.as_bytes()));
}

#[test]
fn oversized_set_is_rejected_before_checksum_verification() {
    let value = json!({"checksum": "invalid", "padding": "x".repeat(MAX_SET_BYTES)});
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("larger than")));
}

#[test]
fn gd3_fields_survive_v1_to_v3_and_back() {
    use cadcraft_ui_egui::wsprofile::migrate::migrate;
    use cadcraft_ui_egui::wsprofile::wire::{from_wire, to_wire};
    let old = json!({"schema_version": "1.0.0", "layout": {
        "ribbon": {"mode": "tabs_only"}, "quick_access_position": "below_ribbon", "density": "comfortable"
    }});
    let migrated = migrate(old).unwrap().value;
    let wire = to_wire(&migrated);
    assert_eq!(wire["schemaVersion"], 3);
    assert_eq!(wire["layout"]["ribbon"]["ribbonMode"], "tabs_only");
    assert_eq!(wire["layout"]["quickAccessPosition"], "below_ribbon");
    assert_eq!(wire["layout"]["density"], "comfortable");
    assert_eq!(from_wire(&wire), migrated);

    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore { current: Some(wire.to_string()), ..Default::default() }));
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    app.commit_workspace_now().unwrap();
    let raw = app.ws.store.as_ref().unwrap().read_current().unwrap();
    let saved = parse_set(&raw).unwrap().user.unwrap();
    let layout = serde_json::to_value(saved.layout).unwrap();
    assert_eq!(layout["ribbon"]["mode"], "tabs_only");
    assert_eq!(layout["quick_access_position"], "below_ribbon");
    assert_eq!(layout["density"], "comfortable");
}

#[test]
fn v3_set_is_camel_case_with_version_3() {
    use cadcraft_ui_egui::wsprofile::wire::is_wire;
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 5})).unwrap();
    app.run("ui.workspace.set", json!({"scope": "device", "pointer": "/command_line/height_ratio", "value": 0.3})).unwrap();
    app.run("ui.workspace.saveas", json!({"name": "Wire fixture"})).unwrap();
    let raw = app.ws.store.as_ref().unwrap().read_current().unwrap();
    let wire: Value = serde_json::from_str(&raw).unwrap();
    assert!(is_wire(&wire));
    assert_eq!(wire["schemaVersion"], 3);
    assert!(wire["profiles"][0]["profileId"].is_string());
    assert_eq!(wire["profiles"][0]["scope"], "user");
    assert_eq!(wire["user"]["layoutPatch"]["commandLine"]["lines"], 5);
    assert!(wire["deviceGeometry"]["layoutPatch"]["commandLine"]["heightRatio"].is_number());
    assert!(wire["deviceGeometry"]["uiScale"].is_number());
    assert_eq!(verify_checksum(&wire), ChecksumCheck::Valid);
    fn assert_fields(value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, child) in fields {
                    assert!(!key.contains('_'), "unexpected field {key}");
                    if !matches!(key.as_str(), "shortcuts" | "aliases" | "items" | "dockTree") {
                        assert_fields(child);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_fields(item);
                }
            }
            _ => {}
        }
    }
    assert_fields(&wire);
}

#[test]
fn wire_round_trip_is_lossless_for_every_fixture() {
    use cadcraft_ui_egui::wsprofile::migrate::migrate;
    use cadcraft_ui_egui::wsprofile::wire::{from_wire, to_wire};
    for raw in [
        include_str!("fixtures/workspace/legacy-v1.json"),
        include_str!("fixtures/workspace/aged-1y.json"),
        include_str!("fixtures/workspace/aged-3y.json"),
        include_str!("fixtures/workspace/aged-5y.json"),
        include_str!("fixtures/workspace/aged-10y.json"),
        include_str!("fixtures/workspace/future-1.3.0.json"),
    ] {
        let original: Value = serde_json::from_str(raw).unwrap();
        if original.get("ui_scale").is_none() {
            assert_eq!(from_wire(&to_wire(&original)), original);
        }
        let value = migrate(original).unwrap().value;
        assert_eq!(from_wire(&to_wire(&value)), value);
    }
    let value = json!({"schema_version": 3, "futureThing": {"command_line": 1}, "Weird_Key": {"profile_id": "x"}, "layout": {
        "shortcuts": {"command_line": "Ctrl+Shift+S"}, "aliases": {"Weird_Key": "line"},
        "status_bar": {"items": {"height_ratio": true}}, "dock_tree": {"x_ratio": {"tab_group": "mode"}},
        "panels": [{"id": "p", "mode": "auto_hide", "futureThing": {"command_line": 7}}]
    }});
    assert_eq!(from_wire(&to_wire(&value)), value);
    assert_eq!(to_wire(&value)["layout"]["panels"][0]["mode"], "auto_hide");
    assert_eq!(to_wire(&value)["layout"]["dockTree"], value["layout"]["dock_tree"]);
    assert!(to_wire(&json!({"schema_version": 3, "ui_scale": 1.5})).get("uiScale").is_none());
}

#[test]
fn policy_accepts_both_spellings() {
    use cadcraft_ui_egui::wsprofile::startup::{policy_layers, policy_problem};
    let wire = json!({"layers": [{"level": "company", "patch": {"commandLine": {"lines": 5}}, "locks": ["/commandLine/lines"]}]}).to_string();
    let snake = json!({"layers": [{"level": "company", "patch": {"command_line": {"lines": 5}}, "locks": ["/command_line/lines"]}]}).to_string();
    assert_eq!(policy_layers(&wire), policy_layers(&snake));
    assert_eq!(policy_problem(&wire), None);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore { policy: Some(wire), ..Default::default() }));
    assert_eq!(app.ws.effective.layout.command_line.lines, 5);
    assert!(app.run("ui.workspace.set", json!({"pointer": "/command_line/lines", "value": 6})).is_err());
}

#[test]
fn schema_versions_accept_numbers_and_legacy_strings() {
    use cadcraft_ui_egui::wsprofile::schema::WorkspaceProfile;
    for (version, major) in [(json!(3), 3), (json!("3"), 3), (json!("1.0.0"), 1), (json!("1.2.0"), 1)] {
        let profile: WorkspaceProfile = serde_json::from_value(json!({"schema_version": version.clone()})).unwrap();
        let set: ProfileSet = serde_json::from_value(json!({"schema_version": version})).unwrap();
        assert_eq!(profile.schema_version, major);
        assert_eq!(set.schema_version, major);
    }
}

#[test]
fn unknown_nested_fields_survive_loading_and_saving() {
    let input = json!({"schemaVersion": 3, "user": {"schemaVersion": 3, "scope": "user", "layout": {
        "ribbon": {"futureThing": {"command_line": 7}},
        "commandLine": {"Weird_Key": {"height_ratio": 9}},
        "panels": [{"id": "properties", "futureThing": ["kept"]}],
        "view": {"futureThing": "kept"}
    }}, "Weird_Key": {"schema_version": "verbatim"}, "deviceGeometry": {
        "futureThing": {"command_line": 7}, "screen": {"Weird_Key": "kept"}
    }});
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore { current: Some(input.to_string()), ..Default::default() }));
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    app.commit_workspace_now().unwrap();
    let saved = parse_set(&app.ws.store.as_ref().unwrap().read_current().unwrap()).unwrap();
    let profile = serde_json::to_value(saved.user.unwrap()).unwrap();
    assert_eq!(profile["layout"]["ribbon"]["futureThing"], input["user"]["layout"]["ribbon"]["futureThing"]);
    assert_eq!(profile["layout"]["command_line"]["Weird_Key"], input["user"]["layout"]["commandLine"]["Weird_Key"]);
    let panel = profile["layout"]["panels"].as_array().unwrap().iter().find(|panel| panel["id"] == "properties").unwrap();
    assert_eq!(panel["futureThing"], json!(["kept"]));
    assert_eq!(profile["layout"]["view"]["futureThing"], "kept");
    assert_eq!(saved.extra.get("Weird_Key"), input.get("Weird_Key"));
    let device = saved.device.unwrap();
    assert_eq!(device.extra.get("futureThing"), input["deviceGeometry"].get("futureThing"));
    assert_eq!(device.screen.unwrap().extra.get("Weird_Key"), Some(&json!("kept")));
}
