#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use cadcraft_engine::cmd::plotters::{PlotterStore, find_device, set_plotter_store};
use cadcraft_engine::sheetplan::ids::sheet_key;
use cadcraft_engine::sheetplan::split::propose_split;
use cadcraft_engine::sheetplan::{FrameRegistry, PreflightContext, Severity, SheetPlan, SheetSource, SheetStatus, codes, preflight};
use serde_json::json;
use support::pdf_probe::pages;

fn setup() -> Session {
    static DEVICE: OnceLock<()> = OnceLock::new();
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, options| cadcraft_io::plotter::plot(d, space, options).map_err(|error| error.to_string())),
    });
    DEVICE.get_or_init(|| {
        set_plotter_store(PlotterStore::memory());
        let mut s = Session::new();
        s.execute("plottermanager", &json!({"action": "new", "name": "A1 Plotter", "paper": {"name": "A1 test", "width": 594, "height": 841}}))
            .unwrap();
        let config = s.execute("plottermanager", &json!({"action": "get", "name": "A1 Plotter"})).unwrap();
        for paper in config["papers"].as_array().unwrap().iter().filter(|paper| paper["name"] != "A1 test") {
            s.execute("plottermanager", &json!({"action": "set", "name": "A1 Plotter", "removePaper": paper["name"]})).unwrap();
        }
    });
    let mut s = Session::new();
    // Fixtures are in millimetres; the default drawing is in inches, which UBPLOT now scales physically.
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    s
}

fn scan(s: &mut Session, width: f64, height: f64, device: &str) -> SheetPlan {
    serde_json::from_value(
        s.execute(
            "ubplot.scan",
            &json!({
                "mode": "window", "windows": [[[0, 0], [width, height]]],
                "paper": format!("{width}x{height}"), "scale": "1:1", "device": device
            }),
        )
        .unwrap(),
    )
    .unwrap()
}

fn approve_split(s: &mut Session, plan: &SheetPlan, overlap: Option<f64>) -> SheetPlan {
    let mut item = json!({"sheetKey": sheet_key(&plan.sheets[0])});
    if let Some(overlap) = overlap {
        item["overlapMm"] = json!(overlap);
    }
    serde_json::from_value(s.execute("ubplot.approve", &json!({"plan": plan, "split": [item], "by": "user"})).unwrap()).unwrap()
}

fn published_bytes(s: &mut Session) -> Vec<u8> {
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    assert!(result["failed"].as_array().unwrap().is_empty());
    base64_decode(result["files"][0]["data"].as_str().unwrap()).unwrap()
}

#[test]
fn near_device_limit_warns() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 800.0, "A1 Plotter");
    assert!(plan.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_NEAR && issue.severity == Severity::Warn));
    assert_eq!(plan.sheets[0].status, SheetStatus::NeedsPreview, "{:?}", plan.sheets[0].issues);
    assert!(plan.sheets[0].split.is_none());
}

#[test]
fn over_device_limit_blocks_with_split_proposal() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "A1 Plotter");
    assert!(plan.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_EXCEEDED && issue.severity == Severity::Block));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    let split = plan.sheets[0].split.as_ref().unwrap_or_else(|| panic!("no split: {:?}", plan.sheets[0].issues));
    assert_eq!(split.pieces.len(), 3);
    assert_eq!(split.overlap_mm, 20.0);
    assert!(!split.approved);
    assert!(s.execute("ubplot.publish", &json!({})).is_err());
}

