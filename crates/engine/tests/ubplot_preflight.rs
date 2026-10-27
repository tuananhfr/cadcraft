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

struct FontSettings {
    map: Vec<(String, String)>,
    alt: String,
    fallback: String,
}

impl FontSettings {
    fn isolated() -> Self {
        let saved = Self { map: cadcraft_fonts::font_map(), alt: cadcraft_fonts::ttf::font_alt(), fallback: cadcraft_fonts::ttf::fallback_fonts() };
        cadcraft_fonts::set_font_map([]);
        cadcraft_fonts::ttf::set_font_alt("zz-unavailable-preflight-alt.ttf");
        cadcraft_fonts::ttf::set_fallback_fonts("");
        saved
    }
}

impl Drop for FontSettings {
    fn drop(&mut self) {
        cadcraft_fonts::set_font_map(self.map.clone());
        cadcraft_fonts::ttf::set_font_alt(&self.alt);
        cadcraft_fonts::ttf::set_fallback_fonts(&self.fallback);
    }
}

fn missing_style(s: &mut Session) {
    missing_style_font(s, "zz-missing-font.ttf");
}

fn missing_style_font(s: &mut Session, font: &str) {
    let d = s.doc_mut().unwrap();
    d.text_styles.push(cadcraft_doc::TextStyle { name: "Missing".into(), font: font.into(), family: String::new(), ..Default::default() });
    let block = Arc::make_mut(d.blocks.get_mut("FRAME").unwrap());
    let handle = block.entities.iter().find(|entity| matches!(&entity.kind, EntityKind::Text(_))).unwrap().handle;
    assert!(block.entities.modify(handle, |entity| {
        if let EntityKind::Text(text) = &mut entity.kind {
            text.style = "Missing".into();
        }
    }));
}

/// The code and message the missing style's font gets: a missing TrueType font is drawn with a
/// Latin outline font when one is installed, else with the stroke font.
fn missing_font_issue() -> (&'static str, String) {
    match cadcraft_fonts::font_info("zz-missing-font.ttf", "").with {
        Some(face) => (codes::FONT_FALLBACK, format!("zz-missing-font.ttf is drawn with {face}")),
        None => (codes::FONT_MISSING, "zz-missing-font.ttf is drawn with the built-in stroke font".into()),
    }
}

#[test]
fn font_missing_warns_at_plan_level() {
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings = FontSettings::isolated();
    let (mut s, handles) = setup(1);
    missing_style(&mut s);
    let plan = typed(&scan(&mut s, &handles));
    let entry = plan.issues.iter().find(|entry| matches!(entry.code.as_str(), codes::FONT_FALLBACK | codes::FONT_MISSING)).unwrap();
    assert_eq!(entry.severity, Severity::Warn);
    assert_eq!(entry.sheet_id, None);
    let (code, message) = missing_font_issue();
    assert_eq!(entry.code, code);
    assert_eq!(entry.message, message);
    assert!(plan.sheets.iter().all(|sheet| !sheet.issues.iter().any(|entry| entry.code.starts_with("FONT_"))));
}

#[test]
fn missing_shx_font_warns_with_the_stroke_font() {
    // Missing SHX fonts never take an outline face, so this holds whatever is installed.
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings = FontSettings::isolated();
    let (mut s, handles) = setup(1);
    missing_style_font(&mut s, "zz-missing-font.shx");
    let plan = typed(&scan(&mut s, &handles));
    let entry = plan.issues.iter().find(|entry| entry.code.starts_with("FONT_")).unwrap();
    assert_eq!((entry.code.as_str(), entry.severity, entry.sheet_id.as_deref()), (codes::FONT_MISSING, Severity::Warn, None));
    assert_eq!(entry.message, "zz-missing-font.shx is drawn with the built-in stroke font");
}

#[test]
fn mapped_font_is_info() {
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings = FontSettings::isolated();
    let (mut s, handles) = setup(1);
    missing_style(&mut s);
    cadcraft_fonts::set_font_map([("zz-missing-font.ttf".into(), cadcraft_fonts::BUILTIN_FONT.into())]);
    let plan = typed(&scan(&mut s, &handles));
    let entry = plan.issues.iter().find(|entry| entry.code == codes::FONT_MAPPED).unwrap();
    assert_eq!(entry.severity, Severity::Info);
    assert_eq!(entry.sheet_id, None);
    assert!(entry.message.contains("zz-missing-font.ttf"));
    assert!(!plan.issues.iter().any(|entry| matches!(entry.code.as_str(), codes::FONT_MISSING | codes::FONT_FALLBACK)));
    cadcraft_fonts::set_font_map([]);
    let refreshed = typed(&s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap());
    assert!(!refreshed.issues.iter().any(|entry| entry.code == codes::FONT_MAPPED));
    assert!(refreshed.issues.iter().any(|entry| entry.code == missing_font_issue().0));
}

