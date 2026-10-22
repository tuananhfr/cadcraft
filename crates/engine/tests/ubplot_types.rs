use cadcraft_engine::sheetplan::codes::{FRAME_AMBIGUOUS, issue};
use cadcraft_engine::sheetplan::ids::{canonical_json, job_id, sha256_hex, sheet_key};
use cadcraft_engine::sheetplan::{MAX_SCAN_FRAMES, PlotScale, SHEETPLAN_SCHEMA, Severity, SheetMode, SheetSource, SheetStatus, parse_plan};
use serde_json::{Value, json};

const APPENDIX: &str = r#"{"schema_version":"1.0","job_id":"example-001","sheets":[{"sheet_id":"KT-01","mode":"block","block_handle":"A21","frame_template_id":"lpc-v1","paper_mm":[841,2400],"plot_scale":"1:100","order":1,"status":"NEEDS_PREVIEW"}]}"#;

fn appendix() -> Value {
    serde_json::from_str(APPENDIX).unwrap()
}

#[test]
fn appendix_example_parses_and_round_trips() {
    let plan = parse_plan(&appendix()).unwrap();
    let s = &plan.sheets[0];
    assert_eq!((s.sheet_id.as_str(), s.order, s.mode, s.status), ("KT-01", 1, SheetMode::Block, SheetStatus::NeedsPreview));
    assert!(matches!(&s.source, SheetSource::Block { block_handle, .. } if block_handle == "A21"));
    assert!(matches!(&s.source, SheetSource::Block { outline, .. } if *outline == [[0.0; 2]; 4]));
    assert!(matches!(&s.source, SheetSource::Block { rotation_deg, .. } if *rotation_deg == 0.0));
    assert_eq!(s.plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 100.0 });
    assert_eq!(s.paper_mm, [841.0, 2400.0]);
    assert!(!s.landscape);
    assert_eq!(plan.source_hash, "");
    assert!(plan.issues.is_empty() && s.issues.is_empty() && s.attributes.is_empty());
    let text = serde_json::to_string(&plan).unwrap();
    for needle in [
        r#""mode":"block""#,
        r#""status":"NEEDS_PREVIEW""#,
        r#""plot_scale":"1:100""#,
        r#""schema_version":"1.0""#,
        r#""block_handle":"A21""#,
        r#""paper_mm":[841,2400]"#,
    ] {
        assert!(text.contains(needle), "{needle} in {text}");
    }
    assert_eq!(parse_plan(&serde_json::from_str(&text).unwrap()).unwrap(), plan);
}

