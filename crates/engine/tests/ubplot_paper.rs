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
        // UBPLOT scans hash the drawing through the writer.
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
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