#[test]
fn unused_current_and_off_sheet_fonts_do_not_warn() {
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings = FontSettings::isolated();
    let (mut s, handles) = setup(1);
    let d = s.doc_mut().unwrap();
    d.text_styles.push(cadcraft_doc::TextStyle {
        name: "Missing".into(),
        font: "zz-missing-font.ttf".into(),
        family: String::new(),
        ..Default::default()
    });
    d.header.set_str("TEXTSTYLE", "Missing");
    let mut text = support::frames::text("OUTSIDE");
    text.style = "Missing".into();
    text.insert = Vec2::new(3000.0, 3000.0).to3(0.0);
    d.add(&Space::Model, Common::default(), EntityKind::Text(text)).unwrap();
    frame_block(d, "UNUSED", 420.0, 297.0);
    let block = Arc::make_mut(d.blocks.get_mut("UNUSED").unwrap());
    let handle = block.entities.iter().find(|entity| matches!(&entity.kind, EntityKind::Text(_))).unwrap().handle;
    block.entities.modify(handle, |entity| {
        if let EntityKind::Text(text) = &mut entity.kind {
            text.style = "Missing".into();
        }
    });
    let plan = typed(&scan(&mut s, &handles));
    assert!(!plan.issues.iter().any(|entry| entry.code.starts_with("FONT_")));
    assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
}

#[test]
fn duplicate_sheet_ids_warn_on_every_copy() {
    let (mut s, _) = setup(0);
    let handles = ["KT-01", "KT-01", "KT-02"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| place(&mut s, "FRAME", [index as f64 * 500.0, 0.0], 1.0, 0.0, false, &[("SHEET_ID", id)]).hex())
        .collect::<Vec<_>>();
    let plan = typed(&scan(&mut s, &handles));
    let entries: Vec<_> = plan.sheets.iter().flat_map(|sheet| &sheet.issues).filter(|entry| entry.code == codes::SHEET_ID_DUPLICATE).collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.severity == Severity::Warn && entry.sheet_id.as_deref() == Some("KT-01")));
    assert!(plan.sheets[..2].iter().all(|sheet| sheet.status == SheetStatus::NeedsPreview));
    assert_eq!(plan.sheets[2].status, SheetStatus::Ready);
}

#[test]
fn old_revision_against_register_warns() {
    for (revision, expected_code, severity) in [
        ("B", Some(codes::REVISION_OLD), Severity::Warn),
        ("D", Some(codes::REVISION_NEWER), Severity::Info),
        ("", Some(codes::REVISION_OLD), Severity::Warn),
        ("c", None, Severity::Pass),
    ] {
        let (mut s, _) = setup(0);
        let handle = place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[("SHEET_ID", "KT-01"), ("REV", revision)]);
        let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()], "revisions": {"KT-01": "C"}})).unwrap());
        let sheet = &plan.sheets[0];
        let entry = sheet.issues.iter().find(|entry| matches!(entry.code.as_str(), codes::REVISION_OLD | codes::REVISION_NEWER));
        assert_eq!(entry.map(|entry| entry.code.as_str()), expected_code);
        if let Some(entry) = entry {
            assert_eq!(entry.severity, severity);
            assert_eq!(entry.sheet_id.as_deref(), Some("KT-01"));
            assert!(entry.message.contains('C'));
        }
        assert_eq!(sheet.status, if severity == Severity::Warn { SheetStatus::NeedsPreview } else { SheetStatus::Ready });
    }
}

#[test]
fn rev_cmp_natural_order() {
    use std::cmp::Ordering;

    use cadcraft_engine::sheetplan::preflight::rev_cmp;

    for (a, b) in [("B", "C"), ("9", "10"), ("A2", "A10"), ("", "A"), ("A9B2", "a9B10"), ("A18446744073709551616", "A18446744073709551617")] {
        assert_eq!(rev_cmp(a, b), Ordering::Less, "{a} < {b}");
        assert_eq!(rev_cmp(b, a), Ordering::Greater, "{b} > {a}");
    }
    for (a, b) in [("", ""), ("a", "A"), ("A02", "a2"), ("000", "0"), ("Ä2", "ä2")] {
        assert_eq!(rev_cmp(a, b), Ordering::Equal);
    }
}

