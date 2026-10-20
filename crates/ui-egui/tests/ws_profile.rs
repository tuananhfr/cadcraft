//! Workspace profile model: schema round trips, import validation and the embedded presets.

use std::collections::BTreeSet;

use cadcraft_engine::command_specs;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::menus::UI_COMMANDS;
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, SAFE_PRESET, preset, preset_json};
use cadcraft_ui_egui::wsprofile::schema::{Dock, OwnerType, PanelMode, ProfileStatus, SCHEMA_VERSION, WorkspaceLayout, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::validate::{Issue, Mode, validate};
use serde_json::{Value, json};

const SPEC_SAMPLE: &str = r#"{"schema_version":"1.0.0","profile_id":"uuid","revision":12,"base_id":"AUTOCAD_FAMILIAR","panels":[{"id":"properties","mode":"dock","side":"right","width_ratio":0.24}],"command_line":{"side":"bottom","height_ratio":0.15},"shortcuts":{"line":"L"}}"#;

/// Dispatched by `run_ui_command` and used by the tool bar, but absent from `UI_COMMANDS`.
const UNLISTED_UI_COMMANDS: &[&str] = &["ui.toggle.palettes"];

fn parse(value: Value) -> WorkspaceProfile {
    serde_json::from_value(value).unwrap()
}

fn has_pointer(issues: &[Issue], pointer: &str) -> bool {
    issues.iter().any(|issue| issue.pointer == pointer)
}

fn known_commands() -> BTreeSet<String> {
    let mut ids: BTreeSet<String> = command_specs().iter().map(|spec| spec.id.to_string()).collect();
    ids.extend(UI_COMMANDS.iter().map(|command| command.0.to_string()));
    ids.extend(UNLISTED_UI_COMMANDS.iter().map(|id| id.to_string()));
    ids
}

fn profile_commands(profile: &WorkspaceProfile) -> Vec<&String> {
    let layout = &profile.layout;
    let ribbon = layout.ribbon.tabs.iter().flat_map(|tab| tab.groups.iter()).flat_map(|group| group.commands.iter());
    let toolbars = layout.toolbars.iter().flat_map(|toolbar| toolbar.commands.iter());
    ribbon.chain(toolbars).chain(layout.quick_access.iter()).chain(layout.favorites.iter()).chain(layout.toolsets.hidden_tools.iter()).collect()
}

#[test]
fn spec_sample_imports() {
    let value = validate(SPEC_SAMPLE, Mode::Import).unwrap();
    let profile = parse(value);
    assert_eq!(profile.schema_version, SCHEMA_VERSION);
    assert_eq!(profile.base_id.as_deref(), Some("AUTOCAD_FAMILIAR"));
    assert_eq!(profile.revision, 12);
    let properties = profile.layout.panels.iter().find(|panel| panel.id == "properties").unwrap();
    assert_eq!(properties.mode, PanelMode::Dock);
    assert_eq!(properties.side, Some(Dock::Right));
    assert_eq!(properties.width_ratio, 0.24);
    assert_eq!(profile.layout.command_line.dock, Dock::Bottom);
    assert_eq!(profile.layout.command_line.height_ratio, 0.15);
    assert_eq!(profile.layout.shortcuts.get("line").map(String::as_str), Some("L"));
    assert!(!profile.extra.contains_key("panels"), "hoisted keys must not stay at the root");
    assert!(!profile.extra.contains_key("command_line"));
    assert!(!profile.extra.contains_key("shortcuts"));
}

#[test]
fn layout_keys_win_over_root_keys() {
    let profile = parse(json!({
        "shortcuts": {"line": "L"},
        "layout": {"shortcuts": {"circle": "C"}}
    }));
    assert_eq!(profile.layout.shortcuts.len(), 1);
    assert_eq!(profile.layout.shortcuts.get("circle").map(String::as_str), Some("C"));
    assert_eq!(profile.extra.get("shortcuts"), Some(&json!({"line": "L"})));
}

#[test]
fn unknown_fields_round_trip() {
    let raw = json!({
        "profile_id": "p1",
        "future_field": {"a": 1},
        "layout": {"future_layout_field": [1, 2, {"b": null}]}
    });
    let profile = parse(raw);
    assert_eq!(profile.extra.get("future_field"), Some(&json!({"a": 1})));
    let back = serde_json::to_value(&profile).unwrap();
    assert_eq!(back["future_field"], json!({"a": 1}));
    assert_eq!(back["layout"]["future_layout_field"], json!([1, 2, {"b": null}]));
    let again = parse(back.clone());
    assert_eq!(serde_json::to_value(&again).unwrap(), back);
}

#[test]
fn unknown_command_ids_are_kept_verbatim() {
    let profile = parse(json!({"layout": {"quick_access": ["draw.line", "line"], "toolbars": [{"id": "t", "commands": ["draw.line"]}]}}));
    assert_eq!(profile.layout.quick_access, ["draw.line", "line"]);
    let back = serde_json::to_value(&profile).unwrap();
    assert_eq!(back["layout"]["toolbars"][0]["commands"], json!(["draw.line"]));
}

#[test]
fn ratios_and_scale_are_clamped() {
    let profile = parse(json!({
        "ui_scale": 9.0,
        "layout": {
            "panels": [{"id": "layers", "x_ratio": -3.0, "width_ratio": 7.5}],
            "command_line": {"height_ratio": 2.0, "x_ratio": -0.5},
            "toolbars": [{"id": "t", "x_ratio": 4.0, "y_ratio": -1.0}]
        }
    }));
    assert_eq!(profile.ui_scale, 2.0);
    assert_eq!(profile.layout.panels[0].x_ratio, 0.0);
    assert_eq!(profile.layout.panels[0].width_ratio, 1.0);
    assert_eq!(profile.layout.command_line.height_ratio, 1.0);
    assert_eq!(profile.layout.command_line.x_ratio, 0.0);
    assert_eq!(profile.layout.toolbars[0].x_ratio, 1.0);
    assert_eq!(profile.layout.toolbars[0].y_ratio, 0.0);
    assert_eq!(parse(json!({"ui_scale": 0.1})).ui_scale, 0.75);
}

#[test]
fn enums_use_documented_spellings() {
    assert_eq!(serde_json::to_value(OwnerType::Discipline).unwrap(), json!("discipline"));
    assert_eq!(serde_json::to_value(Dock::Floating).unwrap(), json!("floating"));
    assert_eq!(serde_json::to_value(PanelMode::AutoHide).unwrap(), json!("auto_hide"));
}

#[test]
fn import_rejects_executable_and_secrets() {
    let cases = [
        (r#"{"layout":{"startup":"x.lsp"}}"#, "/layout/startup", true),
        (r#"{"x":"C:\\a\\b.dll"}"#, "/x", true),
        (r#"{"token":"abc"}"#, "/token", false),
        (r#"{"x":"file:///etc"}"#, "/x", true),
        (r#"{"x":"\\\\srv\\s"}"#, "/x", true),
    ];
    for (raw, pointer, local_ok) in cases {
        let issues = validate(raw, Mode::Import).unwrap_err();
        assert!(has_pointer(&issues, pointer), "{raw}: expected {pointer}, got {issues:?}");
        assert_eq!(validate(raw, Mode::Local).is_ok(), local_ok, "{raw} in Local mode");
    }
}

#[test]
fn import_payload_rules_cover_the_whole_list() {
    for extension in ["lsp", "fas", "vlx", "dvb", "arx", "dll", "exe", "js", "ps1", "bat", "cmd"] {
        let raw = json!({"a": {"b": [format!("tool.{}", extension.to_uppercase())]}}).to_string();
        let issues = validate(&raw, Mode::Import).unwrap_err();
        assert!(has_pointer(&issues, "/a/b/0"), "{extension}: {issues:?}");
    }
    for key in ["script", "lisp", "exec", "eval", "code", "onload", "startup", "StartUp"] {
        let raw = json!({"layout": {key: 1}}).to_string();
        let issues = validate(&raw, Mode::Import).unwrap_err();
        assert!(has_pointer(&issues, &format!("/layout/{key}")), "{key}: {issues:?}");
    }
    for key in ["token", "access_token", "Password", "client_secret", "cookie", "Authorization", "apiKey", "api_key"] {
        let raw = json!({ key: "x" }).to_string();
        for mode in [Mode::Local, Mode::Import] {
            let issues = validate(&raw, mode).unwrap_err();
            assert!(has_pointer(&issues, &format!("/{key}")), "{key}: {issues:?}");
        }
    }
    for value in ["/etc/passwd", "\\\\srv\\share", "\\windows", "D:/x", "d:\\x", "FILE:x"] {
        let raw = json!({"p": value}).to_string();
        assert!(has_pointer(&validate(&raw, Mode::Import).unwrap_err(), "/p"), "{value}");
    }
    // `.json`, `.jsx`-like suffixes and ordinary text are not executable payloads.
    let fine = json!({"a": "settings.json", "b": "Ctrl+Shift+L", "c": "draw.line", "d": "E:", "e": "a/b"}).to_string();
    assert!(validate(&fine, Mode::Import).is_ok());
}

#[test]
fn pointers_escape_special_characters() {
    let issues = validate(r#"{"a/b":{"c~d":"x.exe"}}"#, Mode::Import).unwrap_err();
    assert!(has_pointer(&issues, "/a~1b/c~0d"), "{issues:?}");
}

#[test]
fn limits() {
    let big = format!("{{\"a\":\"{}\"}}", "x".repeat(300 * 1024));
    let issues = validate(&big, Mode::Local).unwrap_err();
    assert!(issues[0].reason.contains("too large"), "{issues:?}");

    let nested = |depth: usize| format!("{}{{}}{}", "{\"a\":".repeat(depth - 1), "}".repeat(depth - 1));
    let issues = validate(&nested(40), Mode::Local).unwrap_err();
    assert!(issues.iter().any(|issue| issue.reason.contains("deep")), "{issues:?}");
    assert!(validate(&nested(32), Mode::Local).is_ok());
    assert!(validate(&nested(33), Mode::Local).is_err());
    let abyss = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(validate(&abyss, Mode::Local).is_err());

    let array = |n: usize| format!("{{\"a\":[{}]}}", vec!["0"; n].join(","));
    let issues = validate(&array(600), Mode::Local).unwrap_err();
    assert!(has_pointer(&issues, "/a") && issues[0].reason.contains("array"), "{issues:?}");
    assert!(validate(&array(500), Mode::Local).is_ok());
    assert!(validate(&array(501), Mode::Local).is_err());

    let string = |n: usize| format!("{{\"a\":\"{}\"}}", "x".repeat(n));
    let issues = validate(&string(2000), Mode::Local).unwrap_err();
    assert!(has_pointer(&issues, "/a") && issues[0].reason.contains("string"), "{issues:?}");
    assert!(validate(&string(1024), Mode::Local).is_ok());
    assert!(validate(&string(1025), Mode::Local).is_err());

    let exactly_max = format!("{{}}{}", " ".repeat(256 * 1024 - 2));
    assert!(validate(&exactly_max, Mode::Local).is_ok());
    assert!(validate(&format!("{exactly_max} "), Mode::Local).is_err());
}

#[test]
fn invalid_json_and_non_objects_are_reported() {
    for raw in ["", "{", "nope", "[1]", "42", "null", "{\"a\":}", "\u{feff}{}"] {
        let issues = validate(raw, Mode::Local).unwrap_err();
        assert_eq!(issues[0].pointer, "", "{raw}");
    }
}

#[test]
fn every_preset_validates_and_uses_real_commands() {
    let commands = known_commands();
    let panels: BTreeSet<&str> = Panel::ALL.iter().chain(Panel::ON_DEMAND).map(|panel| panel.id()).collect();
    for id in PRESET_IDS {
        let raw = preset_json(id).unwrap();
        for mode in [Mode::Local, Mode::Import] {
            validate(raw, mode).unwrap_or_else(|issues| panic!("{id} {mode:?}: {issues:?}"));
        }
        let profile = preset(id).unwrap_or_else(|| panic!("{id} does not parse"));
        assert_eq!(profile.schema_version, SCHEMA_VERSION, "{id}");
        assert_eq!(profile.owner_type, OwnerType::Platform, "{id}");
        assert_eq!(&profile.profile_id, &format!("preset.{id}"), "{id}");
        for command in profile_commands(&profile) {
            assert!(commands.contains(command), "{id}: unknown command {command}");
        }
        for panel in &profile.layout.panels {
            assert!(panels.contains(panel.id.as_str()), "{id}: unknown panel {}", panel.id);
        }
        let back = serde_json::to_value(&profile).unwrap();
        assert_eq!(serde_json::to_value(parse(back.clone())).unwrap(), back, "{id} round trip");
    }
    assert!(preset("LPC_COMPANY").is_none());
    assert!(preset("simple").is_none());
    assert!(PRESET_IDS.contains(&SAFE_PRESET));
}

#[test]
fn presets_keep_ratios_in_range_and_ids_unique() {
    for id in PRESET_IDS {
        let layout = preset(id).unwrap().layout;
        let in_unit = |value: f32| (0.0..=1.0).contains(&value);
        for panel in &layout.panels {
            assert!([panel.x_ratio, panel.y_ratio, panel.width_ratio, panel.height_ratio].into_iter().all(in_unit), "{id} {}", panel.id);
        }
        for toolbar in &layout.toolbars {
            assert!(in_unit(toolbar.x_ratio) && in_unit(toolbar.y_ratio), "{id} {}", toolbar.id);
        }
        let unique = |ids: Vec<&str>| ids.len() == ids.iter().collect::<BTreeSet<_>>().len();
        assert!(unique(layout.panels.iter().map(|panel| panel.id.as_str()).collect()), "{id} panels");
        assert!(unique(layout.toolbars.iter().map(|toolbar| toolbar.id.as_str()).collect()), "{id} toolbars");
        assert!(unique(layout.ribbon.tabs.iter().map(|tab| tab.id.as_str()).collect()), "{id} tabs");
    }
}

#[test]
fn discipline_presets_are_drafts() {
    for id in PRESET_IDS {
        let status = preset(id).unwrap().status;
        let expected = if matches!(*id, "SIMPLE" | "AUTOCAD_FAMILIAR") { ProfileStatus::Published } else { ProfileStatus::Draft };
        assert_eq!(status, expected, "{id}");
    }
}

#[test]
fn command_line_cannot_dock_to_a_side() {
    let profile = parse(json!({"layout": {"command_line": {"dock": "left"}}, "panels": [{"id": "p", "side": "left"}]}));
    assert_eq!(profile.layout.command_line.dock, Dock::Bottom);
    assert_eq!(profile.layout.panels[0].side, Some(Dock::Left));
}

#[test]
fn ribbon_presets_carry_the_seven_standard_tabs() {
    for id in PRESET_IDS.iter().filter(|id| **id != "SIMPLE") {
        let ribbon = preset(id).unwrap().layout.ribbon;
        assert!(ribbon.visible, "{id}");
        let standard: Vec<&str> = ribbon.tabs.iter().filter(|tab| tab.contextual.is_none()).map(|tab| tab.label.as_str()).collect();
        assert_eq!(standard, ["Home", "Insert", "Annotate", "Layers", "View", "Layout", "Tools"], "{id}");
        let contextual: Vec<&str> = ribbon.tabs.iter().filter_map(|tab| tab.contextual.as_deref()).collect();
        assert_eq!(contextual, ["dimension", "hatch"], "{id}");
        assert!(ribbon.tabs.iter().all(|tab| !tab.groups.is_empty() && tab.groups.iter().all(|group| !group.commands.is_empty())), "{id}");
    }
}

/// The ids of `chrome::TOOLBAR`, read from the source so the preset cannot drift from the shipped tool bar.
fn shipped_toolbar_ids() -> Vec<String> {
    let source = include_str!("../src/chrome.rs");
    let start = source.find("const TOOLBAR").unwrap();
    let body = &source[start..start + source[start..].find("\n];").unwrap()];
    body.split("(Icon::").skip(1).map(|entry| entry.split('"').nth(1).unwrap().to_string()).collect()
}

#[test]
fn simple_equals_current_ui() {
    let simple = preset("SIMPLE").unwrap();
    let standard = simple.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
    let shipped = shipped_toolbar_ids();
    assert_eq!(shipped.len(), 25, "chrome::TOOLBAR changed: update SIMPLE and presets.md");
    assert_eq!(standard.commands, shipped);
    assert_eq!(standard.dock, Dock::Top);
    assert!(standard.visible);
    assert!(simple.layout.toolsets.visible);
    assert_eq!(simple.layout.toolsets.tab, "Drafting");
    assert!(!simple.layout.ribbon.visible);
    assert!(simple.layout.status_bar.visible);
    assert!(simple.layout.command_line.visible);
    assert_eq!(simple.layout.command_line.dock, Dock::Bottom);
    assert_eq!(simple.layout.command_line.lines, 3);
    let panel_ids: Vec<&str> = simple.layout.panels.iter().map(|panel| panel.id.as_str()).collect();
    assert_eq!(panel_ids, ["toolSets", "layers", "properties", "externalReferences"]);
    assert_eq!(simple.ui_scale, 1.0);
}

#[test]
fn defaults_are_the_simple_preset() {
    let simple = preset(SAFE_PRESET).unwrap();
    assert_eq!(WorkspaceLayout::default(), simple.layout);
    assert_eq!(WorkspaceProfile::default(), simple);
    assert!(serde_json::from_value::<WorkspaceProfile>(json!({})).is_ok_and(|profile| profile.layout == simple.layout));
}

/// Tiny deterministic generator: the tests must not depend on a rand crate or on the clock.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn hostile_profiles_never_panic() {
    let mut rng = Lcg(0x5eed);
    for id in PRESET_IDS {
        let original = preset_json(id).unwrap().as_bytes().to_vec();
        for round in 0..200 {
            let mut bytes = original.clone();
            match round % 3 {
                0 => {
                    for _ in 0..1 + rng.next() % 8 {
                        let at = (rng.next() as usize) % bytes.len();
                        bytes[at] ^= 1 << (rng.next() % 8);
                    }
                }
                1 => bytes.truncate((rng.next() as usize) % bytes.len()),
                _ => {
                    let at = (rng.next() as usize) % bytes.len();
                    bytes.splice(at..at, b"[{\"\\".iter().copied().cycle().take(1 + (rng.next() % 40) as usize));
                }
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            for mode in [Mode::Local, Mode::Import] {
                if let Ok(value) = validate(&text, mode)
                    && let Ok(profile) = serde_json::from_value::<WorkspaceProfile>(value)
                {
                    let _ = serde_json::to_string(&profile).unwrap();
                }
            }
        }
    }
}

#[test]
fn hostile_shapes_are_errors_not_panics() {
    for raw in [
        json!({"layout": null}),
        json!({"layout": []}),
        json!({"layout": {"toolbars": [null]}}),
        json!({"layout": {"panels": {"id": "x"}}}),
        json!({"layout": {"panels": [{"id": "x", "mode": "bogus"}]}}),
        json!({"revision": -1}),
        json!({"revision": 1.5}),
        json!({"ui_scale": "big"}),
        json!({"ui_scale": 1e300}),
        json!({"panels": "x", "command_line": 3, "shortcuts": []}),
        json!({"layout": {"command_line": {"dock": "left"}}}),
    ] {
        let _ = serde_json::from_value::<WorkspaceProfile>(raw);
    }
}

fn schema_properties<'a>(schema: &'a Value, node: &'a Value) -> Option<&'a serde_json::Map<String, Value>> {
    let node = match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => schema.pointer(reference.trim_start_matches('#'))?,
        None => node,
    };
    node.get("properties").and_then(Value::as_object)
}

fn assert_documented(schema: &Value, node: &Value, value: &Value, path: &str) {
    let node = match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => schema.pointer(reference.trim_start_matches('#')).unwrap(),
        None => node,
    };
    match value {
        Value::Object(map) => {
            // Maps keyed by user data (shortcuts, aliases, status bar items) carry no field names.
            let Some(properties) = schema_properties(schema, node) else { return };
            for (key, child) in map {
                let child_node = properties.get(key).unwrap_or_else(|| panic!("schema lacks {path}/{key}"));
                assert_documented(schema, child_node, child, &format!("{path}/{key}"));
            }
        }
        Value::Array(items) => {
            if let Some(item_node) = node.get("items") {
                for item in items {
                    assert_documented(schema, item_node, item, &format!("{path}[]"));
                }
            }
        }
        _ => {}
    }
}

#[test]
fn schema_doc_lists_every_field() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/workspace/workspace-profile.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert_eq!(schema["properties"]["schema_version"]["const"], SCHEMA_VERSION);
    let default = serde_json::to_value(WorkspaceProfile::default()).unwrap();
    assert_documented(&schema, &schema, &default, "");
    for id in PRESET_IDS {
        let value = serde_json::to_value(preset(id).unwrap()).unwrap();
        assert_documented(&schema, &schema, &value, &format!("{id}:"));
    }
}
