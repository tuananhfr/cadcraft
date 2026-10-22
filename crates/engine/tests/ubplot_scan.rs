mod support;

use cadcraft_doc::{Common, EntityKind, Handle, Layout, Line, Lineweight, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::base64_decode;
use cadcraft_engine::sheetplan::{PlotScale, ScanMode, ScanRequest, Severity, SheetMode, SheetPlan, SheetSource, SheetStatus, codes, ids};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};
use support::frames::{frame_block, place};

#[test]
fn preview_outline_renders_the_rotated_frame() {
    let mut s = Session::new();
    let center = Vec2::new(512345.678, 2109876.543);
    let rotation = std::f64::consts::PI / 6.0;
    let world = |p: Vec2| center + p.rotate(rotation);
    let outline = [Vec2::new(-210.0, -148.5), Vec2::new(210.0, -148.5), Vec2::new(210.0, 148.5), Vec2::new(-210.0, 148.5)].map(world);
    // A two-pixel border sits three pixels inside the crop so corner samples are unambiguous.
    let inset = 3.5 * 420.0 / 400.0;
    let border = [
        Vec2::new(-210.0 + inset, -148.5 + inset),
        Vec2::new(210.0 - inset, -148.5 + inset),
        Vec2::new(210.0 - inset, 148.5 - inset),
        Vec2::new(-210.0 + inset, 148.5 - inset),
    ]
    .map(world);
    let d = s.doc_mut().unwrap();
    d.header.set("MEASUREMENT", cadcraft_doc::HVal::Int(1));
    for (a, b) in border.iter().zip(border.iter().cycle().skip(1)) {
        let common = Common { lineweight: Lineweight::Mm100(210), ..Default::default() };
        d.add(&Space::Model, common, EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })).unwrap();
    }
    d.add(
        &Space::Model,
        Common::default(),
        EntityKind::Line(Line { a: world(Vec2::new(5000.0, 0.0)).to3(0.0), b: world(Vec2::new(6000.0, 0.0)).to3(0.0) }),
    )
    .unwrap();
    let outline = json!(outline.map(|p| [p.x, p.y]));
    let result = s.execute("preview", &json!({"outline": outline, "width": 400})).unwrap();
    assert_eq!(result["width"], 400);
    let png = base64_decode(result["data"].as_str().unwrap()).unwrap();
    let image = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!((image.width(), image.height()), (400, 283));
    for (x, y) in [(3, 3), (396, 3), (396, 279), (3, 279)] {
        let pixel = image.pixel(x, y).unwrap();
        assert!(pixel.red() < 250 && pixel.green() < 250 && pixel.blue() < 250, "white corner ({x}, {y})");
    }
    let middle = image.pixel(200, 141).unwrap();
    assert_eq!((middle.red(), middle.green(), middle.blue()), (255, 255, 255));
    for outline in [json!([[0, 0], [420, 0], [400, 297], [0, 297]]), json!([[0, 0], [1, 0], [1, 1]]), json!([[null, 0], [1, 0], [1, 1], [0, 1]])] {
        assert!(s.execute("preview", &json!({"outline": outline, "width": 400})).is_err());
    }
}

fn scan_setup(count: usize) -> (Session, Vec<String>) {
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "FRAME-A3", 420.0, 297.0);
    let handles = (0..count)
        .map(|index| place(&mut s, "FRAME-A3", [(index % 10) as f64 * 500.0, (index / 10) as f64 * 400.0], 1.0, 0.0, false, &[]).hex())
        .collect();
    (s, handles)
}

fn scan(s: &mut Session, params: Value) -> Value {
    s.execute("ubplot.scan", &params).unwrap()
}

fn typed(result: &Value) -> SheetPlan {
    serde_json::from_value(result.clone()).unwrap()
}

