#[allow(dead_code)]
mod support;

use cadcraft_doc::{Common, EntityKind, Layer, Space};
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use cadcraft_engine::cmd::resolve_alias;
use cadcraft_engine::sheetplan::kcs::{KCS_LAYER, NO_PLOT_AREAS};
use cadcraft_engine::sheetplan::{ScanMode, Severity, SheetPlan, SheetSource, codes, ids::sheet_key};
use cadcraft_engine::{Input, Session};
use cadcraft_geom::{Vec2, Vec3};
use serde_json::{Value, json};
use support::pdf_probe::pages;

fn session() -> Session {
    set_io(IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|error| error.to_string())),
    });
    let mut s = Session::new();
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    s
}

fn add_line(s: &mut Session, layer: &str, a: [f64; 2], b: [f64; 2]) -> String {
    let common = Common { layer: layer.into(), ..Common::default() };
    let kind = EntityKind::Line(cadcraft_doc::Line { a: Vec3::new(a[0], a[1], 0.0), b: Vec3::new(b[0], b[1], 0.0) });
    s.doc_mut().unwrap().add(&Space::Model, common, kind).unwrap().hex()
}

/// A marker whose top-left corner is `(x, top)`, drawn bottom-left to top-right like KL users do.
fn marker(s: &mut Session, x: f64, top: f64, width: f64, height: f64) -> String {
    add_line(s, KCS_LAYER, [x, top - height], [x + width, top])
}

fn scan(s: &mut Session, params: Value) -> SheetPlan {
    s.execute("ubplot.scan", &params).unwrap();
    s.ubplot.last.as_ref().unwrap().plan.clone()
}

fn top_left(plan: &SheetPlan) -> Vec<(i64, i64)> {
    plan.sheets
        .iter()
        .map(|sheet| match &sheet.source {
            SheetSource::Window { outline, .. } => {
                let xs = outline.map(|[x, _]| x);
                let ys = outline.map(|[_, y]| y);
                (xs.iter().copied().fold(f64::INFINITY, f64::min).round() as i64, ys.iter().copied().fold(f64::NEG_INFINITY, f64::max).round() as i64)
            }
            other => panic!("KCS sheets are windows, got {other:?}"),
        })
        .collect()
}

fn labels(plan: &SheetPlan) -> Vec<&str> {
    plan.sheets.iter().map(|sheet| sheet.scale_label.as_deref().unwrap_or("")).collect()
}

#[test]
fn kcs_scan_makes_one_a3_fit_sheet_per_marker_with_snapped_scales() {
    let mut s = session();
    marker(&mut s, 0.0, 0.0, 42000.0, 29700.0);
    marker(&mut s, 50000.0, 0.0, 46200.0, 32670.0);
    marker(&mut s, 100000.0, 0.0, 21000.0, 14850.0);
    // Floating-point noise from a real drawing must still read as a whole scale.
    marker(&mut s, 130000.0, 0.0, 21000.0000003, 14849.9999997);
    marker(&mut s, 160000.0, 0.0, 29700.0, 42000.0);
    let plan = scan(&mut s, json!({"mode": "kcs"}));
    assert_eq!(plan.sheets.len(), 5);
    assert_eq!(labels(&plan), ["1:100", "1:110", "1:50", "1:50", "1:100"]);
    for sheet in &plan.sheets {
        assert_eq!(sheet.paper_mm, [297.0, 420.0]);
        assert!(sheet.paper_name.as_deref().is_some_and(|name| name.contains("A3")), "{:?}", sheet.paper_name);
        assert_eq!(sheet.plot_scale, cadcraft_engine::sheetplan::PlotScale::Fit);
        assert_eq!(sheet.exclude_layers, [KCS_LAYER]);
    }
    let landscape: Vec<_> = plan.sheets.iter().map(|sheet| sheet.landscape).collect();
    assert_eq!(landscape, [true, true, true, true, false]);
    assert_eq!(s.ubplot.last.as_ref().unwrap().request.mode, ScanMode::Kcs);
}

