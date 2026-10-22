#[allow(dead_code)]
mod support;

use std::cell::RefCell;
use std::sync::Arc;

use cadcraft_doc::{Common, EntityKind, Layer, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use serde_json::{Value, json};
use support::frames::{frame_block, place, polyline};
use support::pdf_probe::{pages, path_bbox};

thread_local! {
    static PLOT_OPTIONS: RefCell<Value> = const { RefCell::new(Value::Null) };
}

fn session(measurement: i64) -> Session {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Err("no writer in this test".into()),
        plot: Some(|d, space, options| {
            PLOT_OPTIONS.with(|stored| *stored.borrow_mut() = options.clone());
            let mut options = options.clone();
            options["compress"] = json!(false);
            options["lineweights"] = json!(false);
            cadcraft_io::plotter::plot(d, space, &options).map_err(|error| error.to_string())
        }),
    });
    let mut s = Session::new();
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    s.doc_mut().unwrap().header.set_i64("MEASUREMENT", measurement);
    s
}

fn frame(s: &mut Session, width: f64, height: f64) -> usize {
    let d = s.doc_mut().unwrap();
    let layer = d.layers.len();
    d.layers.push(Layer::new("Measured frame"));
    frame_block(d, "Test frame", width, height);
    let block = Arc::make_mut(d.blocks.get_mut("Test frame").unwrap());
    let handle = block.entities.iter().find(|entity| matches!(entity.kind, EntityKind::LwPolyline(_))).unwrap().handle;
    block.entities.modify(handle, |entity| entity.common.layer = "Measured frame".into());
    layer
}

fn published(s: &mut Session) -> Vec<u8> {
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    assert!(result["failed"].as_array().unwrap().is_empty());
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    base64_decode(result["files"][0]["data"].as_str().unwrap()).unwrap()
}

fn assert_mm(got_pt: f64, want_mm: f64) {
    let want_pt = want_mm * 72.0 / 25.4;
    assert!((got_pt - want_pt).abs() <= 0.05, "{got_pt} pt != {want_pt} pt");
}

#[test]
fn ratio_sheet_prints_at_scale_even_if_frame_smaller_than_paper() {
    for measurement in [1, 0] {
        let mut s = session(measurement);
        let layer = frame(&mut s, 400.0, 280.0);
        let handle = place(&mut s, "Test frame", [1000.0, 2000.0], 1.0, 0.0, false, &[]);
        s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()], "overrides": {"S-001@1": {"paper": "A3", "scale": "1:1"}}}))
            .unwrap();
        let bytes = published(&mut s);
        let pdf_pages = pages(&bytes);
        assert_eq!(pdf_pages.len(), 1);
        assert_mm(pdf_pages[0].media_box[2], 420.0);
        assert_mm(pdf_pages[0].media_box[3], 297.0);
        let [left, bottom, right, top] = path_bbox(&bytes, layer);
        assert_mm(right - left, 400.0);
        assert_mm(top - bottom, 280.0);
        assert_mm((left + right) / 2.0, 210.0);
        assert_mm((bottom + top) / 2.0, 148.5);
        PLOT_OPTIONS.with(|options| {
            let options = options.borrow();
            assert!(options.get("margins").is_none());
            let page = &options["pages"][0];
            assert_eq!(page["fit"], false);
            assert_eq!(page["margins"], json!([0.0, 0.0, 0.0, 0.0]));
            let expected = if measurement == 1 { 1.0 } else { 1.0 / 25.4 };
            assert!((page["scale"].as_f64().unwrap() - expected).abs() < 1e-12);
        });
    }
}

#[test]
fn fit_sheet_still_fills_the_paper() {
    let mut s = session(1);
    let d = s.doc_mut().unwrap();
    let layer = d.layers.len();
    d.layers.push(Layer::new("Measured frame"));
    d.add(&Space::Model, Common { layer: "Measured frame".into(), ..Default::default() }, polyline(100.0, 50.0)).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "window", "windows": [[[0, 0], [100, 50]]], "paper": "A3", "scale": "fit"})).unwrap();
    let bytes = published(&mut s);
    let [left, _, right, _] = path_bbox(&bytes, layer);
    assert_mm(right - left, 420.0);
    PLOT_OPTIONS.with(|options| {
        let options = options.borrow();
        assert_eq!(options["pages"][0]["fit"], true);
        assert!(options["pages"][0].get("scale").is_none());
    });
}

