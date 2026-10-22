#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use cadcraft_doc::{Common, EntityKind, Handle, Layout, Space, Viewport, XrefStatus};
use cadcraft_engine::Session;
use cadcraft_engine::sheetplan::codes::{self, issue};
use cadcraft_engine::sheetplan::ids::{sha256_hex, sheet_key};
use cadcraft_engine::sheetplan::preflight::{PreflightContext, derive_status, preflight, verdict};
use cadcraft_engine::sheetplan::{AuditEntry, FrameRegistry, Severity, SheetPlan, SheetStatus};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};
use support::frames::{frame_block, nest, place};

fn setup(count: usize) -> (Session, Vec<String>) {
    let mut s = Session::new();
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    frame_block(s.doc_mut().unwrap(), "FRAME", 420.0, 297.0);
    let handles = (0..count).map(|index| place(&mut s, "FRAME", [index as f64 * 500.0, 0.0], 1.0, 0.0, false, &[]).hex()).collect();
    (s, handles)
}

fn xref(s: &mut Session, name: &str, status: XrefStatus) {
    let d = s.doc_mut().unwrap();
    // Cached geometry makes the required entity_bounds overlap test independent of file loading.
    frame_block(d, name, 20.0, 20.0);
    let block = Arc::make_mut(d.blocks.get_mut(name).unwrap());
    block.xref_path = Some(format!("missing/{name}.dxf"));
    block.xref_status = status;
}

fn scan(s: &mut Session, handles: &[String]) -> Value {
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap()
}

fn typed(value: &Value) -> SheetPlan {
    serde_json::from_value(value.clone()).unwrap()
}

fn check(s: &Session, plan: &mut SheetPlan, web: bool) {
    let registry = FrameRegistry::builtin();
    let revisions = BTreeMap::new();
    let ctx = PreflightContext { registry: &registry, device: None, device_missing: false, revisions: &revisions, fonts: &[], web };
    preflight(s.doc().unwrap(), plan, &ctx);
}

