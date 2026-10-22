//! Workspace profile model: schema round trips, import validation and the embedded presets.

use std::collections::BTreeSet;

use cadcraft_engine::{Session, command_specs};
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::menus::UI_COMMANDS;
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, SAFE_PRESET, preset, preset_json};
use cadcraft_ui_egui::wsprofile::schema::{Dock, OwnerType, PanelMode, ProfileStatus, SCHEMA_VERSION, WorkspaceLayout, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::wsprofile::validate::{Issue, Mode, validate};
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const SPEC_SAMPLE: &str = r#"{"schema_version":"1.0.0","profile_id":"uuid","revision":12,"base_id":"AUTOCAD_FAMILIAR","panels":[{"id":"properties","mode":"dock","side":"right","width_ratio":0.24}],"command_line":{"side":"bottom","height_ratio":0.15},"shortcuts":{"line":"L"}}"#;

fn parse(value: Value) -> WorkspaceProfile {
    serde_json::from_value(value).unwrap()
}

fn has_pointer(issues: &[Issue], pointer: &str) -> bool {
    issues.iter().any(|issue| issue.pointer == pointer)
}

fn known_commands() -> BTreeSet<String> {
    let mut ids: BTreeSet<String> = command_specs().iter().map(|spec| spec.id.to_string()).collect();
    ids.extend(UI_COMMANDS.iter().map(|command| command.0.to_string()));
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
fn import_checks_object_keys_like_values() {
    // serde decodes the escape before the key is checked.
    let issues = validate(r#"{"layout":{"\u0073cript":"x"}}"#, Mode::Import).unwrap_err();
    assert!(has_pointer(&issues, "/layout/script"), "{issues:?}");
    let issues = validate(r#"{"layout":{"toolbars":[{"id":"t","meta":{"exec":"x"}}]}}"#, Mode::Import).unwrap_err();
    assert!(has_pointer(&issues, "/layout/toolbars/0/meta/exec"), "{issues:?}");
    let cases = [
        (r#"{"layout":{"aliases":{"go.lsp":"line"}}}"#, "/layout/aliases/go.lsp"),
        (r#"{"shortcuts":{"C:\\tools\\run":"L"}}"#, "/shortcuts/C:\\tools\\run"),
        (r#"{"layout":{"aliases":{"/etc/x":"line"}}}"#, "/layout/aliases/~1etc~1x"),
        (r#"{"x":{"file:a":1}}"#, "/x/file:a"),
    ];
    for (raw, pointer) in cases {
        let issues = validate(raw, Mode::Import).unwrap_err();
        assert!(has_pointer(&issues, pointer), "{raw}: expected {pointer}, got {issues:?}");
        assert!(validate(raw, Mode::Local).is_ok(), "{raw} in Local mode");
    }
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
fn ribbon_presets_carry_the_eight_standard_tabs() {
    for id in PRESET_IDS.iter().filter(|id| **id != "SIMPLE") {
        let ribbon = preset(id).unwrap().layout.ribbon;
        assert!(ribbon.visible, "{id}");
        let standard: Vec<&str> = ribbon.tabs.iter().filter(|tab| tab.contextual.is_none()).map(|tab| tab.label.as_str()).collect();
        assert_eq!(standard, ["Home", "Insert", "Annotate", "Layers", "View", "Layout", "Tools", "Output"], "{id}");
        let contextual: Vec<&str> = ribbon.tabs.iter().filter_map(|tab| tab.contextual.as_deref()).collect();
        assert_eq!(contextual, ["dimension", "hatch"], "{id}");
        assert!(ribbon.tabs.iter().all(|tab| !tab.groups.is_empty() && tab.groups.iter().all(|group| !group.commands.is_empty())), "{id}");
    }
}

/// The ids of the shipped Standard tool bar, so the preset cannot drift from it.
fn shipped_toolbar_ids() -> Vec<String> {
    cadcraft_ui_egui::toolbars::builtin_toolbar_commands("standard").into_iter().map(str::to_string).collect()
}

#[test]
fn simple_equals_current_ui() {
    let simple = preset("SIMPLE").unwrap();
    let standard = simple.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
    let shipped = shipped_toolbar_ids();
    assert_eq!(shipped.len(), 25, "the shipped Standard tool bar changed: update SIMPLE and presets.md");
    assert_eq!(standard.commands, shipped);
    assert_eq!(standard.dock, Dock::Top);
    assert!(standard.visible);
    assert!(simple.layout.toolsets.visible);
    assert_eq!(simple.layout.toolsets.tab, "Drafting");
    assert!(!simple.layout.ribbon.visible);
    assert!(simple.layout.status_bar.visible);
    assert!(simple.layout.command_line.visible);
    assert_eq!(simple.layout.command_line.dock, Dock::Floating);
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

fn first_run_app(skip_picker: bool) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = skip_picker;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app
}

fn shell(mut app: CadApp) -> Harness<'static, CadApp> {
    app.ui.in_window_menu = false;
    app.integrated_titlebar = true;
    let mut harness = Harness::builder().with_size(vec2(1280.0, 760.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    harness
}

fn stored_active(app: &CadApp) -> String {
    let raw = app.ws.store.as_deref().and_then(|store| store.read_current()).expect("a workspace was stored");
    serde_json::from_str::<Value>(&raw).unwrap()["active"].as_str().unwrap().to_string()
}

#[test]
fn first_run_shows_picker_ux01() {
    let mut harness = shell(first_run_app(false));
    assert_eq!(harness.state().ui.dialog.as_deref(), Some("workspace_picker"));
    assert!(harness.state().ws.store.as_deref().unwrap().read_current().is_none(), "nothing is stored before the user chooses");

    harness.state_mut().run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    harness.run_steps(4);
    let app = harness.state();
    assert_eq!(app.ui.dialog, None, "choosing closes the picker");
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
    assert_eq!(stored_active(app), "AUTOCAD_FAMILIAR", "the choice is stored, so the picker does not come back");
    let layout = &app.ws.effective.layout;
    assert!(layout.ribbon.visible);
    assert!(layout.command_line.visible && app.ui.show_command_line);
    let properties = layout.panels.iter().find(|panel| panel.id == "properties").unwrap();
    assert!(properties.visible && properties.mode == PanelMode::Dock);
    assert!(app.ui.docking.layout.contains(&Panel::Properties));
    let preset_tabs: Vec<String> = preset("AUTOCAD_FAMILIAR").unwrap().layout.ribbon.tabs.iter().map(|tab| tab.id.clone()).collect();
    let tabs: Vec<String> = layout.ribbon.tabs.iter().map(|tab| tab.id.clone()).collect();
    assert_eq!(tabs, preset_tabs);
    assert_eq!(tabs[..7], ["home", "insert", "annotate", "layers", "view", "layout", "tools"]);
    let orders: Vec<u32> = layout.ribbon.tabs.iter().map(|tab| tab.order).collect();
    assert!(orders.windows(2).all(|pair| pair[0] < pair[1]), "tab order follows the preset: {orders:?}");
}

#[test]
fn picker_lists_every_preset_and_a_row_selects_it() {
    let mut harness = shell(first_run_app(false));
    for name in [
        "Simple",
        "AutoCAD Familiar",
        "Planning and GIS",
        "Architecture",
        "Structure",
        "MEP and Fire",
        "Infrastructure",
        "Quantity Survey and QA",
        "Sheets and Publishing",
    ] {
        harness.get_by_label(name);
    }
    harness.get_by_label("Familiar to AutoCAD users");
    harness.get_by_label("Architecture").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ui.dialog, None);
    assert_eq!(harness.state().ws.active, "ARCHITECTURE");
    assert_eq!(stored_active(harness.state()), "ARCHITECTURE");
}

#[test]
fn picker_escape_selects_simple() {
    let mut harness = shell(first_run_app(false));
    assert_eq!(harness.state().ui.dialog.as_deref(), Some("workspace_picker"));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);
    let app = harness.state();
    assert_eq!(app.ui.dialog, None);
    assert_eq!(app.ws.active, "SIMPLE");
    assert_eq!(stored_active(app), "SIMPLE");

    // Same store, next run: something is stored now, so no picker.
    let store = harness.state_mut().ws.store.take().unwrap();
    let mut again = CadApp::new(Session::new(), Services::default());
    again.attach_profile_store(store);
    let again = shell(again);
    assert_eq!(again.state().ui.dialog, None);
}

#[test]
fn picker_closed_with_its_button_selects_simple() {
    let mut harness = shell(first_run_app(false));
    harness.get_by_label("Close window").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ui.dialog, None);
    assert_eq!(stored_active(harness.state()), "SIMPLE");
}

#[test]
fn host_or_sample_skips_picker() {
    let harness = shell(first_run_app(true));
    assert_eq!(harness.state().ui.dialog, None);
    assert_eq!(harness.state().ws.active, "SIMPLE");

    // Not the first run: nothing to offer even when the flag is off.
    let mut stored = first_run_app(true);
    stored.run("ui.workspace.select", json!({"id": "ARCHITECTURE"})).unwrap();
    let store = stored.ws.store.take().unwrap();
    let mut returning = CadApp::new(Session::new(), Services::default());
    returning.attach_profile_store(store);
    assert_eq!(shell(returning).state().ui.dialog, None);
}

#[test]
fn select_without_an_id_opens_the_picker_only_from_a_menu() {
    let mut app = first_run_app(true);
    assert!(app.run("ui.workspace.select", json!({})).is_err(), "a programmatic call without an id is an error, not a dialog");
    assert_eq!(app.ui.dialog, None);
    app.start("ui.workspace.select");
    assert_eq!(app.ui.dialog.as_deref(), Some("workspace_picker"));
}

fn pointer(harness: &mut Harness<'_, CadApp>, pos: egui::Pos2, pressed: Option<bool>) {
    match pressed {
        None => harness.event(Event::PointerMoved(pos)),
        Some(pressed) => harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }),
    }
    harness.run_steps(2);
}

#[test]
fn switch_during_drag_cancels_drag_keeps_command() {
    let mut harness = shell(first_run_app(true));
    let source = harness.get_by_role_and_label(egui::accesskit::Role::Tab, "Layers").rect().center();
    let target = harness.get_by_role_and_label(egui::accesskit::Role::Tab, "Tool Sets").rect().center();
    pointer(&mut harness, source, None);
    pointer(&mut harness, source, Some(true));
    for step in 1..=6 {
        pointer(&mut harness, source.lerp(target, step as f32 / 6.0), None);
    }
    assert!(harness.ctx.dragged_id().is_some(), "the tab is being dragged");

    harness.state_mut().start("line");
    harness.run_steps(2);
    assert_eq!(harness.state().session.running.as_ref().map(|command| command.id.as_str()), Some("line"));

    harness.state_mut().run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    harness.run_steps(3);
    assert!(harness.ctx.dragged_id().is_none(), "the switch cancelled the drag");
    let after_switch = serde_json::to_value(&harness.state().ui.docking).unwrap();

    // Letting go where the tab was heading changes nothing: the drag is gone.
    pointer(&mut harness, target, Some(false));
    harness.run_steps(3);
    assert_eq!(serde_json::to_value(&harness.state().ui.docking).unwrap(), after_switch);
    let app = harness.state();
    assert_eq!(app.session.running.as_ref().map(|command| command.id.as_str()), Some("line"), "the command is still waiting for its point");
    assert!(app.session.prompting());
    assert!(!app.session.state().unwrap().is_dirty());
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
}

fn run_ok(app: &mut CadApp, id: &str, params: Value) -> Value {
    app.run(id, params.clone()).unwrap_or_else(|error| panic!("{id} {params}: {error}"))
}

#[test]
fn switching_is_a_layout_change_that_layout_undo_takes_back() {
    let mut app = first_run_app(true);
    let before = app.ui.docking.layout.clone();
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    assert_ne!(app.ui.docking.layout, before);
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
    assert_eq!(app.ws.user.base_id.as_deref(), Some("AUTOCAD_FAMILIAR"));

    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ui.docking.layout, before);
    assert_eq!(app.ws.active, "SIMPLE", "undo takes the workspace back, not only its layout");
    assert_eq!(app.ws.user.base_id.as_deref(), Some("SIMPLE"));
    assert_eq!(app.ws.user.profile_id, "user.SIMPLE");
    let status = app.workspace_status();
    assert_eq!((status["active"].as_str(), status["base_id"].as_str()), (Some("SIMPLE"), Some("SIMPLE")));

    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.active, "AUTOCAD_FAMILIAR");
    assert_eq!(app.ws.user.base_id.as_deref(), Some("AUTOCAD_FAMILIAR"));
    assert_ne!(app.ui.docking.layout, before);
    assert!(!app.session.state().unwrap().is_dirty());
}

#[test]
fn undoing_a_switch_never_rewrites_a_saved_workspace() {
    let mut app = first_run_app(true);
    run_ok(&mut app, "ui.cmdline.lines", json!({"lines": 9}));
    let mine = run_ok(&mut app, "ui.workspace.saveas", json!({"name": "Mine"}))["profile_id"].as_str().unwrap().to_string();
    run_ok(&mut app, "ui.workspace.select", json!({"id": "ARCHITECTURE"}));
    let saved_before = app.ws.profiles[0].clone();
    assert_eq!(saved_before.layout.command_line.lines, 9);

    run_ok(&mut app, "ui.workspace.select", json!({"id": mine}));
    assert_eq!(app.ws.active, mine);
    let stored_revision = app.ws.profiles[0].revision;
    run_ok(&mut app, "ui.layout.undo", json!({}));
    assert_eq!(app.ws.active, "ARCHITECTURE");
    assert_eq!(app.ws.user.base_id.as_deref(), Some("ARCHITECTURE"));
    app.commit_workspace_now().unwrap();
    // Selecting it stores it once (revision), but its content and identity must stay as saved.
    let saved_after = &app.ws.profiles[0];
    assert_eq!(
        (&saved_after.profile_id, &saved_after.name, &saved_after.base_id, &saved_after.layout),
        (&saved_before.profile_id, &saved_before.name, &saved_before.base_id, &saved_before.layout),
        "the saved workspace is untouched by undo and the commit after it"
    );
    let raw = app.ws.store.as_deref().unwrap().read_current().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["active"], json!("ARCHITECTURE"));
    assert_eq!(app.ws.profiles[0].revision, stored_revision, "undo and the commit after it did not rewrite the saved workspace");

    run_ok(&mut app, "ui.layout.redo", json!({}));
    assert_eq!(app.ws.active, mine);
    assert_eq!(app.ws.user.profile_id, mine);
    assert_eq!(app.ws.user.layout.command_line.lines, 9);
}

#[test]
fn menu_check_and_status_follow_the_workspace_after_undo() {
    let mut harness = shell(first_run_app(true));
    harness.state_mut().run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    harness.state_mut().run("ui.layout.undo", json!({})).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Workspace Switching").click();
    harness.run_steps(3);
    assert_eq!(harness.get_by_label("Simple").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_ne!(harness.get_by_label("AutoCAD Familiar").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
}

#[test]
fn a_second_dialog_keeps_the_picker_for_a_later_frame() {
    let mut app = first_run_app(false);
    app.ui.dialog = Some("about".into());
    let mut harness = shell(app);
    assert_eq!(harness.state().ui.dialog.as_deref(), Some("about"), "an open dialog is not replaced");
    harness.state_mut().ui.dialog = None;
    harness.run_steps(3);
    assert_eq!(harness.state().ui.dialog.as_deref(), Some("workspace_picker"));
}

#[test]
fn choosing_the_active_workspace_in_the_menu_changes_nothing() {
    let mut harness = shell(first_run_app(true));
    harness.state_mut().run("ui.workspace.select", json!({"id": "ARCHITECTURE"})).unwrap();
    harness.state_mut().run("ui.cmdline.lines", json!({"lines": 7})).unwrap();
    harness.run_steps(2);
    let undo_steps = harness.state().ws.history.undo_len();
    let layout = harness.state().ws.user.layout.clone();
    harness.get_by_label("Workspace Switching").click();
    harness.run_steps(3);
    harness.get_by_label("Architecture").click();
    harness.run_steps(3);
    let app = harness.state();
    assert_eq!(app.ws.user.layout, layout, "the user's changes are kept");
    assert_eq!(app.ui.history_lines, 7);
    assert_eq!(app.ws.history.undo_len(), undo_steps);
    assert_eq!(app.ws.active, "ARCHITECTURE");
}

#[test]
fn status_bar_shows_save_state() {
    let mut harness = shell(first_run_app(true));
    harness.state_mut().run("ui.cmdline.lines", json!({"lines": 7})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Saving…");
    harness.state_mut().commit_workspace_now().unwrap();
    harness.run_steps(3);
    harness.get_by_label("Saved locally");
    assert!(harness.query_by_label("Saving…").is_none());

    let mut failing = first_run_app(true);
    failing.ws.store = Some(Box::new(MemoryStore { fail_writes: true, ..Default::default() }));
    let mut harness = shell(failing);
    harness.state_mut().run("ui.cmdline.lines", json!({"lines": 7})).unwrap();
    let _ = harness.state_mut().commit_workspace_now();
    harness.run_steps(3);
    harness.get_by_label("Not saved");
}

#[test]
fn status_bar_without_storage_shows_no_save_state() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    let harness = shell(app);
    for label in ["Saved locally", "Saving…", "Not saved"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn workspace_menu_switches_saves_and_resets() {
    let mut harness = shell(first_run_app(true));
    harness.get_by_label("Workspace Switching").click();
    harness.run_steps(3);
    harness.get_by_label("Architecture").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ws.active, "ARCHITECTURE");

    harness.state_mut().run("ui.toggle.statusbar", json!({"on": false})).unwrap();
    harness.run_steps(2);
    assert!(!harness.state().ui.show_status_bar);
    // The status bar is hidden now; bring it back through the menu's Reset.
    harness.state_mut().run("ui.toggle.statusbar", json!({"on": true})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Workspace Switching").click();
    harness.run_steps(3);
    assert!(!harness.get_by_label("Customize...").accesskit_node().is_disabled(), "Customize is available");
    harness.get_by_label("Reset").click();
    harness.run_steps(3);
    assert!(harness.state().ui.show_status_bar);

    harness.get_by_label("Workspace Switching").click();
    harness.run_steps(3);
    harness.get_by_label("Save As...").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.dialog.as_deref(), Some("workspace_saveas"));
    harness.query_all_by_role(egui::accesskit::Role::TextInput).find(|node| node.accesskit_node().is_focused()).unwrap().type_text("My Layout");
    harness.run_steps(2);
    harness.get_by_label("Save").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ui.dialog, None);
    assert!(harness.state().ws.profiles.iter().any(|profile| profile.name == "My Layout"));
    assert_eq!(harness.state().ws.active, harness.state().ws.profiles[0].profile_id);
}