#[test]
fn scan_block_mode_ten_of_hundred_in_pick_order() {
    let (mut s, handles) = scan_setup(100);
    let picked: Vec<_> = [37, 2, 98, 11, 56, 43, 70, 5, 26, 89].map(|index| handles[index].clone()).into();
    let result = scan(&mut s, json!({"mode": "block", "handles": picked}));
    let plan = typed(&result);
    assert_eq!(plan.sheets.len(), 10);
    assert_eq!(result["summary"]["sheets"], 10);
    assert_eq!(result["summary"]["blocked"], 0);
    for (index, (sheet, handle)) in plan.sheets.iter().zip(&picked).enumerate() {
        assert_eq!(sheet.order, (index + 1) as u32);
        assert!(matches!(&sheet.source, SheetSource::Block { block_handle, .. } if block_handle == handle));
        assert_eq!(sheet.paper_mm, [297.0, 420.0]);
        assert!(sheet.landscape);
        assert_eq!(sheet.plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 1.0 });
        assert_eq!(result["sheets"][index]["plot_scale"], "1:1");
        assert_eq!(sheet.status, SheetStatus::NeedsPreview);
    }
}

#[test]
fn scan_block_mode_reads_sheet_id_attribute() {
    let (mut s, _) = scan_setup(0);
    let first = place(&mut s, "FRAME-A3", [0.0, 0.0], 1.0, 0.0, false, &[("sheet_no", "KT-01"), ("REV", "B")]);
    let second = place(&mut s, "FRAME-A3", [500.0, 0.0], 1.0, 0.0, false, &[]);
    let plan = typed(&scan(&mut s, json!({"mode": "block", "handles": [first.hex(), second.hex()]})));
    assert_eq!(plan.sheets[0].sheet_id, "KT-01");
    assert_eq!(plan.sheets[0].attributes.get("SHEET_NO").map(String::as_str), Some("KT-01"));
    assert_eq!(plan.sheets[0].attributes.get("REV").map(String::as_str), Some("B"));
    assert_eq!(plan.sheets[1].sheet_id, "S-002");
}

#[test]
fn scan_block_mode_in_layout_space() {
    let (mut s, handles) = scan_setup(1);
    let handle = Handle(u64::from_str_radix(&handles[0], 16).unwrap());
    let d = s.doc_mut().unwrap();
    if d.layout("Layout1").is_none() {
        d.layouts.push(Layout::new("Layout1", 1));
    }
    let entity = d.remove_entity(handle).unwrap();
    let paper = Space::Paper("Layout1".into());
    d.space_mut(&paper).unwrap().push((*entity).clone());
    // A model-space decoy must not change which INSERT the saved sheet resolves to.
    place(&mut s, "FRAME-A3", [10000.0, 0.0], 1.0, 0.0, false, &[]);
    let plan = typed(&scan(&mut s, json!({"mode": "block", "handles": handles})));
    assert_eq!(plan.sheets.len(), 1);
    let SheetSource::Block { block_handle, outline, .. } = &plan.sheets[0].source else { panic!("block source required") };
    assert_eq!(block_handle, &handle.hex());
    assert_eq!(s.doc().unwrap().space_of(handle), Some(paper));
    assert_eq!(*outline, [[0.0, 0.0], [420.0, 0.0], [420.0, 297.0], [0.0, 297.0]]);
    assert_eq!(plan.sheets[0].status, SheetStatus::NeedsPreview);
}

#[test]
fn scan_window_mode_order_and_no_scale_inference() {
    let (mut s, _) = scan_setup(0);
    let windows = json!([[[420, 297], [0, 0]], [[500, 0], [920, 0], [920, 297], [500, 297]], [[1000, 0], [1100, 200]]]);
    let plan = typed(&scan(&mut s, json!({"mode": "window", "windows": windows})));
    assert_eq!(plan.sheets.len(), 3);
    for (index, sheet) in plan.sheets.iter().enumerate() {
        assert_eq!(sheet.order, (index + 1) as u32);
        assert_eq!(sheet.plot_scale, PlotScale::Fit);
        assert_eq!(sheet.paper_mm, [297.0, 420.0]);
        assert!(sheet.issues.iter().any(|i| i.code == codes::SCALE_NOT_SET && i.severity == Severity::Info));
    }
    assert!(plan.sheets[0].landscape);
    assert!(!plan.sheets[2].landscape);
    assert!(
        matches!(plan.sheets[0].source, SheetSource::Window { outline, .. } if outline == [[0.0, 0.0], [420.0, 0.0], [420.0, 297.0], [0.0, 297.0]])
    );
    let plan = typed(&scan(&mut s, json!({"mode": "window", "windows": windows, "scale": "1:50"})));
    for sheet in plan.sheets {
        assert_eq!(sheet.plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 });
        assert!(!sheet.issues.iter().any(|i| i.code == codes::SCALE_NOT_SET));
    }
}

