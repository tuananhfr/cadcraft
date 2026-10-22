use cadcraft_engine::sheetplan::{AttributeTags, FrameRegistry, GeometryRule};
use serde_json::{Value, json};

fn template() -> Value {
    json!({"id": "office-a1", "revision": "1", "block_names": ["KHUNG*"], "geometry": [{"kind": "largest_closed_rect"}]})
}

fn registry(templates: Vec<Value>) -> Value {
    json!({"schema_version": "1.0", "templates": templates})
}

fn import(value: Value) -> FrameRegistry {
    FrameRegistry::from_json(&value.to_string()).unwrap()
}

#[test]
fn builtin_has_only_generic_without_lpc_assumptions() {
    let builtin = FrameRegistry::builtin();
    assert_eq!(builtin.schema_version, "2");
    assert_eq!(builtin.templates.len(), 1);
    let generic = &builtin.templates[0];
    assert_eq!(generic.id, "generic");
    assert_eq!(generic.revision, "1");
    assert_eq!(generic.block_names, ["*"]);
    assert_eq!(generic.geometry, [GeometryRule::LargestClosedRect, GeometryRule::EnclosingRect]);
    assert_eq!(generic.attributes.sheet_id, ["SHEET_ID", "SHEET_NO", "SHEETNO", "DWG_NO", "DRAWING_NO"]);
    assert_eq!(generic.attributes.revision, ["REV", "REVISION"]);
    assert_eq!(generic.attributes.title, ["TITLE"]);
    assert!(generic.media_mm.is_empty());
    assert_eq!(generic.superseded_by, None);
    let text = serde_json::to_string(&builtin).unwrap();
    assert!(!text.to_uppercase().contains("LPC"));
    assert_eq!(FrameRegistry::from_json(&text).unwrap(), builtin);
    for name in ["anything", "", "*U123"] {
        assert_eq!(builtin.find(name), Some(generic));
    }
    let empty = import(registry(vec![]));
    assert_eq!(empty.find("other"), None);
    assert_eq!(builtin.merged_with(empty), builtin);
}

#[test]
fn import_adds_and_find_prefers_specific() {
    let builtin = FrameRegistry::builtin();
    let mut specific = template();
    specific["geometry"] = json!([
        {"kind": "layer_rect", "layer": "FRAME"},
        {"kind": "local_rect", "min": [-10, -20], "max": [410, 277]}
    ]);
    specific["attributes"] = json!({"sheet_id": ["NUMBER"], "revision": ["ISSUE"], "title": ["NAME"]});
    specific["media_mm"] = json!([[297, 420]]);
    let mut exact = template();
    exact["id"] = json!("exact");
    exact["block_names"] = json!(["Exact-Name", "BẢN VẼ", "MID*LITERAL", "KHUNG-A1"]);
    let imported = import(registry(vec![specific, exact]));
    let office = &imported.templates[0];
    assert_eq!(office.attributes.sheet_id, ["NUMBER"]);
    assert_eq!(office.attributes.revision, ["ISSUE"]);
    assert_eq!(office.attributes.title, ["NAME"]);
    assert_eq!(office.media_mm, [[297.0, 420.0]]);
    assert_eq!(office.superseded_by, None);
    assert_eq!(office.geometry[0], GeometryRule::LayerRect { layer: "FRAME".into() });
    assert_eq!(office.geometry[1], GeometryRule::LocalRect { min: [-10.0, -20.0], max: [410.0, 277.0] });
    assert_eq!(imported.templates[1].attributes, AttributeTags::default());
    assert!(imported.templates[1].media_mm.is_empty());
    assert_eq!(imported.templates[1].superseded_by, None);
    assert_eq!(imported.find("other"), None);
    let merged = builtin.merged_with(imported.clone());
    assert_eq!(merged.templates.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["office-a1", "exact", "generic"]);
    for name in ["khung-a1", "KHUNG-A2", "khung"] {
        assert_eq!(merged.find(name).unwrap().id, "office-a1", "{name}");
    }
    for name in ["exact-name", "bản vẽ", "mid*literal"] {
        assert_eq!(merged.find(name).unwrap().id, "exact", "{name}");
    }
    for name in ["other", "xkhung-a1", "exact-name-extra", "mid-any-literal"] {
        assert_eq!(merged.find(name).unwrap().id, "generic", "{name}");
    }
    assert_eq!(import(serde_json::to_value(&merged).unwrap()), merged);
    let mut generic_first = merged.clone();
    generic_first.templates.rotate_right(1);
    assert_eq!(generic_first.find("khung-a1").unwrap().id, "office-a1");

    let mut replacement = serde_json::to_value(&imported.templates[0]).unwrap();
    replacement["revision"] = json!("2");
    replacement["block_names"] = json!(["REPLACED"]);
    let mut generic = serde_json::to_value(&builtin.templates[0]).unwrap();
    generic["revision"] = json!("9");
    generic["block_names"] = json!(["UNUSED"]);
    let mut added = template();
    added["id"] = json!("new");
    let replaced = merged.merged_with(import(registry(vec![generic, replacement, added])));
    assert_eq!(replaced.templates.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["office-a1", "exact", "new", "generic"]);
    assert_eq!(replaced.find("replaced").unwrap().revision, "2");
    assert_eq!(replaced.find("khung-a2").unwrap().id, "new");
    assert_eq!(replaced.find("other").unwrap().revision, "9");
    assert_eq!(replaced.templates.last().unwrap().id, "generic");
    assert_eq!(merged.templates[0].revision, "1");
    assert_eq!(builtin.templates.len(), 1);

    let mut wildcard = template();
    wildcard["id"] = json!("wildcard");
    wildcard["block_names"] = json!(["*"]);
    let any = builtin.merged_with(import(registry(vec![wildcard])));
    for name in ["anything", ""] {
        assert_eq!(any.find(name).unwrap().id, "wildcard");
    }
}

