#[allow(dead_code)]
mod support;

use std::cell::RefCell;

use cadcraft_doc::{Common, Layer, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use cadcraft_engine::sheetplan::{PlotScale, Severity, SheetPlan, codes, parse_plan};
use serde_json::{Value, json};
use support::frames::{frame_block, place, polyline};

thread_local! {
    static PLOT_OPTIONS: RefCell<Value> = const { RefCell::new(Value::Null) };
}

fn session(insunits: i64, measurement: i64) -> Session {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        // UBPLOT scans hash the drawing through the writer.
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, options| {
            PLOT_OPTIONS.with(|stored| *stored.borrow_mut() = options.clone());
            cadcraft_io::plotter::plot(d, space, options).map_err(|error| error.to_string())
        }),
    });
    let mut s = Session::new();
    let d = s.doc_mut().unwrap();
    d.header.set_i64("INSUNITS", insunits);
    d.header.set_i64("MEASUREMENT", measurement);
    s
}

fn rectangle(s: &mut Session, width: f64, height: f64) -> usize {
    let d = s.doc_mut().unwrap();
    let layer = d.layers.len();
    d.layers.push(Layer { name: "Scale rectangle".into(), ..Layer::default() });
    d.add(&Space::Model, Common { layer: "Scale rectangle".into(), ..Common::default() }, polyline(width, height)).unwrap();
    layer
}

fn path_bbox(s: &mut Session, command: &str, layer: usize, params: Value) -> [f64; 2] {
    assert_eq!(params["compress"], false);
    let result = s.execute(command, &params).unwrap();
    let bytes = base64_decode(result["data"].as_str().unwrap()).unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(!pdf.contains("/FlateDecode"));
    let marker = format!("/OC /L{layer} BDC\n");
    assert_eq!(pdf.matches(&marker).count(), 1);
    let path = pdf.split_once(&marker).unwrap().1.split_once("EMC").unwrap().0;
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut vertices = 0;
    for line in path.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if let [x, y, "m" | "l"] = fields.as_slice() {
            let point = [x.parse::<f64>().unwrap(), y.parse::<f64>().unwrap()];
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
            vertices += 1;
        }
    }
    assert!(vertices >= 4, "rectangle path required: {path}");
    [max[0] - min[0], max[1] - min[1]]
}

fn assert_bbox(actual: [f64; 2], expected: [f64; 2]) {
    for (got, want) in actual.into_iter().zip(expected) {
        assert!((got - want).abs() <= 0.1, "{got} != {want} pt");
    }
}

#[test]
fn ubplot_meters_frame_scale_label() {
    let mut s = session(6, 1);
    frame_block(s.doc_mut().unwrap(), "A1", 841.0, 594.0);
    let handle = place(&mut s, "A1", [0.0, 0.0], 0.1, 0.0, false, &[]);
    let value = s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()]})).unwrap();
    let plan: SheetPlan = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(plan.sheets[0].plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 0.1 });
    assert_eq!(plan.sheets[0].scale_label.as_deref(), Some("1:100"));
    assert_eq!(plan.sheets[0].paper_mm, [594.0, 841.0]);
    assert!(!plan.issues.iter().any(|entry| entry.code == codes::UNITS_ASSUMED));
    assert_eq!(parse_plan(&value).unwrap(), plan);
    let supplied = s.execute("ubplot.scan", &json!({"mode": "plan", "plan": value})).unwrap();
    let rescanned: SheetPlan = serde_json::from_value(supplied).unwrap();
    assert_eq!(rescanned.sheets[0].scale_label, plan.sheets[0].scale_label);
    assert_eq!(rescanned.sheets[0].plot_scale, plan.sheets[0].plot_scale);
    let rounded = place(&mut s, "A1", [1000.0, 0.0], 0.123456789, 0.0, false, &[]);
    let result = s.execute("ubplot.scan", &json!({"mode": "block", "handles": [rounded.hex()]})).unwrap();
    let rounded: SheetPlan = serde_json::from_value(result).unwrap();
    assert_eq!(rounded.sheets[0].scale_label.as_deref(), Some("1:123.457"));
    assert_eq!(rounded.sheets[0].plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 0.123456789 });
}