#[test]
fn scan_layout_mode_uses_page_setups() {
    let (mut s, _) = scan_setup(0);
    let mut first = Layout::new("Second", 20);
    first.page.width_mm = 594.0;
    first.page.height_mm = 841.0;
    first.page.landscape = false;
    first.page.scale_to_fit = false;
    first.page.scale = 0.02;
    let mut second = Layout::new("First", 10);
    second.page.width_mm = 210.0;
    second.page.height_mm = 297.0;
    second.page.landscape = true;
    second.page.scale_to_fit = true;
    s.doc_mut().unwrap().layouts = vec![first, second];
    let plan = typed(&scan(&mut s, json!({"mode": "layout", "layouts": ["*"]})));
    assert_eq!(plan.sheets.len(), 2);
    assert_eq!(plan.sheets[0].source, SheetSource::Layout { name: "First".into() });
    assert_eq!(plan.sheets[0].order, 1);
    assert_eq!(plan.sheets[0].paper_mm, [210.0, 297.0]);
    assert!(plan.sheets[0].landscape);
    assert_eq!(plan.sheets[0].plot_scale, PlotScale::Fit);
    assert_eq!(plan.sheets[1].source, SheetSource::Layout { name: "Second".into() });
    assert_eq!(plan.sheets[1].order, 2);
    assert_eq!(plan.sheets[1].paper_mm, [594.0, 841.0]);
    assert!(!plan.sheets[1].landscape);
    assert_eq!(plan.sheets[1].plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 });
    let named = typed(&scan(&mut s, json!({"mode": "layout", "layouts": ["Second", "First"]})));
    assert_eq!(named.sheets[0].source, plan.sheets[1].source);
}

#[test]
fn scan_last_reuses_and_rescans_when_changed() {
    let (mut s, handles) = scan_setup(1);
    assert!(s.execute("ubplot.scan", &json!({"mode": "last"})).is_err());
    let first = scan(&mut s, json!({"mode": "block", "handles": handles}));
    let again = scan(&mut s, json!({"mode": "last"}));
    assert_eq!(again, first);
    s.execute("move", &json!({"handles": handles, "delta": [10, 20]})).unwrap();
    let moved = scan(&mut s, json!({"mode": "last"}));
    assert_eq!(moved["rescanned"], true);
    assert_ne!(moved["jobId"], first["jobId"]);
    let sheet = &typed(&moved).sheets[0];
    assert!(matches!(sheet.source, SheetSource::Block { outline, .. } if outline == [[10.0, 20.0], [430.0, 20.0], [430.0, 317.0], [10.0, 317.0]]));
    assert_eq!(s.ubplot.last.as_ref().unwrap().request.mode, ScanMode::Block);
    s.new_drawing(false);
    assert!(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap_err().to_string().contains("no previous UBPLOT plan for this drawing"));
}

#[test]
fn scan_job_id_stable_and_changes_with_settings() {
    let (mut s, handles) = scan_setup(1);
    let params = json!({"mode": "block", "handles": handles});
    let first = scan(&mut s, params.clone());
    let again = scan(&mut s, params.clone());
    assert_eq!(again["jobId"], first["jobId"]);
    assert_eq!(first["jobId"], first["job_id"]);
    let req: ScanRequest = serde_json::from_value(params).unwrap();
    let request = serde_json::to_value(&req).unwrap();
    let source = first["source_hash"].as_str().unwrap();
    assert_eq!(first["jobId"], ids::job_id(source, &request));
    let changed = scan(&mut s, json!({"mode": "block", "handles": handles, "paper": "A4"}));
    assert_ne!(changed["jobId"], first["jobId"]);
    assert_eq!(changed["source_hash"], first["source_hash"]);
    assert_eq!(typed(&changed).sheets[0].paper_mm, [210.0, 297.0]);
}