#[test]
fn plot_scale_accepts_string_and_object_forms() {
    assert_eq!(PlotScale::parse("fit"), Some(PlotScale::Fit));
    assert_eq!(serde_json::from_value::<PlotScale>(json!("fit")).unwrap(), PlotScale::Fit);
    assert_eq!(serde_json::to_string(&PlotScale::Fit).unwrap(), r#""fit""#);
    let expected = PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 };
    for text in ["1:50", "1/50"] {
        assert_eq!(PlotScale::parse(text), Some(expected.clone()));
        assert_eq!(serde_json::from_value::<PlotScale>(json!(text)).unwrap(), expected);
    }
    assert_eq!(serde_json::from_value::<PlotScale>(json!({"ratio": {"paper_mm": 1, "drawing_units": 50}})).unwrap(), expected);
    assert_eq!(serde_json::to_string(&expected).unwrap(), r#""1:50""#);
    assert_eq!(serde_json::to_string(&PlotScale::Ratio { paper_mm: 1.0, drawing_units: 0.5 }).unwrap(), r#""1:0.5""#);
    for text in ["0:1", "1:0", "x", "NaN:1", "1:inf", "-1:2", "1:2:3", "1/2/3", "1e309:1"] {
        assert_eq!(PlotScale::parse(text), None, "{text}");
        assert!(serde_json::from_value::<PlotScale>(json!(text)).is_err(), "{text}");
    }
    for value in [json!({"ratio": {"paper_mm": 0, "drawing_units": 50}}), json!({"ratio": {"paper_mm": 1, "drawing_units": -1}})] {
        assert!(serde_json::from_value::<PlotScale>(value).is_err());
    }
    for number in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(serde_json::to_string(&PlotScale::Ratio { paper_mm: number, drawing_units: 1.0 }).is_err());
        assert!(serde_json::to_string(&PlotScale::Ratio { paper_mm: 1.0, drawing_units: number }).is_err());
    }
}

#[test]
fn severity_orders_pass_info_warn_block() {
    assert!(Severity::Pass < Severity::Info && Severity::Info < Severity::Warn && Severity::Warn < Severity::Block);
    for (severity, name) in [(Severity::Pass, "PASS"), (Severity::Info, "INFO"), (Severity::Warn, "WARN"), (Severity::Block, "BLOCK")] {
        assert_eq!(serde_json::to_value(severity).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<Severity>(json!(name)).unwrap(), severity);
    }
}

#[test]
fn canonical_json_sorts_keys_recursively() {
    assert_eq!(canonical_json(&json!({"b": 1, "a": {"d": 2, "c": 3}})), r#"{"a":{"c":3,"d":2},"b":1}"#);
    let v = json!({"z": [{"b": false, "a": null}, "quote\"\\\n"], "a": []});
    let text = canonical_json(&v);
    assert_eq!(text, r#"{"a":[],"z":[{"a":null,"b":false},"quote\"\\\n"]}"#);
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), v);
    assert_eq!(canonical_json(&json!({})), "{}");
    assert_eq!(canonical_json(&json!(null)), "null");
    let mut nested = Value::Null;
    for _ in 0..256 {
        nested = Value::Array(vec![nested]);
    }
    assert_eq!(canonical_json(&nested), format!("{}null{}", "[".repeat(256), "]".repeat(256)));
}

#[test]
fn sha256_matches_nist_vector() {
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(cadcraft_engine::hash::sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
}

#[test]
fn job_id_is_stable_16_hex_and_depends_on_inputs() {
    let a = job_id("h1", &json!({"mode": "block", "handles": ["A"]}));
    assert_eq!(a.len(), 16);
    assert!(a.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    assert_eq!(a, job_id("h1", &json!({"handles": ["A"], "mode": "block"})));
    assert_eq!(a, sha256_hex(br#"h1{"handles":["A"],"mode":"block"}"#).chars().take(16).collect::<String>());
    assert_ne!(a, job_id("h2", &json!({"mode": "block", "handles": ["A"]})));
    assert_ne!(a, job_id("h1", &json!({"mode": "block", "handles": ["B"]})));
    assert_ne!(job_id("h1", &json!([1, 2])), job_id("h1", &json!([2, 1])));
}

#[test]
fn hostile_plans_are_errors() {
    let sheet = appendix()["sheets"][0].clone();
    let mut cases = vec![Value::Null, json!([])];
    let mut duplicate = appendix();
    duplicate["sheets"] = json!([sheet.clone(), sheet.clone()]);
    cases.push(duplicate);
    let mut many = appendix();
    many["sheets"] = Value::Array(vec![sheet; MAX_SCAN_FRAMES + 1]);
    cases.push(many);
    for (key, value) in [
        ("order", json!(0)),
        ("order", json!(-1)),
        ("order", json!(u64::MAX)),
        ("paper_mm", json!([f64::NAN, 1])),
        ("paper_mm", json!([-1, 1])),
        ("paper_mm", json!([0, 1])),
        ("paper_mm", json!([1])),
        ("plot_scale", json!("1:0")),
        ("block_handle", json!("not-hex")),
    ] {
        let mut plan = appendix();
        plan["sheets"][0][key] = value;
        cases.push(plan);
    }
    for value in [Value::Null, json!({}), json!("sheets")] {
        let mut plan = appendix();
        plan["sheets"] = value;
        cases.push(plan);
    }
    let mut missing_paper = appendix();
    missing_paper["sheets"][0].as_object_mut().unwrap().remove("paper_mm");
    cases.push(missing_paper);
    let mut bad_schema = appendix();
    bad_schema["schema_version"] = json!("2.0");
    cases.push(bad_schema);
    for source in [
        json!({"kind": "layout", "name": "Layout1"}),
        json!({"kind": "block", "block_handle": "A21", "effective_name": "FRAME",
            "outline": [[null, 0], [1, 0], [1, 1], [0, 1]], "rotation_deg": 0}),
        json!({"kind": "block", "block_handle": "A21", "effective_name": "FRAME",
            "outline": [[0, 0], [1, 0], [1, 1], [0, 1]], "rotation_deg": null}),
    ] {
        let mut plan = appendix();
        plan["sheets"][0]["source"] = source;
        cases.push(plan);
    }
    let mut nested = Value::Null;
    for _ in 0..96 {
        nested = json!({"nested": nested});
    }
    let mut deep = appendix();
    deep["sheets"][0]["source"] = nested;
    cases.push(deep);
    for value in cases {
        let result = std::panic::catch_unwind(|| parse_plan(&value));
        let error = result.expect("hostile plans must never panic").expect_err("hostile plans must be rejected");
        assert!(error.starts_with("PLAN_INVALID: "), "{error}");
    }
}

#[test]
fn full_d2_plan_preserves_sources_and_sorts_order() {
    let mut value = appendix();
    value["source_hash"] = json!("abc123");
    value["issues"] = json!([{"severity": "INFO", "code": "SOURCE_HASH_WEAK", "message": "Source hash uses drawing identity"}]);
    let mut block = value["sheets"][0].clone();
    block.as_object_mut().unwrap().remove("block_handle");
    block["source"] = json!({
        "kind": "block", "block_handle": "a21", "effective_name": "FRAME",
        "outline": [[10, 20], [30, 20], [30, 40], [10, 40]], "rotation_deg": 30
    });
    block["landscape"] = json!(true);
    block["status"] = json!("READY");
    block["order"] = json!(3);
    block["attributes"] = json!({"TITLE": "Drawing"});
    let mut window = block.clone();
    window["mode"] = json!("window");
    window["order"] = json!(2);
    window["source"] = json!({"kind": "window", "outline": [[0, 0], [1, 0], [1, 1], [0, 1]], "rotation_deg": 0});
    let mut layout = block.clone();
    layout["mode"] = json!("layout");
    layout["order"] = json!(1);
    layout["source"] = json!({"kind": "layout", "name": "Layout1"});
    value["sheets"] = json!([block, window, layout]);
    let plan = parse_plan(&value).unwrap();
    assert_eq!(plan.schema_version, SHEETPLAN_SCHEMA);
    assert_eq!(plan.source_hash, "abc123");
    assert_eq!(plan.issues[0].severity, Severity::Info);
    assert_eq!(plan.issues[0].sheet_id, None);
    assert!(!plan.issues[0].approved_by_user);
    assert_eq!(plan.sheets.iter().map(|s| s.order).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(plan.sheets[0].source, SheetSource::Layout { name: "Layout1".into() });
    assert_eq!(plan.sheets[1].mode, SheetMode::Window);
    let block = &plan.sheets[2];
    let SheetSource::Block { block_handle, effective_name, rotation_deg, outline } = &block.source else { panic!("expected a block source") };
    assert_eq!(block_handle, "A21");
    assert_eq!(effective_name, "FRAME");
    assert_eq!(*rotation_deg, 30.0);
    assert_eq!(outline[0], [10.0, 20.0]);
    assert!(block.landscape);
    assert_eq!(block.status, SheetStatus::Ready);
    assert_eq!(block.attributes.get("TITLE").map(String::as_str), Some("Drawing"));
    assert_eq!(parse_plan(&serde_json::to_value(&plan).unwrap()).unwrap(), plan);
    assert_eq!(sheet_key(block), "KT-01@3");
}

#[test]
fn short_form_requires_preview_and_infers_landscape() {
    let mut value = appendix();
    value["sheets"][0]["paper_mm"] = json!([420, 297]);
    value["sheets"][0]["status"] = json!("DONE");
    value["sheets"][0]["block_handle"] = json!("a21");
    let plan = parse_plan(&value).unwrap();
    assert!(plan.sheets[0].landscape);
    assert_eq!(plan.sheets[0].status, SheetStatus::NeedsPreview);
    assert!(matches!(&plan.sheets[0].source, SheetSource::Block { block_handle, .. } if block_handle == "A21"));
}

#[test]
fn scan_frame_limit_is_inclusive() {
    let mut value = appendix();
    let sheet = value["sheets"][0].clone();
    value["sheets"] = Value::Array(
        (1..=MAX_SCAN_FRAMES)
            .map(|order| {
                let mut sheet = sheet.clone();
                sheet["order"] = json!(order);
                sheet
            })
            .collect(),
    );
    assert_eq!(parse_plan(&value).unwrap().sheets.len(), MAX_SCAN_FRAMES);
}

#[test]
fn issue_helper_preserves_context_without_approval() {
    let issue = issue(Severity::Block, FRAME_AMBIGUOUS, "Select a window", Some("KT-01"));
    assert_eq!(issue.severity, Severity::Block);
    assert_eq!(issue.code, "FRAME_AMBIGUOUS");
    assert_eq!(issue.message, "Select a window");
    assert_eq!(issue.sheet_id.as_deref(), Some("KT-01"));
    assert!(!issue.approved_by_user);
    let value = serde_json::to_value(&issue).unwrap();
    assert_eq!(value["severity"], "BLOCK");
}