#[test]
fn insunits_unset_assumes_mm_with_info() {
    let mut s = session(0, 1);
    let params = json!({"mode": "window", "windows": [[[0, 0], [100, 50]]], "scale": "1:50"});
    let value = s.execute("ubplot.scan", &params).unwrap();
    let plan: SheetPlan = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(plan.sheets[0].plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 });
    assert_eq!(plan.sheets[0].scale_label.as_deref(), Some("1:50"));
    let issues: Vec<_> = plan.issues.iter().filter(|entry| entry.code == codes::UNITS_ASSUMED).collect();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, Severity::Info);
    assert_eq!(issues[0].sheet_id, None);
    let repeated = s.execute("ubplot.scan", &json!({"mode": "plan", "plan": value})).unwrap();
    let repeated: SheetPlan = serde_json::from_value(repeated).unwrap();
    assert_eq!(repeated.issues.iter().filter(|entry| entry.code == codes::UNITS_ASSUMED).count(), 1);
    for insunits in [-1, 22] {
        s.doc_mut().unwrap().header.set_i64("INSUNITS", insunits);
        let unknown: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &params).unwrap()).unwrap();
        assert_eq!(unknown.sheets[0].plot_scale, plan.sheets[0].plot_scale);
        assert!(unknown.issues.iter().any(|entry| entry.code == codes::UNITS_ASSUMED && entry.severity == Severity::Info));
    }
}

#[test]
fn insunits_meters_plot_1_100() {
    for measurement in [1, 0] {
        let mut s = session(6, measurement);
        // 84.1 × 59.4 m at 1:100 fills an A1 sheet; fit:false keeps automatic Fit from hiding the unit conversion.
        let layer = rectangle(&mut s, 84.1, 59.4);
        let size = path_bbox(
            &mut s,
            "plot",
            layer,
            json!({"layout": "Model", "paper": "A1", "landscape": true, "scaleMode": "real", "scale": "1:100",
                "fit": false, "margins": [0, 0, 0, 0], "compress": false, "lineweights": false}),
        );
        assert_bbox(size, [2383.94, 1683.78]);
    }
}

#[test]
fn inches_drawing_real_ratio_ignores_measurement() {
    for command in ["plot", "exportpdf"] {
        let mut sizes = Vec::new();
        for measurement in [0, 1] {
            let mut s = session(1, measurement);
            let layer = rectangle(&mut s, 12.0, 6.0);
            let size = path_bbox(
                &mut s,
                command,
                layer,
                json!({"layout": "Model", "paper": "A1", "landscape": true, "scaleMode": "real", "scale": "1:12",
                    "fit": false, "margins": [0, 0, 0, 0], "compress": false, "lineweights": false}),
            );
            assert_bbox(size, [72.0, 36.0]);
            sizes.push(size);
        }
        assert_bbox(sizes[0], sizes[1]);
    }
}

#[test]
fn plot_default_scale_meaning_is_unchanged() {
    for command in ["plot", "exportpdf"] {
        let mut s = session(6, 1);
        let layer = rectangle(&mut s, 10.0, 6.0);
        let size = path_bbox(
            &mut s,
            command,
            layer,
            json!({"layout": "Model", "paper": "A1", "scale": "1:2", "fit": false,
                "margins": [0, 0, 0, 0], "compress": false, "lineweights": false}),
        );
        PLOT_OPTIONS.with(|options| assert_eq!(options.borrow()["scale"], 0.5));
        assert_bbox(size, [14.1732, 8.5039]);
    }
}

#[test]
fn real_ratio_validates_units_and_converted_terms() {
    assert_eq!(PlotScale::from_real_ratio("1:100", 1000.0), Some(PlotScale::Ratio { paper_mm: 1.0, drawing_units: 0.1 }));
    assert_eq!(PlotScale::from_real_ratio("fit", 1000.0), Some(PlotScale::Fit));
    for unit in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        assert!(PlotScale::from_real_ratio("1:100", unit).is_none());
    }
    for text in ["0:100", "1:0", "-1:100", "1:-100", "NaN:100", "1:inf", "1:1e-300", "1e308:1e-300", "bad"] {
        assert!(PlotScale::from_real_ratio(text, 1e300).is_none(), "{text}");
    }
}

#[test]
fn scan_real_ratio_overrides_preserve_labels() {
    let mut s = session(6, 0);
    let result = s
        .execute(
            "ubplot.scan",
            &json!({"mode": "window", "windows": [[[0, 0], [10, 5]]], "scale": "1:100",
                "overrides": {"S-001@1": {"scale": "2:50"}}}),
        )
        .unwrap();
    let plan: SheetPlan = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(plan.sheets[0].plot_scale, PlotScale::Ratio { paper_mm: 2.0, drawing_units: 0.05 });
    assert_eq!(plan.sheets[0].scale_label.as_deref(), Some("2:50"));
    assert_eq!(parse_plan(&result).unwrap(), plan);
}