#[test]
fn scan_overrides_and_exclude_are_recorded() {
    let (mut s, handles) = scan_setup(10);
    let plan = typed(&scan(
        &mut s,
        json!({
            "mode": "block", "handles": handles, "exclude": ["S-002@2"], "paper": "A4",
            "overrides": {"S-003@3": {"window": [[100, 200], [520, 497]], "paper": "A3", "scale": "1:50", "sheetId": "KT-03"}}
        }),
    ));
    assert_eq!(plan.sheets.len(), 9);
    assert!(!plan.sheets.iter().any(|sheet| sheet.order == 2));
    assert!(plan.issues.iter().any(|i| i.code == codes::SHEET_EXCLUDED && i.severity == Severity::Info && i.sheet_id.as_deref() == Some("S-002")));
    let changed = plan.sheets.iter().find(|sheet| sheet.order == 3).unwrap();
    assert_eq!(changed.mode, SheetMode::Window);
    assert_eq!(changed.sheet_id, "KT-03");
    assert_eq!(changed.paper_mm, [297.0, 420.0]);
    assert_eq!(changed.plot_scale, PlotScale::Ratio { paper_mm: 1.0, drawing_units: 50.0 });
    assert!(
        matches!(changed.source, SheetSource::Window { outline, .. } if outline == [[100.0, 200.0], [520.0, 200.0], [520.0, 497.0], [100.0, 497.0]])
    );
    let last = s.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.exclude, ["S-002@2"]);
    assert_eq!(last.request.overrides["S-003@3"].sheet_id.as_deref(), Some("KT-03"));
    assert_eq!(typed(&scan(&mut s, json!({"mode": "last"}))), plan);
}

#[test]
fn scan_duplicate_handles_print_once() {
    let (mut s, handles) = scan_setup(2);
    let duplicate = format!("00{}", handles[0].to_ascii_lowercase());
    let result = scan(&mut s, json!({"mode": "block", "handles": [handles[0], duplicate, handles[1]]}));
    let plan = typed(&result);
    assert_eq!(plan.sheets.len(), 2);
    assert_eq!(plan.sheets[0].order, 1);
    assert_eq!(plan.sheets[1].order, 2);
    assert_eq!(plan.issues.iter().filter(|i| i.code == codes::HANDLE_DUPLICATE && i.severity == Severity::Info).count(), 1);
    assert_eq!(result["summary"]["info"], 2);
}

#[test]
fn scan_plan_mode_resolves_appendix_short_form() {
    let (mut s, handles) = scan_setup(1);
    let appendix = json!({
        "schema_version": "1.0", "job_id": "appendix",
        "sheets": [{"sheet_id": "KT-01", "order": 1, "mode": "block", "block_handle": handles[0],
            "paper_mm": [297, 420], "plot_scale": "1:1", "status": "READY"}]
    });
    let plan = typed(&scan(&mut s, json!({"mode": "plan", "plan": appendix})));
    assert_eq!(plan.sheets[0].sheet_id, "KT-01");
    assert_eq!(plan.sheets[0].status, SheetStatus::NeedsPreview);
    assert!(matches!(&plan.sheets[0].source, SheetSource::Block { block_handle, outline, .. }
        if block_handle == &handles[0] && *outline == [[0.0, 0.0], [420.0, 0.0], [420.0, 297.0], [0.0, 297.0]]));
}

#[test]
fn scan_rejects_hostile_params() {
    let (mut s, handles) = scan_setup(1);
    let invalid = [
        json!({"mode": "block", "handles": vec![handles[0].clone(); 10_001]}),
        json!({"mode": "window", "windows": [[[f64::NAN, 0], [1, 1]]]}),
        json!({"mode": "layout", "layouts": ["missing"]}),
        json!({"mode": "unknown"}),
        json!({"mode": "window", "windows": [[[0, 0], [1, 1]]], "scale": "0:1"}),
        json!({"mode": "plan", "plan": []}),
        json!({"mode": "block", "handles": ["not-hex"]}),
        json!({"mode": "window", "windows": [[[0, 0], [1, 0], [2, 1], [0, 1]]]}),
        json!({"mode": "block", "handles": handles, "overrides": {"S-001@1": {"scale": "1:0"}}}),
    ];
    for params in invalid {
        assert!(s.execute("ubplot.scan", &params).is_err());
    }
    let result = scan(&mut s, json!({"mode": "block", "handles": ["FFFFFFFFFFFFFFFE"]}));
    assert_eq!(result["summary"]["blocked"], 1);
    let plan = typed(&result);
    assert!(plan.sheets[0].issues.iter().any(|i| i.code == codes::FRAME_NOT_FOUND && i.severity == Severity::Block));
    assert_eq!(plan.sheets[0].status, SheetStatus::Blocked);
}