#[test]
fn rotated_paper_space_ratio_frame_stays_centered() {
    let mut s = session(1);
    let layer = frame(&mut s, 400.0, 280.0);
    let handle = place(&mut s, "Test frame", [1000.0, 2000.0], 1.0, 30.0, false, &[]);
    let d = s.doc_mut().unwrap();
    let entity = d.remove_entity(handle).unwrap();
    let handle = d.add(&Space::Paper("Layout1".into()), entity.common.clone(), entity.kind.clone()).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()], "overrides": {"S-001@1": {"paper": "A3", "scale": "1:1"}}}))
        .unwrap();
    let bytes = published(&mut s);
    let [left, bottom, right, top] = path_bbox(&bytes, layer);
    assert_mm(right - left, 400.0);
    assert_mm(top - bottom, 280.0);
    assert_mm((left + right) / 2.0, 210.0);
    assert_mm((bottom + top) / 2.0, 148.5);
}

#[test]
fn publish_layout_uses_device_paper_margins() {
    for (device, expected) in [("CADCraft PNG", [5.0; 4]), ("CADCraft PDF", [0.0; 4])] {
        let mut s = session(1);
        s.execute("pagesetup", &json!({"layout": "Layout1", "device": device, "paper": "A3"})).unwrap();
        s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Layout1"]})).unwrap();
        let bytes = published(&mut s);
        assert_eq!(pages(&bytes).len(), 1);
        PLOT_OPTIONS.with(|options| assert_eq!(options.borrow()["pages"][0]["margins"], json!(expected)));
        let text = String::from_utf8_lossy(&bytes);
        let clip: Vec<f64> =
            text.lines().find(|line| line.ends_with(" re W n")).unwrap().split_whitespace().take(4).map(|field| field.parse().unwrap()).collect();
        assert_mm(clip[0], expected[0]);
        assert_mm(clip[1], expected[1]);
    }
}

fn add_paper_frame(s: &mut Session, name: &str, width: f64, height: f64, at: [f64; 2], scale: f64) -> (String, usize) {
    let layer_name = format!("Measured {name}");
    let d = s.doc_mut().unwrap();
    let layer = d.layers.len();
    d.layers.push(Layer::new(&layer_name));
    frame_block(d, name, width, height);
    let block = Arc::make_mut(d.blocks.get_mut(name).unwrap());
    let handle = block.entities.iter().find(|entity| matches!(entity.kind, EntityKind::LwPolyline(_))).unwrap().handle;
    block.entities.modify(handle, |entity| entity.common.layer = layer_name);
    (place(s, name, at, scale, 0.0, false, &[]).hex(), layer)
}

fn scan_papers(s: &mut Session, params: Value) -> cadcraft_engine::sheetplan::SheetPlan {
    let value = s.execute("ubplot.scan", &params).unwrap();
    cadcraft_engine::sheetplan::parse_plan(&value).unwrap()
}

#[test]
fn iso_a0_to_a5_frames_match_their_paper() {
    let mut s = session(1);
    let names = ["A0", "A1", "A2", "A3", "A4", "A5"];
    let handles: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let paper = cadcraft_render::paper_size(name).unwrap();
            add_paper_frame(&mut s, name, paper.height_mm, paper.width_mm, [index as f64 * 2000.0, 0.0], 1.0).0
        })
        .collect();
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": handles, "paper": "auto"}));
    assert_eq!(plan.sheets.len(), 6);
    for (sheet, name) in plan.sheets.iter().zip(names) {
        let paper = cadcraft_render::paper_size(name).unwrap();
        assert_eq!(sheet.paper_name.as_deref(), Some(paper.name));
        assert_eq!(sheet.paper_mm, [paper.width_mm, paper.height_mm]);
        assert!(sheet.landscape);
        assert!(sheet.issues.iter().all(|entry| entry.severity != cadcraft_engine::sheetplan::Severity::Block));
    }
}

