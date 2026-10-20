//! Workspace profile migrations: legacy conversion, aged and future documents, corrupt input, rollback.

use cadcraft_ui_egui::wsprofile::migrate::{CHANGELOG, MigrateError, Migrated, STEPS, Step, from_legacy_v1, migrate, migrate_with};
use cadcraft_ui_egui::wsprofile::presets::{SAFE_PRESET, preset};
use cadcraft_ui_egui::wsprofile::schema::{OwnerType, SCHEMA_VERSION, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::validate::{Mode, validate};
use serde_json::{Value, json};

const LEGACY_V1: &str = include_str!("fixtures/workspace/legacy-v1.json");
const AGED_1Y: &str = include_str!("fixtures/workspace/aged-1y.json");
const AGED_3Y: &str = include_str!("fixtures/workspace/aged-3y.json");
const AGED_5Y: &str = include_str!("fixtures/workspace/aged-5y.json");
const AGED_10Y: &str = include_str!("fixtures/workspace/aged-10y.json");
const FUTURE_MINOR: &str = include_str!("fixtures/workspace/future-1.3.0.json");
const CORRUPT_TRUNCATED: &str = include_str!("fixtures/workspace/corrupt-truncated.json");
const CORRUPT_WRONGTYPES: &str = include_str!("fixtures/workspace/corrupt-wrongtypes.json");

const VALID_FIXTURES: [(&str, &str); 6] = [
    ("legacy-v1", LEGACY_V1),
    ("aged-1y", AGED_1Y),
    ("aged-3y", AGED_3Y),
    ("aged-5y", AGED_5Y),
    ("aged-10y", AGED_10Y),
    ("future-1.3.0", FUTURE_MINOR),
];

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn profile_of(value: &Value) -> WorkspaceProfile {
    serde_json::from_value(value.clone()).unwrap()
}

/// True when every object key, array element and scalar of `part` appears in `whole` at the same place.
fn contains(whole: &Value, part: &Value) -> bool {
    match (whole, part) {
        (Value::Object(whole), Value::Object(part)) => part.iter().all(|(key, value)| whole.get(key).is_some_and(|found| contains(found, value))),
        (Value::Array(whole), Value::Array(part)) => whole.len() == part.len() && whole.iter().zip(part).all(|(a, b)| contains(a, b)),
        _ => whole == part,
    }
}

fn strings_at<'a>(value: &'a Value, pointer: &str) -> Vec<&'a str> {
    value.pointer(pointer).and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

#[test]
fn legacy_v1_becomes_user_layer_over_simple() {
    let input = json_of(LEGACY_V1);
    let migrated = migrate(input.clone()).unwrap();
    assert_eq!(migrated.steps, vec!["legacy_v1"]);
    assert_eq!(migrated.backup, Some(input.clone()));
    assert_eq!(migrated.value, from_legacy_v1(&input));

    let profile = profile_of(&migrated.value);
    assert_eq!(profile.schema_version, SCHEMA_VERSION);
    assert_eq!(profile.owner_type, OwnerType::User);
    assert_eq!(profile.base_id.as_deref(), Some("SIMPLE"));
    assert_eq!(profile.layout.dock_tree.as_ref(), input.pointer("/workspace/docking"));

    let simple = preset("SIMPLE").unwrap();
    let standard = profile.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
    assert!(!standard.visible);
    let simple_standard = simple.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
    assert_eq!(standard.commands, simple_standard.commands);

    let layout = &profile.layout;
    assert!(layout.toolsets.visible);
    assert_eq!(layout.toolsets.tab, "Modeling");
    assert_eq!(layout.toolsets.collapsed_groups, ["Draw", "Modify"]);
    assert!(layout.status_bar.visible);
    assert!(layout.command_line.visible);
    assert_eq!(layout.command_line.lines, 5);
    // The view switches land in `layout.view`, not in `extra`.
    let view = &layout.view;
    assert!(!view.viewcube);
    assert!(view.ucs_icon);
    assert!(!view.layer_list);
    assert!(view.file_tabs);
    assert!(view.properties_all);
    assert!(!view.in_window_menu);
    for key in ["showViewcube", "showUcsIcon", "showLayerList", "showFileTabs", "propertiesAll", "inWindowMenu"] {
        assert!(!layout.extra.contains_key(key), "{key} stayed in extra");
    }

    let text = serde_json::to_string(&migrated.value).unwrap();
    assert!(validate(&text, Mode::Local).is_ok());
}

#[test]
fn legacy_hidden_palettes_hide_their_panels() {
    let input = json!({"version": 1, "workspace": {"showPalettes": false}});
    let profile = profile_of(&from_legacy_v1(&input));
    for id in ["layers", "properties"] {
        let panel = profile.layout.panels.iter().find(|panel| panel.id == id).unwrap();
        assert!(!panel.visible, "{id}");
    }
}