#[test]
fn kcs_paper_option_overrides_a3_and_rescales_the_label() {
    let mut s = session();
    marker(&mut s, 0.0, 0.0, 42000.0, 29700.0);
    let plan = scan(&mut s, json!({"mode": "kcs", "paper": "A4"}));
    assert_eq!(plan.sheets[0].paper_mm, [210.0, 297.0]);
    // 1:141.42 is within 0.5 % of 1:141.
    assert_eq!(labels(&plan), ["1:141"]);

    let mut s = session();
    marker(&mut s, 0.0, 0.0, 2050.0, 1000.0);
    let plan = scan(&mut s, json!({"mode": "kcs", "paper": "100x200"}));
    assert_eq!(labels(&plan), ["1:10.25"]);
}

#[test]
fn kcs_order_is_rows_top_down_then_left_to_right_whatever_the_drawing_order() {
    let mut s = session();
    // Rows of 1:50 sheets; the second sheet of each row is 3 units lower, inside the 14850 / 12.5 row tolerance.
    for (x, top) in [(42000.0, -14850.0), (0.0, 0.0), (21000.0, -14853.0), (21000.0, -3.0), (0.0, -14850.0), (42000.0, 0.0)] {
        marker(&mut s, x, top, 21000.0, 14850.0);
    }
    let plan = scan(&mut s, json!({"mode": "kcs"}));
    assert_eq!(top_left(&plan), [(0, 0), (21000, -3), (42000, 0), (0, -14850), (21000, -14853), (42000, -14850)]);
    let orders: Vec<_> = plan.sheets.iter().map(|sheet| sheet.order).collect();
    assert_eq!(orders, [1, 2, 3, 4, 5, 6]);
    assert_eq!(assert_kcs_order(&plan), 2);
}

#[test]
fn kcs_degenerate_marker_is_skipped_with_a_warning_naming_it() {
    let mut s = session();
    marker(&mut s, 0.0, 0.0, 42000.0, 29700.0);
    let flat = add_line(&mut s, KCS_LAYER, [0.0, -50000.0], [42000.0, -50000.0]);
    let upright = add_line(&mut s, "kcs_plotter", [5.0, 0.0], [5.0, 900.0]);
    let plan = scan(&mut s, json!({"mode": "kcs"}));
    assert_eq!(plan.sheets.len(), 1);
    let warnings: Vec<_> = plan.issues.iter().filter(|issue| issue.code == codes::KCS_LINE_DEGENERATE).collect();
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().all(|issue| issue.severity == Severity::Warn));
    assert!(warnings[0].message.contains(&flat), "{}", warnings[0].message);
    assert!(warnings[1].message.contains(&upright), "{}", warnings[1].message);
}

#[test]
fn kcs_without_markers_reports_the_kcs_error() {
    let mut s = session();
    add_line(&mut s, "0", [0.0, 0.0], [42000.0, 29700.0]);
    let error = s.execute("ubplot.scan", &json!({"mode": "kcs"})).unwrap_err().to_string();
    assert!(error.contains(NO_PLOT_AREAS), "{error}");
    add_line(&mut s, KCS_LAYER, [0.0, 0.0], [0.0, 100.0]);
    let error = s.execute("kcsplotter", &json!({})).unwrap_err().to_string();
    assert!(error.contains(NO_PLOT_AREAS), "{error}");
    assert!(s.ubplot.last.is_none());
}

#[test]
fn kcs_handles_restrict_the_scan_to_selected_markers() {
    let mut s = session();
    let first = marker(&mut s, 0.0, 0.0, 42000.0, 29700.0);
    marker(&mut s, 50000.0, 0.0, 42000.0, 29700.0);
    let third = marker(&mut s, 100000.0, 0.0, 42000.0, 29700.0);
    let other = add_line(&mut s, "0", [0.0, 0.0], [1.0, 1.0]);
    let plan = scan(&mut s, json!({"mode": "kcs", "handles": [third, first, other]}));
    assert_eq!(top_left(&plan), [(0, 0), (100000, 0)]);
    assert!(plan.issues.iter().any(|issue| issue.code == codes::KCS_NOT_MARKER && issue.message.contains(&other)));
    let error = s.execute("ubplot.scan", &json!({"mode": "kcs", "handles": ["zz"]})).unwrap_err().to_string();
    assert!(error.contains("hexadecimal"), "{error}");
}