#[test]
fn essential_missing_xref_blocks_the_sheet() {
    for status in [XrefStatus::NotFound, XrefStatus::Unresolved, XrefStatus::NotLoaded] {
        let (mut s, handles) = setup(2);
        xref(&mut s, "SITE", status);
        place(&mut s, "SITE", [520.0, 30.0], 1.0, 0.0, false, &[]);
        let plan = typed(&scan(&mut s, &handles));
        assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
        assert_eq!(verdict(&plan.sheets[0]), Severity::Pass);
        let sheet = &plan.sheets[1];
        assert_eq!(sheet.status, SheetStatus::Blocked);
        assert_eq!(verdict(sheet), Severity::Block);
        let entry = sheet.issues.iter().find(|entry| entry.code == codes::XREF_MISSING).unwrap();
        assert_eq!(entry.severity, Severity::Block);
        assert_eq!(entry.sheet_id.as_deref(), Some("S-002"));
        assert!(entry.message.contains("SITE") && entry.message.contains("missing/SITE.dxf"));
        assert!(!plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
        assert!(s.execute("ubplot.publish", &json!({})).unwrap_err().to_string().contains("BLOCKED: S-002@2"));
    }
}

#[test]
fn geometry_free_missing_xref_blocks_by_insertion_point() {
    let (mut s, handles) = setup(2);
    let d = s.doc_mut().unwrap();
    let mut block = cadcraft_doc::Block::new("SITE");
    block.xref_path = Some("missing/SITE.dxf".into());
    block.xref_status = XrefStatus::NotFound;
    d.blocks.insert("SITE".into(), Arc::new(block));
    place(&mut s, "SITE", [520.0, 30.0], 1.0, 0.0, false, &[]);
    place(&mut s, "SITE", [2000.0, 2000.0], 1.0, 0.0, false, &[]);
    let plan = typed(&scan(&mut s, &handles));
    assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
    assert_eq!(plan.sheets[1].status, SheetStatus::Blocked);
    assert!(plan.sheets[1].issues.iter().any(|entry| entry.code == codes::XREF_MISSING));
    assert!(plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
}

#[test]
fn xref_outside_all_sheets_is_info() {
    let (mut s, handles) = setup(2);
    xref(&mut s, "OFFSITE", XrefStatus::NotFound);
    place(&mut s, "OFFSITE", [2000.0, 30.0], 1.0, 0.0, false, &[]);
    let result = scan(&mut s, &handles);
    let plan = typed(&result);
    assert!(plan.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready && verdict(sheet) == Severity::Pass));
    let outside: Vec<_> = plan.issues.iter().filter(|entry| entry.code == codes::XREF_MISSING_OUTSIDE).collect();
    assert_eq!(outside.len(), 1);
    assert_eq!(outside[0].severity, Severity::Info);
    assert_eq!(outside[0].sheet_id, None);
    assert!(outside[0].message.contains("OFFSITE") && outside[0].message.contains("missing/OFFSITE.dxf"));
    assert_eq!(result["summary"], json!({"pass": 2, "info": 0, "warn": 0, "block": 0}));
}

#[test]
fn unloaded_xref_warns() {
    let (mut s, handles) = setup(1);
    xref(&mut s, "SITE", XrefStatus::Unloaded);
    place(&mut s, "SITE", [20.0, 30.0], 1.0, 0.0, false, &[]);
    let plan = typed(&scan(&mut s, &handles));
    let sheet = &plan.sheets[0];
    assert_eq!(sheet.status, SheetStatus::NeedsPreview);
    assert_eq!(verdict(sheet), Severity::Warn);
    let entry = sheet.issues.iter().find(|entry| entry.code == codes::XREF_UNLOADED).unwrap();
    assert_eq!(entry.severity, Severity::Warn);
    assert!(!entry.approved_by_user);
}

#[test]
fn nested_xref_inside_frame_block_is_essential() {
    let (mut s, _) = setup(0);
    xref(&mut s, "SITE", XrefStatus::NotFound);
    nest(s.doc_mut().unwrap(), "INNER", "SITE", [40.0, 30.0], 15.0);
    nest(s.doc_mut().unwrap(), "FRAME", "INNER", [10.0, 10.0], 5.0);
    let frame = place(&mut s, "FRAME", [700.0, 50.0], 2.0, 30.0, false, &[]);
    let plan = typed(&scan(&mut s, &[frame.hex()]));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::XREF_MISSING && entry.severity == Severity::Block));
    assert!(!plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
}