#[test]
fn split_requires_explicit_approval_and_is_audited() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "A1 Plotter");
    assert!(s.execute("ubplot.publish", &json!({})).is_err());
    let approved = approve_split(&mut s, &plan, None);
    assert_eq!(approved.audit.len(), 1);
    assert_eq!(approved.audit[0].action, "split");
    assert_eq!(approved.audit[0].by, "user");
    assert_eq!(approved.audit[0].sheet_key, "S-001@1");
    assert_eq!(approved.sheets[0].status, SheetStatus::Ready);
    assert!(!approved.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_EXCEEDED));
    let split = approved.sheets[0].split.as_ref().unwrap();
    assert!(split.approved);
    for piece in &split.pieces {
        assert!((piece[2][1] - piece[0][1]) <= 841.0);
    }
    for adjacent in split.pieces.windows(2) {
        assert!((adjacent[0][2][1] - adjacent[1][0][1] - 20.0).abs() < 1e-8);
    }
    let bytes = published_bytes(&mut s);
    let output = pages(&bytes);
    assert_eq!(output.len(), 3);
    for (index, page) in output.iter().enumerate() {
        assert_eq!(page.label.as_deref(), Some(format!("S-001 ({}/3)", index + 1).as_str()));
        assert!(page.media_box[2].max(page.media_box[3]) * page.user_unit <= 841.0 * 72.0 / 25.4 + 0.01);
    }
    let rescanned: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap()).unwrap();
    assert_eq!(rescanned.audit, approved.audit);
    assert!(rescanned.sheets[0].split.as_ref().unwrap().approved);
    assert_eq!(pages(&published_bytes(&mut s)).len(), 3);
}

#[test]
fn split_never_happens_automatically() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "A1 Plotter");
    assert!(s.execute("ubplot.publish", &json!({"plan": plan})).is_err());
    let result = s.execute("ubplot.publish", &json!({"output": "split"})).unwrap();
    assert!(result["files"].as_array().unwrap().is_empty());
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    let mut forged = plan.clone();
    forged.sheets[0].split.as_mut().unwrap().approved = true;
    assert!(s.execute("ubplot.publish", &json!({"plan": forged})).is_err());
    let approved = approve_split(&mut s, &plan, None);
    let mut unapproved = approved.clone();
    unapproved.sheets[0].split.as_mut().unwrap().approved = false;
    assert!(s.execute("ubplot.publish", &json!({"plan": unapproved})).is_err());
    let mut forged = approved;
    forged.sheets[0].split.as_mut().unwrap().pieces[0][0][0] += 1.0;
    assert!(s.execute("ubplot.publish", &json!({"plan": forged})).is_err());
}

#[test]
fn short_side_too_wide_cannot_split() {
    let mut s = setup();
    let plan = scan(&mut s, 900.0, 2400.0, "A1 Plotter");
    assert!(plan.sheets[0].split.is_none());
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert!(s.execute("ubplot.approve", &json!({"split": [{"sheetKey": "S-001@1"}], "by": "user"})).is_err());
    assert!(s.execute("ubplot.publish", &json!({})).is_err());
}

#[test]
fn missing_device_falls_back_to_pdf_with_warn() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "Nope");
    let sheet = &plan.sheets[0];
    assert!(sheet.issues.iter().any(|issue| issue.code == codes::DEVICE_UNAVAILABLE && issue.severity == Severity::Warn));
    assert!(!sheet.issues.iter().any(|issue| issue.severity == Severity::Block), "{:?}", sheet.issues);
    assert!(sheet.split.is_none());
    assert!(s.execute("ubplot.publish", &json!({})).is_err());
    s.execute("ubplot.approve", &json!({"approvals": [{"sheetKey": sheet_key(sheet), "code": codes::DEVICE_UNAVAILABLE}], "by": "user"})).unwrap();
    assert_eq!(pages(&published_bytes(&mut s)).len(), 1);
}

#[test]
fn web_context_marks_devices_unavailable() {
    let mut s = setup();
    let mut plan = scan(&mut s, 594.0, 2400.0, "A1 Plotter");
    let registry = FrameRegistry::builtin();
    let revisions = BTreeMap::new();
    let device = find_device("A1 Plotter").unwrap();
    let context =
        PreflightContext { registry: &registry, device: Some(&device), device_missing: false, revisions: &revisions, fonts: &[], web: true };
    preflight(s.doc().unwrap(), &mut plan, &context);
    let first = plan.clone();
    preflight(s.doc().unwrap(), &mut plan, &context);
    assert_eq!(plan, first);
    assert!(plan.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_UNAVAILABLE && issue.severity == Severity::Warn));
    assert!(!plan.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_EXCEEDED));
    assert!(plan.sheets[0].split.is_none());
}