#[test]
fn superseded_template_warns() {
    let (mut s, handles) = setup(1);
    let template = json!({
        "schema_version": "1.0",
        "templates": [{
            "id": "v1", "revision": "1", "block_names": ["FRAME"],
            "geometry": [{"kind": "largest_closed_rect"}],
            "attributes": {"revision": ["ISSUE_REV"]}, "superseded_by": "v2"
        }]
    });
    s.execute("ubplot.templates", &json!({"action": "import", "text": template.to_string()})).unwrap();
    let mut plan = typed(&scan(&mut s, &handles));
    assert_eq!(plan.sheets[0].frame_template_id.as_deref(), Some("generic@1"));
    plan.sheets[0].frame_template_id = Some("v1@1".into());
    let revisions = BTreeMap::new();
    let ctx = PreflightContext { registry: &s.ubplot.registry, device: None, device_missing: false, revisions: &revisions, fonts: &[], web: false };
    preflight(s.doc().unwrap(), &mut plan, &ctx);
    let entry = plan.sheets[0].issues.iter().find(|entry| entry.code == codes::TEMPLATE_SUPERSEDED).unwrap();
    assert_eq!(entry.severity, Severity::Warn);
    assert_eq!(entry.sheet_id.as_deref(), Some("S-001"));
    assert!(entry.message.contains("v1") && entry.message.contains("v2"));
    assert_eq!(plan.sheets[0].status, SheetStatus::NeedsPreview);
}

#[test]
fn split_publish_reports_renamed_files_as_info() {
    use cadcraft_engine::cmd::file::{IoHooks, set_io};

    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, options| cadcraft_io::plotter::plot(d, space, options).map_err(|error| error.to_string())),
    });
    let (mut s, _) = setup(0);
    let handles = ["KT/01", "KT/01", "CON"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| place(&mut s, "FRAME", [index as f64 * 500.0, 0.0], 1.0, 0.0, false, &[("SHEET_ID", id)]).hex())
        .collect::<Vec<_>>();
    scan(&mut s, &handles);
    let approvals: Vec<_> = s
        .ubplot
        .last
        .as_ref()
        .unwrap()
        .plan
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|entry| entry.severity == Severity::Warn)
                .map(|entry| json!({"sheetKey": cadcraft_engine::sheetplan::ids::sheet_key(sheet), "code": entry.code}))
        })
        .collect();
    s.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap();
    for _ in 0..2 {
        let result = s.execute("ubplot.publish", &json!({"output": "split"})).unwrap();
        let plan = typed(&result["plan"]);
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            files.iter().map(|file| file["name"].as_str().unwrap()).collect::<Vec<_>>(),
            ["Drawing-KT_01-1.pdf", "Drawing-KT_01-2.pdf", "Drawing-CON-3.pdf"]
        );
        for (sheet, file) in plan.sheets.iter().zip(files) {
            let entries: Vec<_> = sheet.issues.iter().filter(|entry| entry.code == codes::FILE_RENAMED).collect();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].severity, Severity::Info);
            assert_eq!(entries[0].sheet_id.as_deref(), Some(sheet.sheet_id.as_str()));
            assert!(entries[0].message.contains(file["name"].as_str().unwrap()));
            assert_eq!(sheet.status, SheetStatus::Done);
        }
    }
    let result = s.execute("ubplot.publish", &json!({"output": "merged"})).unwrap();
    let plan = typed(&result["plan"]);
    assert!(plan.sheets.iter().all(|sheet| !sheet.issues.iter().any(|entry| entry.code == codes::FILE_RENAMED)));
}