#[test]
fn import_rejects_hostile() {
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let valid = registry(vec![template()]).to_string();
    let mut padded = valid.clone();
    padded.push_str(&" ".repeat(MAX_BYTES - padded.len()));
    assert_eq!(padded.len(), MAX_BYTES);
    assert!(FrameRegistry::from_json(&padded).is_ok());
    padded.push(' ');
    let mut cases = vec![padded, "{".into(), "null".into(), "[]".into(), "".into()];

    let templates: Vec<Value> = (0..1000)
        .map(|index| {
            let mut value = template();
            value["id"] = json!(format!("office-{index}"));
            value
        })
        .collect();
    assert_eq!(import(registry(templates.clone())).templates.len(), 1000);
    let mut too_many = templates;
    too_many.push(template());
    cases.push(registry(too_many).to_string());
    let mut names = template();
    names["block_names"] = json!((0..64).map(|index| format!("FRAME-{index}")).collect::<Vec<_>>());
    assert_eq!(import(registry(vec![names.clone()])).templates[0].block_names.len(), 64);
    names["block_names"].as_array_mut().unwrap().push(json!("FRAME-65"));
    cases.push(registry(vec![names]).to_string());

    for (min, max) in [
        (json!([1, 0]), json!([1, 1])),
        (json!([2, 0]), json!([1, 1])),
        (json!([0, 1]), json!([1, 1])),
        (json!([0, 2]), json!([1, 1])),
        (json!([null, 0]), json!([1, 1])),
        (json!([0, 0]), json!([1, null])),
    ] {
        let mut value = template();
        value["geometry"] = json!([{"kind": "local_rect", "min": min, "max": max}]);
        cases.push(registry(vec![value]).to_string());
    }
    for paper in [json!([-1, 420]), json!([297, -1]), json!([0, 420]), json!([297, 0]), json!([null, 420])] {
        let mut value = template();
        value["media_mm"] = json!([paper]);
        cases.push(registry(vec![value]).to_string());
    }
    for number in ["NaN", "Infinity", "-Infinity", "1e400"] {
        let mut value = template();
        value["geometry"] = json!([{"kind": "local_rect", "min": ["BAD_NUMBER", 0], "max": [1, 1]}]);
        cases.push(registry(vec![value]).to_string().replace("\"BAD_NUMBER\"", number));
        let mut value = template();
        value["media_mm"] = json!([[297, "BAD_NUMBER"]]);
        cases.push(registry(vec![value]).to_string().replace("\"BAD_NUMBER\"", number));
    }
    for (key, value) in [("schema_version", json!("2.0")), ("schema_version", Value::Null), ("templates", json!({}))] {
        let mut input = registry(vec![template()]);
        input[key] = value;
        cases.push(input.to_string());
    }
    let mut unknown_geometry = template();
    unknown_geometry["geometry"] = json!([{"kind": "unknown"}]);
    cases.push(registry(vec![unknown_geometry]).to_string());
    cases.push(format!(r#"{{"schema_version":"1.0","templates":[],"nested":{}null{}}}"#, "[".repeat(160), "]".repeat(160)));
    for (index, text) in cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| FrameRegistry::from_json(text));
        assert!(result.expect("hostile registry input must never panic").is_err(), "case {index}");
    }
}