#[test]
fn legacy_values_of_the_wrong_type_are_kept_not_applied() {
    let input = json!({"version": 1, "workspace": {"showToolbar": "no", "historyLines": 99, "toolsetTab": 7, "showStatusBar": null}});
    let profile = profile_of(&from_legacy_v1(&input));
    let standard = profile.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
    assert!(standard.visible);
    assert_eq!(profile.layout.command_line.lines, 3);
    assert_eq!(profile.layout.extra.get("showToolbar"), Some(&json!("no")));
    assert_eq!(profile.layout.extra.get("historyLines"), Some(&json!(99)));
    assert_eq!(profile.layout.extra.get("toolsetTab"), Some(&json!(7)));
    assert_eq!(profile.layout.extra.get("showStatusBar"), Some(&json!(null)));
}

#[test]
fn migration_is_idempotent() {
    for (name, text) in VALID_FIXTURES {
        let first = migrate(json_of(text)).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let second = migrate(first.value.clone()).unwrap_or_else(|error| panic!("{name} twice: {error:?}"));
        assert_eq!(second.value, first.value, "{name}");
        assert!(second.steps.is_empty(), "{name}: {:?}", second.steps);
        assert!(second.backup.is_none(), "{name}");
    }
}

#[test]
fn aged_profiles_load_without_loss() {
    for (name, text) in [("1y", AGED_1Y), ("3y", AGED_3Y), ("5y", AGED_5Y), ("10y", AGED_10Y)] {
        let input = json_of(text);
        let migrated = migrate(input.clone()).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        match &migrated.backup {
            Some(backup) => {
                assert!(!migrated.steps.is_empty(), "{name}");
                assert_eq!(backup, &input, "{name}");
            }
            None => assert!(migrated.steps.is_empty(), "{name}"),
        }
        profile_of(&migrated.value);
    }

    let one_year = migrate(json_of(AGED_1Y)).unwrap().value;
    // `RENAMED` is empty today, so an id this build does not know stays exactly as written.
    assert!(cadcraft_engine::cmd::registry::resolve_stable("oldline").is_none());
    assert_eq!(strings_at(&one_year, "/layout/toolbars/0/commands"), ["oldline", "circle", "pline"]);
    assert_eq!(one_year.pointer("/layout/shortcuts/oldline"), Some(&json!("L")));
    assert_eq!(one_year.pointer("/layout/aliases/ln"), Some(&json!("oldline")));

    let three_years = json_of(AGED_3Y);
    let migrated = migrate(three_years.clone()).unwrap();
    assert!(contains(&migrated.value, &three_years));
    let profile = profile_of(&migrated.value);
    assert_eq!(profile.layout.toolsets.tab, "Drafting");
    assert!(profile.layout.status_bar.visible);

    let five_years = migrate(json_of(AGED_5Y)).unwrap();
    let panels = profile_of(&five_years.value).layout.panels;
    assert!(panels.iter().any(|panel| panel.id == "legacyPalette"));
    assert_eq!(strings_at(&five_years.value, "/layout/favorites"), ["legacypalette.open"]);

    let ten_years = migrate(json_of(AGED_10Y)).unwrap();
    assert!(!ten_years.steps.is_empty());
    assert_eq!(profile_of(&ten_years.value).layout.extra.get("showRuler"), Some(&json!(true)));
}

#[test]
fn future_minor_keeps_unknown_fields() {
    let input = json_of(FUTURE_MINOR);
    let migrated = migrate(input.clone()).unwrap();
    assert_eq!(migrated.value, input);
    assert!(migrated.steps.is_empty());
    assert!(migrated.backup.is_none());

    let profile = profile_of(&migrated.value);
    assert_eq!(profile.schema_version, "1.3.0");
    assert_eq!(profile.extra.get("future_field"), input.get("future_field"));
    assert_eq!(profile.layout.extra.get("smart_dock"), input.pointer("/layout/smart_dock"));
    let round_trip = serde_json::to_value(&profile).unwrap();
    assert_eq!(round_trip.get("future_field"), input.get("future_field"));
    assert_eq!(round_trip.pointer("/layout/smart_dock"), input.pointer("/layout/smart_dock"));
}

#[test]
fn future_major_is_refused() {
    let mut input = json_of(FUTURE_MINOR);
    input["schema_version"] = json!("2.0.0");
    assert!(matches!(migrate(input.clone()), Err(MigrateError::NewerMajor(version)) if version == "2.0.0"));
    input["schema_version"] = json!("10.4.1");
    assert!(matches!(migrate(input), Err(MigrateError::NewerMajor(version)) if version == "10.4.1"));
    assert_eq!(SAFE_PRESET, "SIMPLE");
    assert!(preset(SAFE_PRESET).is_some());
}