fn publish_bytes(s: &mut Session) -> Vec<u8> {
    let plan = &s.ubplot.last.as_ref().unwrap().plan;
    let approvals: Vec<_> = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == Severity::Warn)
        .map(|issue| json!({"sheetKey": "*", "code": issue.code}))
        .chain(plan.sheets.iter().flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|issue| issue.severity == Severity::Warn)
                .map(|issue| json!({"sheetKey": sheet_key(sheet), "code": issue.code}))
        }))
        .collect();
    if !approvals.is_empty() {
        s.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap();
    }
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "KCS output is one merged PDF");
    base64_decode(files[0]["data"].as_str().unwrap()).unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle.as_bytes())
}

#[test]
fn kcs_publishes_one_merged_pdf_without_the_marker_layer() {
    let mut s = session();
    s.doc_mut().unwrap().layers.push(Layer::new("WALLS"));
    // Plot on, to prove the markers are left out whatever the layer's plot flag says.
    s.doc_mut().unwrap().layers.push(Layer::new(KCS_LAYER));
    for x in [0.0, 50000.0, 100000.0] {
        marker(&mut s, x, 0.0, 42000.0, 29700.0);
        add_line(&mut s, "WALLS", [x + 1000.0, -1000.0], [x + 20000.0, -20000.0]);
    }
    scan(&mut s, json!({"mode": "kcs"}));
    let bytes = publish_bytes(&mut s);
    let probe = pages(&bytes);
    assert_eq!(probe.len(), 3);
    assert!(probe.iter().all(|page| (page.media_box[2] - 1190.55).abs() < 0.5 && (page.media_box[3] - 841.89).abs() < 0.5), "{probe:?}");
    assert!(contains(&bytes, "WALLS"));
    assert!(!contains(&bytes, KCS_LAYER));
    assert!(s.doc().unwrap().layer(KCS_LAYER).unwrap().plot, "the drawing itself is unchanged");

    // A plain window plot of the same areas still shows the marker layer.
    let windows: Vec<_> = [0.0, 50000.0, 100000.0].iter().map(|x| json!([[x, -29700.0], [x + 42000.0, 0.0]])).collect();
    scan(&mut s, json!({"mode": "window", "windows": windows, "paper": "A3"}));
    assert!(contains(&publish_bytes(&mut s), KCS_LAYER));
}

#[test]
fn kcs_typed_keyword_and_kp_alias_scan_all_markers_and_open_the_review() {
    assert_eq!(resolve_alias("KP"), "kcsplotter");
    assert_eq!(resolve_alias("kl"), "kcsarea");
    let mut s = session();
    marker(&mut s, 0.0, 0.0, 42000.0, 29700.0);
    marker(&mut s, 50000.0, 0.0, 42000.0, 29700.0);
    s.start("ubplot").unwrap();
    assert_eq!(s.current_prompt().unwrap().display(), "Plot frames by [Block/Window/Layout/Last/Kcs] <Block>:");
    s.input(Input::Keyword("Kcs".into())).unwrap();
    assert!(s.running.is_none());
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan.sheets.len(), 2);
    assert!(s.ubplot.review);

    s.ubplot.review = false;
    s.start(&resolve_alias("kp")).unwrap();
    assert!(s.running.is_none());
    assert!(s.ubplot.review);
    assert_eq!(s.ubplot.last.as_ref().unwrap().request.mode, ScanMode::Kcs);
    assert!(s.log.iter().any(|entry| entry == "UBPLOT: 2 sheet(s), 0 blocked, 0 warning(s)"), "{:?}", s.log);
}

#[test]
fn kl_draws_a_marker_on_a_new_non_plotting_layer_in_one_undo_step() {
    let mut s = session();
    s.doc_mut().unwrap().layers.push(Layer::new("Walls"));
    s.doc_mut().unwrap().header.set_str("CLAYER", "Walls");
    s.start(&resolve_alias("KL")).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Specify first point of the plot area");
    s.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Specify last point of the plot area");
    s.input(Input::Point(Vec2::new(42000.0, 29700.0))).unwrap();
    assert!(s.running.is_none());
    let d = s.doc().unwrap();
    let layer = d.layer(KCS_LAYER).unwrap();
    assert!(!layer.plot);
    assert_eq!(d.header.str("CLAYER", "0"), "Walls");
    let markers: Vec<_> = d.model.iter().filter(|e| e.common.layer == KCS_LAYER && matches!(e.kind, EntityKind::Line(_))).collect();
    assert_eq!(markers.len(), 1);
    s.undo().unwrap();
    let d = s.doc().unwrap();
    assert!(d.model.iter().all(|e| e.common.layer != KCS_LAYER));
    assert!(d.layer(KCS_LAYER).is_none(), "one undo removes the marker and the layer it created");
    assert!(d.layer("Walls").is_some());
}