#[test]
fn long_sheet_841x2400_custom_and_a1x4_named() {
    use cadcraft_engine::sheetplan::{Severity, codes};

    let mut s = session(1);
    let custom = add_paper_frame(&mut s, "Custom long", 841.0, 2400.0, [0.0, 0.0], 1.0).0;
    let elongated = add_paper_frame(&mut s, "Elongated", 841.0, 2378.0, [2000.0, 0.0], 1.0).0;
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": [custom, elongated]}));
    assert_eq!(plan.sheets[0].paper_mm, [841.0, 2400.0]);
    assert_eq!(plan.sheets[0].paper_name.as_deref(), Some("841x2400"));
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::PAPER_CUSTOM && entry.severity == Severity::Info));
    assert_eq!(plan.sheets[1].paper_mm, [841.0, 2378.0]);
    assert_eq!(plan.sheets[1].paper_name.as_deref(), Some("A1x4"));
    assert!(plan.sheets[1].issues.iter().all(|entry| entry.code != codes::PAPER_CUSTOM));
    let named =
        scan_papers(&mut s, json!({"mode": "plan", "plan": plan, "overrides": {"S-001@1": {"paper": "841x2400"}, "S-002@2": {"paper": "A1x4"}}}));
    assert_eq!(named.sheets[0].paper_mm, plan.sheets[0].paper_mm);
    assert_eq!(named.sheets[1].paper_name, plan.sheets[1].paper_name);
}

#[test]
fn roll_914_length_follows_frame() {
    use cadcraft_engine::sheetplan::Severity;

    let mut s = session(1);
    let (handle, layer) = add_paper_frame(&mut s, "Roll frame", 841.0, 3000.0, [0.0, 0.0], 1.0);
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": [handle], "paper": "roll:914:5"}));
    let sheet = &plan.sheets[0];
    assert_eq!(sheet.paper_mm, [914.0, 3010.0]);
    assert_eq!(sheet.paper_name.as_deref(), Some("roll:914:5"));
    assert!(!sheet.landscape);
    assert!(sheet.issues.iter().all(|entry| entry.severity != Severity::Block));
    let bytes = published(&mut s);
    let pdf_pages = pages(&bytes);
    assert_eq!(pdf_pages.len(), 1);
    assert_mm(pdf_pages[0].media_box[2] * pdf_pages[0].user_unit, 914.0);
    assert_mm(pdf_pages[0].media_box[3] * pdf_pages[0].user_unit, 3010.0);
    let [left, bottom, right, top] = path_bbox(&bytes, layer);
    assert_mm(right - left, 841.0);
    assert_mm(top - bottom, 3000.0);
    PLOT_OPTIONS.with(|options| assert_eq!(options.borrow()["pages"][0]["margins"], json!([5.0, 5.0, 5.0, 5.0])));
}

#[test]
fn roll_frame_wider_than_roll_blocks() {
    use cadcraft_engine::sheetplan::{Severity, SheetStatus, codes};

    let mut s = session(1);
    let handle = add_paper_frame(&mut s, "Too wide", 915.0, 2000.0, [0.0, 0.0], 1.0).0;
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": [handle], "paper": "roll:914"}));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::PAPER_TOO_SMALL && entry.severity == Severity::Block));
    let error = s.execute("ubplot.publish", &json!({})).unwrap_err().to_string();
    assert!(error.contains("BLOCKED"));
    let split = s.execute("ubplot.publish", &json!({"output": "split"})).unwrap();
    assert!(split["files"].as_array().unwrap().is_empty());
    assert_eq!(split["failed"].as_array().unwrap().len(), 1);
}