#[test]
fn corrupt_fixtures_error_not_panic() {
    assert!(serde_json::from_str::<Value>(CORRUPT_TRUNCATED).is_err());
    assert!(validate(CORRUPT_TRUNCATED, Mode::Local).is_err());

    assert!(matches!(migrate(json_of(CORRUPT_WRONGTYPES)), Err(MigrateError::Invalid(_))));

    for hostile in [
        json!({}),
        json!({"schema_version": 1}),
        json!({"schema_version": "one"}),
        json!({"schema_version": "1.0"}),
        json!({"schema_version": "1.0.0.0"}),
        json!({"schema_version": "-1.0.0"}),
        json!({"schema_version": "99999999999999999999.0.0"}),
        json!({"schema_version": "0.9.0"}),
        json!({"version": 1}),
        json!({"version": 1, "workspace": []}),
        json!({"version": 2, "workspace": {}}),
    ] {
        let verdict = migrate(hostile.clone());
        assert!(matches!(verdict, Err(MigrateError::Invalid(_))), "{hostile}: {:?}", verdict.map(|migrated| migrated.value));
    }
    for not_object in [json!(null), json!(7), json!("profile"), json!([])] {
        assert!(matches!(migrate(not_object), Err(MigrateError::NotAnObject)));
    }
}

fn add_ui_density(value: &mut Value) -> Result<(), MigrateError> {
    value["ui_density"] = json!("comfortable");
    Ok(())
}

fn add_pin_bar(value: &mut Value) -> Result<(), MigrateError> {
    value["layout"]["pin_bar"] = json!(["line", "circle"]);
    Ok(())
}

fn refuse(_: &mut Value) -> Result<(), MigrateError> {
    Err(MigrateError::Invalid("step refused".into()))
}

const FAKE_STEPS: [Step; 2] = [
    Step { name: "add_ui_density", from: "1.0.0", to: "1.1.0", apply: add_ui_density },
    Step { name: "add_pin_bar", from: "1.1.0", to: "1.2.0", apply: add_pin_bar },
];

#[test]
fn upgrade_two_versions_then_rollback_ux09() {
    let input = json_of(AGED_5Y);
    let before = serde_json::to_string(&input).unwrap();

    let Migrated { value, steps, backup } = migrate_with(input.clone(), &FAKE_STEPS).unwrap();
    assert_eq!(steps, vec!["add_ui_density", "add_pin_bar"]);
    assert_eq!(value["schema_version"], json!("1.2.0"));
    assert_eq!(value["ui_density"], json!("comfortable"));
    let mut stamped = input.clone();
    stamped["schema_version"] = json!("1.2.0");
    assert!(contains(&value, &stamped), "an upgrade must not drop a field");

    let backup = backup.unwrap();
    assert_eq!(backup, input);
    assert_eq!(serde_json::to_string(&backup).unwrap(), before);

    let again = migrate_with(value.clone(), &FAKE_STEPS).unwrap();
    assert!(again.steps.is_empty());
    assert_eq!(again.value, value);

    let refusing = [Step { name: "refuse", from: "1.0.0", to: "1.1.0", apply: refuse }];
    assert!(matches!(migrate_with(input.clone(), &refusing), Err(MigrateError::Invalid(reason)) if reason == "step refused"));
    assert_eq!(serde_json::to_string(&input).unwrap(), before);
}

#[test]
fn real_registry_and_changelog_agree_with_schema_version() {
    assert_eq!(CHANGELOG.last().map(|entry| entry.0), Some(SCHEMA_VERSION));
    for step in STEPS {
        assert!(CHANGELOG.iter().any(|entry| entry.0 == step.to), "{} has no changelog line", step.to);
    }
    assert!(CHANGELOG.iter().all(|entry| !entry.1.is_empty()));
}

#[test]
fn spec_style_profile_with_root_level_layout_keys_migrates_unchanged() {
    let input = json!({
        "schema_version": "1.0.0",
        "profile_id": "uuid",
        "revision": 12,
        "base_id": "AUTOCAD_FAMILIAR",
        "panels": [{"id": "properties", "mode": "dock", "side": "right", "width_ratio": 0.24}],
        "command_line": {"side": "bottom", "height_ratio": 0.15},
        "shortcuts": {"line": "L"}
    });
    let migrated = migrate(input.clone()).unwrap();
    assert_eq!(migrated.value, input);
    assert!(migrated.steps.is_empty());
    assert_eq!(profile_of(&migrated.value).layout.shortcuts.get("line").map(String::as_str), Some("L"));
}