#[test]
fn split_approval_is_atomic_with_warnings_and_hostile_overlap() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "A1 Plotter");
    for overlap in [json!(-1), json!(201), json!("NaN")] {
        assert!(s.execute("ubplot.approve", &json!({"split": [{"sheetKey": "S-001@1", "overlapMm": overlap}], "by": "user"})).is_err());
        assert_eq!(s.ubplot.last.as_ref().unwrap().plan, plan);
    }
    let original = plan.clone();
    assert!(
        s.execute(
            "ubplot.approve",
            &json!({"split": [{"sheetKey": "S-001@1"}], "approvals": [{"sheetKey": "unknown", "code": "unknown"}], "by": "user"})
        )
        .is_err()
    );
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, original);
    let mut warned = plan.clone();
    warned.issues.push(codes::issue(Severity::Warn, codes::FONT_FALLBACK, "missing font is drawn with stroke", None));
    assert!(
        s.execute(
            "ubplot.approve",
            &json!({
                "plan": warned, "approvals": [{"sheetKey": "*", "code": codes::FONT_FALLBACK}],
                "split": [{"sheetKey": "S-001@1"}, {"sheetKey": "unknown"}], "by": "user"
            })
        )
        .is_err()
    );
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, original);
    let approved = approve_split(&mut s, &plan, Some(0.0));
    assert_eq!(approved.sheets[0].split.as_ref().unwrap().overlap_mm, 0.0);
    let changed = approve_split(&mut s, &approved, Some(200.0));
    assert_eq!(changed.audit.len(), 2);
    assert_eq!(changed.audit[0], approved.audit[0]);
    assert_eq!(changed.sheets[0].split.as_ref().unwrap().overlap_mm, 200.0);
}

#[test]
fn split_preserves_rotated_outline_and_rejects_hostile_limits() {
    let mut s = setup();
    let mut sheet = scan(&mut s, 594.0, 2400.0, "A1 Plotter").sheets.remove(0);
    let angle = 0.37_f64;
    if let SheetSource::Window { outline, rotation_deg } = &mut sheet.source {
        for [x, y] in outline {
            (*x, *y) = (*x * angle.cos() - *y * angle.sin(), *x * angle.sin() + *y * angle.cos());
        }
        *rotation_deg = angle.to_degrees();
    }
    let split = propose_split(&sheet, 841.0, 594.000001, 20.0).unwrap();
    assert_eq!(split.pieces.len(), 3);
    for pair in split.pieces.windows(2) {
        let a = pair[0][3];
        let b = pair[1][0];
        assert!(((a[0] - b[0]).hypot(a[1] - b[1]) - 20.0).abs() < 1e-8);
    }
    for (long, short, overlap) in
        [(f64::NAN, 594.0, 20.0), (841.0, f64::INFINITY, 20.0), (20.0, 594.0, 20.0), (841.0, 594.0, -1.0), (0.01, 594.0, 0.0)]
    {
        assert!(propose_split(&sheet, long, short, overlap).is_none());
    }
}

#[test]
fn removed_device_requires_pdf_fallback_approval() {
    let mut s = setup();
    s.execute("plottermanager", &json!({"action": "new", "name": "Removed plotter", "from": "A1 Plotter"})).unwrap();
    let plan = scan(&mut s, 594.0, 2400.0, "Removed plotter");
    let approved = approve_split(&mut s, &plan, None);
    s.execute("plottermanager", &json!({"action": "delete", "name": "Removed plotter"})).unwrap();
    assert!(s.execute("ubplot.publish", &json!({})).is_err());
    let fallback: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap()).unwrap();
    assert_eq!(fallback.audit, approved.audit);
    assert!(fallback.sheets[0].split.is_none());
    assert!(!fallback.sheets[0].issues.iter().any(|issue| issue.severity == Severity::Block));
    s.execute("ubplot.approve", &json!({"approvals": [{"sheetKey": "S-001@1", "code": codes::DEVICE_UNAVAILABLE}], "by": "user"})).unwrap();
    assert_eq!(pages(&published_bytes(&mut s)).len(), 1);
}

#[test]
fn builtin_pdf_accepts_long_sheets() {
    let mut s = setup();
    let plan = scan(&mut s, 594.0, 2400.0, "CADCraft PDF");
    assert!(!plan.sheets[0].issues.iter().any(|issue| issue.severity >= Severity::Warn), "{:?} {:?}", plan.sheets[0].issues, plan.sheets[0].paper_mm);
    assert!(plan.sheets[0].split.is_none());
    assert_eq!(pages(&published_bytes(&mut s)).len(), 1);
}