#[test]
fn scale_paper_mismatch_blocks() {
    use cadcraft_engine::sheetplan::{Severity, SheetStatus, codes};

    let mut s = session(1);
    let handle = add_paper_frame(&mut s, "A1 scale 100", 841.0, 594.0, [0.0, 0.0], 100.0).0;
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": [handle], "overrides": {"S-001@1": {"scale": "1:50"}}}));
    let sheet = &plan.sheets[0];
    assert_eq!(sheet.status, SheetStatus::Blocked);
    assert_eq!(sheet.paper_mm, [594.0, 841.0]);
    assert_eq!(sheet.plot_scale, cadcraft_engine::sheetplan::PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 });
    let error = sheet.issues.iter().find(|entry| entry.code == codes::SCALE_PAPER_MISMATCH).unwrap();
    assert_eq!(error.severity, Severity::Block);
    assert_eq!(error.message, "frame needs 1682×1188 mm at 1:50; paper ISO A1 (594.00 x 841.00 MM) prints 841×594 mm");
    assert!(s.execute("ubplot.publish", &json!({})).unwrap_err().to_string().contains("BLOCKED"));
    let repaired = scan_papers(&mut s, json!({"mode": "plan", "plan": plan, "overrides": {"S-001@1": {"scale": "1:100"}}}));
    assert!(repaired.sheets[0].issues.iter().all(|entry| entry.code != codes::SCALE_PAPER_MISMATCH));
}

#[test]
fn paper_choice_rejects_hostile() {
    use cadcraft_engine::sheetplan::paper::PaperChoice;

    for text in [
        "0x5",
        "NaNx1",
        "99999x1",
        "roll:-1",
        "roll:914:9999",
        "",
        "1xinf",
        "1e309x1",
        "1x1x1",
        "roll:NaN",
        "roll:914:NaN",
        "roll:914:-1",
        "roll:914:5:1",
        "roll:1:5",
        "roll:15001",
        "0.9x1",
        "1x15001",
    ] {
        assert!(PaperChoice::parse(text).is_err(), "accepted {text}");
    }
    assert!(PaperChoice::parse(&"x".repeat(257)).is_err());
    assert_eq!(PaperChoice::parse(" AUTO "), Ok(PaperChoice::Auto));
    assert_eq!(PaperChoice::parse("841X2400"), Ok(PaperChoice::Custom { w: 841.0, h: 2400.0 }));
    assert_eq!(PaperChoice::parse("roll:914"), Ok(PaperChoice::Roll { width: 914.0, margin: 5.0 }));
    assert_eq!(PaperChoice::parse("roll:914:0"), Ok(PaperChoice::Roll { width: 914.0, margin: 0.0 }));
    assert!(matches!(PaperChoice::parse("a1X4"), Ok(PaperChoice::Named(paper)) if paper.name == "A1x4"));
    assert_eq!(PaperChoice::parse("1x15000"), Ok(PaperChoice::Custom { w: 1.0, h: 15000.0 }));
}

#[test]
fn sp05_mixed_a1_a3_long_in_one_pdf_each_own_mediabox() {
    for measurement in [1, 0] {
        let mut s = session(measurement);
        let sizes = [("A1", 841.0, 594.0), ("A3", 420.0, 297.0), ("Long", 841.0, 2400.0)];
        let frames: Vec<_> =
            sizes.iter().enumerate().map(|(index, (name, w, h))| add_paper_frame(&mut s, name, *w, *h, [index as f64 * 2000.0, 0.0], 1.0)).collect();
        let handles: Vec<_> = frames.iter().map(|(handle, _)| handle).collect();
        scan_papers(&mut s, json!({"mode": "block", "handles": handles}));
        let bytes = published(&mut s);
        let pdf_pages = pages(&bytes);
        assert_eq!(pdf_pages.len(), 3);
        let text = String::from_utf8_lossy(&bytes);
        let streams: Vec<_> = text
            .split("\nstream\n")
            .skip(1)
            .filter_map(|stream| stream.split_once("\nendstream").map(|(content, _)| content))
            .filter(|content| content.starts_with("q\n") && content.contains(" re W n\n"))
            .collect();
        assert_eq!(streams.len(), 3);
        let expected = [[2383.94, 1683.78], [1190.55, 841.89], [2383.94, 6803.15]];
        for (index, ((page, [want_w, want_h]), (_, layer))) in pdf_pages.iter().zip(expected).zip(&frames).enumerate() {
            let [left, bottom, right, top] = page.media_box;
            assert!(((right - left) * page.user_unit - want_w).abs() <= 0.05);
            assert!(((top - bottom) * page.user_unit - want_h).abs() <= 0.05);
            assert_eq!(page.label, Some(format!("S-{:03}", index + 1)));
            let (_, w, h) = sizes[index];
            let [left, bottom, right, top] = path_bbox(streams[index].as_bytes(), *layer);
            assert_mm(right - left, w);
            assert_mm(top - bottom, h);
        }
    }
}