#[test]
fn metadata_preflight_is_idempotent_and_removes_resolved_issues() {
    use cadcraft_engine::sheetplan::preflight::FontStatus;

    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let (mut s, handles) = setup(2);
    let mut plan = typed(&scan(&mut s, &handles));
    for sheet in &mut plan.sheets {
        sheet.sheet_id = "KT-01".into();
        sheet.attributes.insert("REV".into(), "B".into());
    }
    let mut registry = FrameRegistry::builtin();
    registry.templates[0].superseded_by = Some("v2".into());
    let revisions = BTreeMap::from([("KT-01".into(), "C".into())]);
    let fonts = [FontStatus {
        style: "Missing".into(),
        font: "missing.ttf".into(),
        state: cadcraft_fonts::FontState::Missing,
        with: Some("Alternate".into()),
        mapped: false,
    }];
    let ctx = PreflightContext { registry: &registry, device: None, device_missing: false, revisions: &revisions, fonts: &fonts, web: false };
    preflight(s.doc().unwrap(), &mut plan, &ctx);
    let once = plan.clone();
    preflight(s.doc().unwrap(), &mut plan, &ctx);
    assert_eq!(plan, once);
    let fallback = plan.issues.iter().find(|entry| entry.code == codes::FONT_FALLBACK).unwrap();
    assert_eq!(fallback.severity, Severity::Warn);
    assert_eq!(fallback.sheet_id, None);
    assert_eq!(fallback.message, "missing.ttf is drawn with Alternate");
    let metadata_codes = [codes::SHEET_ID_DUPLICATE, codes::REVISION_OLD, codes::TEMPLATE_SUPERSEDED];
    assert!(plan.sheets.iter().all(|sheet| metadata_codes.iter().all(|code| sheet.issues.iter().any(|entry| entry.code == *code))));
    plan.sheets[1].sheet_id = "KT-02".into();
    for sheet in &mut plan.sheets {
        sheet.attributes.insert("REV".into(), "C".into());
    }
    check(&s, &mut plan, false);
    assert!(plan.sheets.iter().all(|sheet| !sheet.issues.iter().any(|entry| metadata_codes.contains(&entry.code.as_str()))));
    assert!(!plan.issues.iter().any(|entry| entry.code.starts_with("FONT_")));
    assert_eq!(plan.issues.iter().filter(|entry| entry.code == codes::PLOT_STYLE).count(), 1);
}

#[test]
fn plot_style_reports_selected_tables_and_display_colors_at_plan_level() {
    let (mut s, _) = setup(0);
    let mut layout = Layout::new("Styled", 1);
    layout.page.plot_style_table = "monochrome.ctb".into();
    s.doc_mut().unwrap().layouts.push(layout);
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Styled", "Layout1"]})).unwrap());
    let entries: Vec<_> = plan.issues.iter().filter(|entry| entry.code == codes::PLOT_STYLE).collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.severity == Severity::Info && entry.sheet_id.is_none()));
    assert!(entries.iter().any(|entry| entry.message == "monochrome.ctb"));
    assert!(entries.iter().any(|entry| entry.message == "colors as displayed"));
}

#[test]
fn revision_uses_template_attribute_tags() {
    let (mut s, _) = setup(0);
    let template = json!({
        "schema_version": "1.0",
        "templates": [{
            "id": "custom-rev", "revision": "1", "block_names": ["FRAME"],
            "geometry": [{"kind": "largest_closed_rect"}],
            "attributes": {"sheet_id": ["SHEET_ID"], "revision": ["ISSUE_REV"]}
        }]
    });
    s.execute("ubplot.templates", &json!({"action": "import", "text": template.to_string()})).unwrap();
    let handle = place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[("SHEET_ID", "KT-01"), ("ISSUE_REV", "B"), ("REV", "D")]);
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()], "revisions": {"KT-01": "C"}})).unwrap());
    assert!(plan.sheets[0].issues.iter().any(|entry| entry.code == codes::REVISION_OLD));
    assert!(!plan.sheets[0].issues.iter().any(|entry| entry.code == codes::REVISION_NEWER));
    let request: cadcraft_engine::sheetplan::ScanRequest = serde_json::from_value(json!({"mode": "block"})).unwrap();
    assert!(request.revisions.is_empty());
    assert_eq!(s.ubplot.registry.templates.iter().find(|template| template.id == "custom-rev").unwrap().superseded_by, None);
}

#[test]
fn font_usage_follows_nested_transforms_and_layout_viewports() {
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings = FontSettings::isolated();
    let (mut s, _) = setup(0);
    missing_style(&mut s);
    nest(s.doc_mut().unwrap(), "OUTER", "FRAME", [20.0, 30.0], 0.0);
    let handle = place(&mut s, "OUTER", [1000.0, 2000.0], 2.0, 30.0, false, &[]);
    let plan = typed(&scan(&mut s, &[handle.hex()]));
    assert!(plan.issues.iter().any(|entry| entry.code == missing_font_issue().0));
    let mut layout = Layout::new("Seen", 1);
    layout.entities.push(cadcraft_doc::Entity::new(
        s.doc_mut().unwrap().new_handle(),
        EntityKind::Viewport(Viewport {
            center: Vec2::new(100.0, 100.0).to3(0.0),
            width: 200.0,
            height: 100.0,
            view_center: Vec2::new(1000.0, 2000.0),
            view_height: 2000.0,
            ..Default::default()
        }),
    ));
    s.doc_mut().unwrap().layouts.push(layout);
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Seen"]})).unwrap());
    assert!(plan.issues.iter().any(|entry| entry.code == missing_font_issue().0));
    let plan = typed(&s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Layout1"]})).unwrap());
    assert!(!plan.issues.iter().any(|entry| entry.code.starts_with("FONT_")));
}