#[test]
fn templates_import_list_reset() {
    let (mut s, handles) = scan_setup(1);
    let builtin = s.execute("ubplot.templates", &json!({"action": "list"})).unwrap();
    assert_eq!(builtin["templates"].as_array().unwrap().len(), 1);
    assert_eq!(builtin["templates"][0]["id"], "generic");
    assert_eq!(builtin["templates"][0]["blockNames"], json!(["*"]));
    let imported = json!({
        "schema_version": "1.0", "templates": [{"id": "synthetic", "revision": "2", "block_names": ["FRAME-A3"],
            "geometry": [{"kind": "largest_closed_rect"}], "attributes": {"sheet_id": ["CUSTOM_NO"]}}]
    });
    let merged = s.execute("ubplot.templates", &json!({"action": "import", "text": imported.to_string()})).unwrap();
    assert_eq!(merged["templates"][0]["id"], "synthetic");
    assert_eq!(merged["templates"][0]["revision"], "2");
    assert_eq!(merged["templates"][0]["blockNames"], json!(["FRAME-A3"]));
    assert_eq!(merged["templates"][1]["id"], "generic");
    let plan = typed(&scan(&mut s, json!({"mode": "block", "handles": handles})));
    assert_eq!(plan.sheets[0].frame_template_id.as_deref(), Some("synthetic"));
    assert!(s.execute("ubplot.templates", &json!({"action": "import", "text": "broken"})).is_err());
    assert_eq!(s.execute("ubplot.templates", &json!({"action": "list"})).unwrap(), merged);
    assert!(s.execute("ubplot.templates", &json!({"action": "other"})).is_err());
    assert_eq!(s.execute("ubplot.templates", &json!({"action": "reset"})).unwrap(), builtin);
}

#[test]
fn scan_is_programmatic_and_sets_no_review_flag() {
    let (mut s, handles) = scan_setup(1);
    let state = s.state().unwrap();
    let (doc, revision, undo_len) = (state.doc.clone(), state.revision, state.undo.len());
    let expected_hash = ids::sha256_hex(format!("{}:{revision}", state.uid).as_bytes());
    s.ubplot.review = true;
    let result = scan(&mut s, json!({"mode": "block", "handles": handles}));
    assert_eq!(result["source_hash"], expected_hash);
    assert!(typed(&result).issues.iter().any(|i| i.code == codes::SOURCE_HASH_WEAK && i.severity == Severity::Info));
    assert!(!s.ubplot.review);
    assert!(s.ubplot.last.is_some());
    assert!(s.running.is_none());
    let state = s.state().unwrap();
    assert!(std::sync::Arc::ptr_eq(&state.doc, &doc));
    assert_eq!(state.revision, revision);
    assert_eq!(state.undo.len(), undo_len);
    for id in ["ubplot.scan", "ubplot.templates"] {
        assert!(!cadcraft_engine::find_command(id).unwrap().undoable);
        assert!(Session::empty().execute(id, &json!({"mode": "block"})).is_err());
    }
}