#[test]
fn kl_json_form_keeps_an_existing_layer_and_refuses_flat_areas() {
    let mut s = session();
    s.doc_mut().unwrap().layers.push(Layer::new("kcs_plotter"));
    let result = s.execute("kcsarea", &json!({"from": [10, 10], "to": [0, 0]})).unwrap();
    assert!(result["handle"].is_string());
    let d = s.doc().unwrap();
    assert!(d.layer(KCS_LAYER).unwrap().plot, "an existing layer is left as the user set it");
    assert_eq!(d.layers.iter().filter(|layer| layer.name.eq_ignore_ascii_case(KCS_LAYER)).count(), 1);
    let error = s.execute("kcsarea", &json!({"from": [0, 0], "to": [10, 0]})).unwrap_err().to_string();
    assert!(error.contains("needs a width and a height"), "{error}");
    let error = s.execute("kcsarea", &json!({"from": [0, 0]})).unwrap_err().to_string();
    assert!(error.contains("`to`"), "{error}");
    let plan = scan(&mut s, json!({"mode": "kcs"}));
    assert_eq!(plan.sheets.len(), 1);
}

/// The scales of the LPC test drawing's KCS sheets in plot order: two 1:100, eight 1:110, seven 1:50.
fn lpc_scales() -> Vec<&'static str> {
    [("1:100", 2), ("1:110", 8), ("1:50", 7)].into_iter().flat_map(|(label, n)| std::iter::repeat_n(label, n)).collect()
}

/// Checks the KCS order of `plan` from its own windows: rows top-down (each row's top edge no higher
/// than the previous row's), left to right within a row. Rows are as KCS finds them: top edges within
/// the first window's height / 12.5 of each other. Returns the number of rows.
fn assert_kcs_order(plan: &SheetPlan) -> usize {
    let height = |sheet: &cadcraft_engine::sheetplan::Sheet| match &sheet.source {
        SheetSource::Window { outline, .. } => {
            let ys = outline.map(|[_, y]| y);
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) - ys.iter().copied().fold(f64::INFINITY, f64::min)
        }
        other => panic!("KCS sheets are windows, got {other:?}"),
    };
    let tolerance = height(&plan.sheets[0]) / 12.5;
    let corners = top_left(plan);
    let mut rows = 1;
    for (i, pair) in corners.windows(2).enumerate() {
        let ((left_a, top_a), (left_b, top_b)) = (pair[0], pair[1]);
        if (top_a - top_b).abs() as f64 <= tolerance {
            assert!(left_b > left_a, "sheet {} is not right of sheet {} in its row", i + 2, i + 1);
        } else {
            assert!(top_b < top_a, "sheet {} starts a row above the previous one", i + 2);
            rows += 1;
        }
    }
    rows
}

/// The LPC drawing (path in `UBPLOT_KCS_DWG`; no customer data is committed): only aggregate facts
/// are checked, the order rule is computed from the scan itself.
#[test]
fn kcs_real_drawing_matches_the_kcs_plotter_sheets() {
    let Ok(path) = std::env::var("UBPLOT_KCS_DWG") else {
        eprintln!("UBPLOT_KCS_DWG is not set; skipping the real-drawing KCS check");
        return;
    };
    let mut s = session();
    let bytes = std::fs::read(&path).unwrap();
    let drawing = cadcraft_io::read(&bytes, &path).unwrap();
    s.open_drawing(drawing, "KCS", Some(path.clone()));
    let plan = scan(&mut s, json!({"mode": "kcs"}));
    assert_eq!(plan.sheets.len(), 17);
    assert!(plan.sheets.iter().all(|sheet| sheet.paper_mm == [297.0, 420.0] && sheet.landscape), "all A3 landscape");
    assert_eq!(labels(&plan), lpc_scales());
    let rows = assert_kcs_order(&plan);
    eprintln!("{} sheets in {rows} rows", plan.sheets.len());
}