#[test]
fn layout_sheet_checks_xrefs_seen_through_viewports() {
    let (mut s, _) = setup(0);
    xref(&mut s, "SITE", XrefStatus::NotFound);
    place(&mut s, "SITE", [1010.0, 2010.0], 1.0, 0.0, false, &[]);
    let mut seen = Layout::new("Seen", 1);
    seen.entities.push(cadcraft_doc::Entity::new(
        s.doc_mut().unwrap().new_handle(),
        EntityKind::Viewport(Viewport {
            center: Vec2::new(100.0, 100.0).to3(0.0),
            width: 200.0,
            height: 100.0,
            view_center: Vec2::new(1020.0, 2020.0),
            view_height: 100.0,
            ..Default::default()
        }),
    ));
    let mut unseen = Layout::new("Unseen", 2);
    unseen.entities.push(cadcraft_doc::Entity::new(
        s.doc_mut().unwrap().new_handle(),
        EntityKind::Viewport(Viewport { view_center: Vec2::ZERO, view_height: 100.0, width: 200.0, height: 100.0, ..Default::default() }),
    ));
    s.doc_mut().unwrap().layouts = vec![seen, unseen];
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Seen", "Unseen"]})).unwrap());
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::XREF_MISSING));
    assert_eq!(plan.sheets[1].status, SheetStatus::Ready);
    assert_eq!(verdict(&plan.sheets[1]), Severity::Pass);
    assert!(!plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
    let d = s.doc_mut().unwrap();
    let viewport = d.layouts[0].entities.iter().next().unwrap().handle;
    assert!(d.layouts[0].entities.modify(viewport, |entity| {
        if let EntityKind::Viewport(viewport) = &mut entity.kind {
            viewport.on = false;
        }
    }));
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Seen"]})).unwrap());
    assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
    assert!(plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
}

#[test]
fn status_and_verdict_rules() {
    let (mut s, handles) = setup(1);
    let mut sheet = typed(&scan(&mut s, &handles)).sheets.remove(0);
    sheet.status = SheetStatus::Done;
    assert_eq!(verdict(&sheet), Severity::Pass);
    assert_eq!(derive_status(&sheet, &[]), SheetStatus::Ready);
    sheet.issues.push(issue(Severity::Info, codes::SCALE_NOT_SET, "Only information.", Some(&sheet.sheet_id)));
    assert_eq!(verdict(&sheet), Severity::Info);
    assert_eq!(derive_status(&sheet, &[]), SheetStatus::Ready);
    let mut warning = issue(Severity::Warn, codes::FRAME_OVERLAP, "Frame overlaps.", Some(&sheet.sheet_id));
    sheet.issues.push(warning.clone());
    assert_eq!(verdict(&sheet), Severity::Warn);
    assert_eq!(derive_status(&sheet, &[]), SheetStatus::NeedsPreview);
    warning.approved_by_user = true;
    sheet.issues.pop();
    sheet.issues.push(warning.clone());
    assert_eq!(derive_status(&sheet, &[]), SheetStatus::NeedsPreview);
    let key = sheet_key(&sheet);
    let fingerprint = sha256_hex(format!("{}|{key}|{}", warning.code, warning.message).as_bytes()).chars().take(12).collect();
    let audit = AuditEntry { at_ms: 0, by: "user".into(), action: "approve".into(), sheet_key: key, code: warning.code, fingerprint };
    assert_eq!(derive_status(&sheet, std::slice::from_ref(&audit)), SheetStatus::Ready);
    assert_eq!(verdict(&sheet), Severity::Warn);
    sheet.issues.last_mut().unwrap().message.push_str(" Changed.");
    assert_eq!(derive_status(&sheet, std::slice::from_ref(&audit)), SheetStatus::NeedsPreview);
    let mut block = issue(Severity::Block, codes::PAPER_TOO_SMALL, "Paper is too small.", Some(&sheet.sheet_id));
    block.approved_by_user = true;
    sheet.issues.push(block);
    assert_eq!(verdict(&sheet), Severity::Block);
    assert_eq!(derive_status(&sheet, &[audit]), SheetStatus::Blocked);
}

#[test]
fn preflight_is_idempotent() {
    let (mut s, handles) = setup(2);
    xref(&mut s, "SITE", XrefStatus::NotFound);
    xref(&mut s, "UNLOADED", XrefStatus::Unloaded);
    place(&mut s, "SITE", [20.0, 30.0], 1.0, 0.0, false, &[]);
    place(&mut s, "SITE", [2000.0, 30.0], 1.0, 0.0, false, &[]);
    place(&mut s, "UNLOADED", [520.0, 30.0], 1.0, 0.0, false, &[]);
    let mut plan = typed(&scan(&mut s, &handles));
    plan.sheets[0].issues.push(issue(Severity::Info, codes::SCALE_NOT_SET, "Keep this issue.", Some("S-001")));
    check(&s, &mut plan, false);
    let once = plan.clone();
    check(&s, &mut plan, false);
    assert_eq!(plan, once);
    let d = s.doc_mut().unwrap();
    Arc::make_mut(d.blocks.get_mut("SITE").unwrap()).xref_status = XrefStatus::Loaded;
    Arc::make_mut(d.blocks.get_mut("UNLOADED").unwrap()).xref_status = XrefStatus::Loaded;
    check(&s, &mut plan, false);
    assert!(plan.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready));
    assert!(plan.sheets.iter().all(|sheet| !sheet.issues.iter().any(|entry| entry.code.starts_with("XREF_"))));
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.message == "Keep this issue."));
    assert!(!plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
}