#[test]
fn scan_block_paper_scale_and_issue_rules() {
    let (mut s, _) = scan_setup(0);
    let d = s.doc_mut().unwrap();
    support::frames::anon_dynamic(d, "FRAME-A3", "*U7");
    support::frames::nest(d, "NESTED", "FRAME-A3", [5.0, 5.0], 30.0);
    support::frames::title_only_block(d, "NO-FRAME");
    frame_block(d, "CUSTOM", 123.4, 456.7);
    frame_block(d, "TOO-LARGE", 6000.0, 200.0);
    let nested = place(&mut s, "NESTED", [0.0, 0.0], 50.0, 0.0, false, &[]);
    let dynamic = place(&mut s, "*U7", [30000.0, 0.0], 1.0, 0.0, false, &[]);
    let custom = place(&mut s, "CUSTOM", [31000.0, 0.0], 1.0, 0.0, false, &[]);
    let large = place(&mut s, "TOO-LARGE", [32000.0, 0.0], 1.0, 0.0, false, &[]);
    let nonuniform = place(&mut s, "FRAME-A3", [40000.0, 0.0], 1.0, 0.0, false, &[]);
    s.doc_mut()
        .unwrap()
        .modify_entity(nonuniform, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                insert.scale.y = 2.0;
            }
        })
        .unwrap();
    let missing = place(&mut s, "NO-FRAME", [50000.0, 0.0], 1.0, 0.0, false, &[]);
    let handles = [nested, dynamic, custom, large, nonuniform, missing].map(Handle::hex);
    let result = scan(&mut s, json!({"mode": "block", "handles": handles}));
    let plan = typed(&result);
    assert_eq!(plan.sheets.len(), 6);
    assert_eq!(plan.sheets[0].paper_mm, [297.0, 420.0]);
    let PlotScale::Ratio { paper_mm, drawing_units } = plan.sheets[0].plot_scale else { panic!("ratio required") };
    assert_eq!(paper_mm, 1.0);
    assert!((drawing_units - 50.0).abs() < 1e-9);
    assert!(matches!(&plan.sheets[1].source, SheetSource::Block { effective_name, .. } if effective_name == "FRAME-A3"));
    assert_eq!(plan.sheets[2].paper_mm, [123.4, 456.7]);
    assert!(plan.sheets[2].issues.iter().any(|i| i.code == codes::PAPER_CUSTOM && i.severity == Severity::Info));
    assert_eq!(plan.sheets[3].status, SheetStatus::Blocked);
    assert!(plan.sheets[3].issues.iter().any(|i| i.code == codes::PAPER_TOO_LARGE && i.severity == Severity::Block));
    assert_eq!(plan.sheets[4].plot_scale, PlotScale::Fit);
    assert!(plan.sheets[4].issues.iter().any(|i| i.code == codes::FRAME_NONUNIFORM && i.severity == Severity::Warn));
    assert_eq!(plan.sheets[5].status, SheetStatus::Blocked);
    assert_eq!(result["summary"]["blocked"], 2);
    let repaired = scan(
        &mut s,
        json!({"mode": "block", "handles": [missing.hex()],
            "overrides": {"S-001@1": {"window": [[0, 0], [420, 297]], "paper": "A3"}}}),
    );
    assert_eq!(typed(&repaired).sheets[0].mode, SheetMode::Window);
    assert_eq!(repaired["summary"]["blocked"], 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn scan_source_hash_saved_bytes_and_revision_cache() {
    struct SyntheticFile(std::path::PathBuf);
    impl Drop for SyntheticFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut s = Session::new();
    let uid = s.state().unwrap().uid;
    let file = SyntheticFile(std::env::temp_dir().join(format!("cadcraft-ubplot-hash-{}-{uid}.dxf", std::process::id())));
    std::fs::write(&file.0, b"synthetic saved drawing").unwrap();
    s.state_mut().unwrap().path = Some(file.0.to_str().unwrap().into());
    let request = json!({"mode": "window", "windows": [[[0, 0], [420, 297]]]});
    let first = scan(&mut s, request.clone());
    assert_eq!(first["source_hash"], ids::sha256_hex(b"synthetic saved drawing"));
    assert!(!typed(&first).issues.iter().any(|i| i.code == codes::SOURCE_HASH_WEAK));
    std::fs::write(&file.0, b"changed saved drawing").unwrap();
    assert_eq!(scan(&mut s, request.clone())["source_hash"], first["source_hash"]);
    s.touch();
    let changed = scan(&mut s, request.clone());
    assert_eq!(changed["source_hash"], ids::sha256_hex(b"changed saved drawing"));
    s.execute("line", &json!({"points": [[0, 0], [1, 1]]})).unwrap();
    let dirty = scan(&mut s, request);
    assert_eq!(dirty["source_hash"], ids::sha256_hex(format!("{uid}:{}", s.state().unwrap().revision).as_bytes()));
    assert!(typed(&dirty).issues.iter().any(|i| i.code == codes::SOURCE_HASH_WEAK));
}