#[test]
fn paper_resolver_checks_roll_limits_orientation_and_printable_area() {
    use cadcraft_engine::sheetplan::codes;
    use cadcraft_engine::sheetplan::paper::{PaperChoice, resolve_paper};

    let roll = PaperChoice::parse("roll:914").unwrap();
    let resolved = resolve_paper(&roll, [3000.1, 841.0], [0.0; 4]).unwrap();
    assert_eq!(resolved.paper_mm, [914.0, 3011.0]);
    assert!(resolved.landscape);
    assert_eq!(resolve_paper(&roll, [841.0, 14991.0], [0.0; 4]).err().unwrap().code, codes::PAPER_TOO_LARGE);
    assert_eq!(resolve_paper(&roll, [905.0, 2000.0], [0.0; 4]).err().unwrap().code, codes::PAPER_TOO_SMALL);
    assert_eq!(resolve_paper(&roll, [841.0, 14990.0], [0.0; 4]).unwrap().paper_mm, [914.0, 15000.0]);
    let a3 = PaperChoice::parse("A3").unwrap();
    assert!(resolve_paper(&a3, [420.5, 297.5], [0.0; 4]).is_ok());
    assert_eq!(resolve_paper(&a3, [420.0, 297.0], [5.0; 4]).err().unwrap().code, codes::SCALE_PAPER_MISMATCH);
    assert!(resolve_paper(&a3, [400.0, 277.0], [5.0, 10.0, 5.0, 10.0]).is_ok());
    for size in [[f64::NAN, 1.0], [1.0, f64::INFINITY], [0.0, 1.0], [-1.0, 1.0]] {
        assert!(resolve_paper(&PaperChoice::Auto, size, [0.0; 4]).is_err());
    }
    assert!(resolve_paper(&a3, [100.0, 100.0], [f64::NAN; 4]).is_err());
    for side in [f64::NAN, f64::INFINITY, -1.0, 0.0, 15001.0] {
        assert!(resolve_paper(&PaperChoice::Custom { w: side, h: 100.0 }, [50.0, 50.0], [0.0; 4]).is_err());
    }
}

#[test]
fn paper_name_roundtrips_and_legacy_plan_defaults() {
    let mut s = session(1);
    let handle = add_paper_frame(&mut s, "Roundtrip roll", 841.0, 2000.0, [0.0, 0.0], 1.0).0;
    let plan = scan_papers(&mut s, json!({"mode": "block", "handles": [handle], "paper": "roll:914"}));
    let value = serde_json::to_value(&plan).unwrap();
    assert_eq!(cadcraft_engine::sheetplan::parse_plan(&value).unwrap(), plan);
    let rescanned = scan_papers(&mut s, json!({"mode": "plan", "plan": value}));
    assert_eq!(rescanned.sheets[0].paper_name.as_deref(), Some("roll:914:5"));
    assert_eq!(rescanned.sheets[0].paper_mm, [914.0, 2010.0]);
    let mut legacy = value;
    legacy["sheets"][0].as_object_mut().unwrap().remove("paper_name");
    assert!(cadcraft_engine::sheetplan::parse_plan(&legacy).unwrap().sheets[0].paper_name.is_none());
    let legacy: cadcraft_engine::sheetplan::SheetPlan = serde_json::from_value(legacy).unwrap();
    assert!(legacy.sheets[0].paper_name.is_none());
}