#[test]
fn summary_counts_pass_info_warn_block() {
    let (mut s, _) = setup(0);
    frame_block(s.doc_mut().unwrap(), "CUSTOM", 400.0, 280.0);
    xref(&mut s, "UNLOADED", XrefStatus::Unloaded);
    xref(&mut s, "MISSING", XrefStatus::NotFound);
    let handles = ["FRAME", "CUSTOM", "FRAME", "FRAME"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| place(&mut s, name, [index as f64 * 500.0, 0.0], 1.0, 0.0, false, &[]).hex())
        .collect::<Vec<_>>();
    for x in [1020.0, 1050.0] {
        place(&mut s, "UNLOADED", [x, 30.0], 1.0, 0.0, false, &[]);
    }
    place(&mut s, "MISSING", [1520.0, 30.0], 1.0, 0.0, false, &[]);
    let result = scan(&mut s, &handles);
    let plan = typed(&result);
    assert_eq!(plan.sheets.iter().map(verdict).collect::<Vec<_>>(), [Severity::Pass, Severity::Info, Severity::Warn, Severity::Block]);
    assert_eq!(result["summary"], json!({"pass": 1, "info": 1, "warn": 1, "block": 1}));
}

#[test]
fn web_missing_xref_explains_platform_limit() {
    let (mut s, handles) = setup(1);
    xref(&mut s, "SITE", XrefStatus::NotFound);
    place(&mut s, "SITE", [20.0, 30.0], 1.0, 0.0, false, &[]);
    let mut plan = typed(&scan(&mut s, &handles));
    check(&s, &mut plan, true);
    let entry = plan.sheets[0].issues.iter().find(|entry| entry.code == codes::XREF_MISSING).unwrap();
    assert_eq!(entry.severity, Severity::Block);
    assert!(entry.message.contains("external references are not loaded on the web"));
    assert!(entry.message.contains("SITE") && entry.message.contains("missing/SITE.dxf"));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
}

#[test]
fn preflight_preserves_paper_scale_blocks() {
    let (mut s, handles) = setup(1);
    let result = s.execute("ubplot.scan", &json!({"mode": "block", "handles": handles, "paper": "A4"})).unwrap();
    let mut plan = typed(&result);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::SCALE_PAPER_MISMATCH));
    check(&s, &mut plan, false);
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert_eq!(verdict(&plan.sheets[0]), Severity::Block);
    assert_eq!(result["summary"], json!({"pass": 0, "info": 0, "warn": 0, "block": 1}));
}

#[test]
fn xref_checks_respect_the_frame_space() {
    let (mut s, handles) = setup(1);
    xref(&mut s, "SITE", XrefStatus::NotFound);
    let model = place(&mut s, "SITE", [20.0, 30.0], 1.0, 0.0, false, &[]);
    let frame = Handle::parse_hex(&handles[0]).unwrap();
    let d = s.doc_mut().unwrap();
    d.layouts.push(Layout::new("Paper", 1));
    let entity = d.remove_entity(frame).unwrap();
    d.add(&Space::Paper("Paper".into()), Common::default(), entity.kind.clone()).unwrap();
    let frame = d.layouts.last().unwrap().entities.iter().last().unwrap().handle;
    let handles = [frame.hex()];
    let plan = typed(&scan(&mut s, &handles));
    assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
    assert!(plan.issues.iter().any(|entry| entry.code == codes::XREF_MISSING_OUTSIDE));
    let d = s.doc_mut().unwrap();
    let entity = d.remove_entity(model).unwrap();
    d.add(&Space::Paper("Paper".into()), Common::default(), entity.kind.clone()).unwrap();
    let plan = typed(&scan(&mut s, &handles));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::XREF_MISSING));
}

#[test]
fn cyclic_xref_traversal_blocks_without_panicking() {
    let (mut s, handles) = setup(1);
    xref(&mut s, "CYCLE", XrefStatus::Loaded);
    nest(s.doc_mut().unwrap(), "CYCLE", "CYCLE", [0.0, 0.0], 0.0);
    place(&mut s, "CYCLE", [20.0, 30.0], 1.0, 0.0, false, &[]);
    let mut plan = typed(&scan(&mut s, &handles));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
    check(&s, &mut plan, false);
    let once = plan.clone();
    check(&s, &mut plan, false);
    assert_eq!(plan, once);
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::XREF_SCAN_LIMIT && entry.severity == Severity::Block));
    assert_eq!(verdict(&plan.sheets[0]), Severity::Block);
}
