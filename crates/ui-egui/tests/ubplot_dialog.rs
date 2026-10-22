use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cadcraft_doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Insert, Line, LwPolyline, Space, XrefStatus};
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::{Severity, SheetMode, SheetSource, SheetStatus, approval::fingerprint, codes, derive_status, ids::sheet_key};
use cadcraft_engine::{Input, Session};
use cadcraft_geom::{PolyVertex, Vec2, Vec3};
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn frame_block(d: &mut Drawing, name: &str, ambiguous: bool) {
    let mut block = Block::new(name);
    let (width, height) = if ambiguous { (180.0, 60.0) } else { (420.0, 297.0) };
    let corners = [Vec2::ZERO, Vec2::new(width, 0.0), Vec2::new(width, height), Vec2::new(0.0, height)];
    if ambiguous {
        for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)).take(4) {
            block.entities.push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })));
        }
    } else {
        block.entities.push(Entity::new(
            d.new_handle(),
            EntityKind::LwPolyline(LwPolyline {
                vertices: corners.map(PolyVertex::new).into(),
                closed: true,
                const_width: 0.0,
                elevation: 0.0,
                plinegen: false,
            }),
        ));
    }
    block
        .entities
        .push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: Vec3::new(0.0, height / 2.0, 0.0), b: Vec3::new(width, height / 2.0, 0.0) })));
    d.blocks.insert(name.into(), Arc::new(block));
}

fn place(s: &mut Session, name: &str, x: f64) -> Handle {
    s.doc_mut()
        .unwrap()
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: name.into(),
                insert: Vec3::new(x, 0.0, 0.0),
                scale: Vec3::new(1.0, 1.0, 1.0),
                rotation: 0.0,
                attribs: Vec::new(),
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap()
}

fn app(ambiguous: &[bool]) -> (CadApp, Vec<Handle>) {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
        plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
    });
    let mut session = Session::empty();
    session.open_drawing(Drawing::new_metric(), "Quick Plot fixture", None);
    frame_block(session.doc_mut().unwrap(), "FRAME-A3", false);
    frame_block(session.doc_mut().unwrap(), "FRAME-TITLE", true);
    let handles = ambiguous
        .iter()
        .enumerate()
        .map(|(index, ambiguous)| place(&mut session, if *ambiguous { "FRAME-TITLE" } else { "FRAME-A3" }, index as f64 * 500.0))
        .collect();
    session.settings.osmode = 0;
    session.settings.snapmode = false;
    session.settings.orthomode = false;
    session.settings.dynmode = false;
    session.set_selection(Vec::new());
    let mut app = CadApp::new(session, Services::default());
    app.skip_first_run_picker = true;
    app.ui.start_tab = false;
    (app, handles)
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    harness_size(app, vec2(1366.0, 768.0))
}

fn harness_size(app: CadApp, size: egui::Vec2) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn scan(app: &mut CadApp, handles: &[Handle]) {
    app.run("ubplot.scan", json!({"mode": "block", "handles": handles.iter().map(|handle| handle.hex()).collect::<Vec<_>>()})).unwrap();
}

fn review(app: &mut CadApp, handles: &[Handle]) {
    scan(app, handles);
    app.session.ubplot.review = true;
}

fn temporary_pdf() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("cadcraft-ubplot-dialog-{}-{}.pdf", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn probe_page_count(path: &Path) -> usize {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let text = String::from_utf8_lossy(&bytes);
    let count = text.matches("/Type /Page ").count();
    assert!(text.contains(&format!("/Count {count} >>")), "the page tree must agree with its leaf count");
    count
}

fn mark_circles(h: &Harness<'_, CadApp>) -> usize {
    let tokens = cadcraft_ui_egui::theme::Tokens::get();
    h.output()
        .shapes
        .iter()
        .filter(|shape| match &shape.shape {
            egui::Shape::Circle(circle) => circle.radius == 9.0 && circle.stroke.width == 2.0 && circle.stroke.color == tokens.accent,
            _ => false,
        })
        .count()
}

#[test]
fn dialog_opens_after_interactive_finish() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false]);
    app.start("ubplot");
    app.session.input(Input::Enter).unwrap();
    app.session.input(Input::Pick(frames)).unwrap();
    app.session.input(Input::Enter).unwrap();
    let mut h = harness(app);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("ubplot"));
    assert!(!h.state().session.ubplot.review);
    assert!(h.query_by_label("2 sheets").is_some());
}

#[test]
fn json_scan_never_opens_dialog() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false]);
    scan(&mut app, &frames);
    let h = harness(app);
    assert!(h.state().ui.dialog.is_none());
    assert!(h.state().session.running.is_none());
    assert!(h.query_by_label("Quick Plot").is_none());
}

#[test]
fn exceptions_list_shows_only_warn_and_block() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false, false, true]);
    review(&mut app, &frames);
    let sheet = &mut app.session.ubplot.last.as_mut().unwrap().plan.sheets[3];
    sheet.issues.push(codes::issue(Severity::Warn, codes::FRAME_OVERLAP, "Synthetic overlapping frame.", Some(&sheet.sheet_id)));
    let mut h = harness(app);
    assert!(h.query_by_label("FRAME_OVERLAP · 1 sheets · WARN").is_some());
    assert!(h.query_by_label("FRAME_AMBIGUOUS · 1 sheets · BLOCK").is_some());
    for order in 1..=3 {
        assert!(h.query_by_label(&format!("S-{order:03} · {order}")).is_none());
    }
    assert!(h.query_by_label("S-004 · 4").is_some());
    assert!(h.query_by_label("S-005 · 5").is_some());
    assert!(h.query_by_label("Approve").is_none());
    h.get_by_label("Show all sheets").click();
    h.run_steps(3);
    for order in 1..=5 {
        assert!(h.query_by_label(&format!("S-{order:03} · {order}")).is_some());
    }
}

#[test]
fn bulk_exclude_same_cause() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[true, true, true]);
    review(&mut app, &frames);
    let mut h = harness(app);
    h.get_by_label("Exclude all").click();
    h.run_steps(3);
    let last = h.state().session.ubplot.last.as_ref().unwrap();
    assert!(last.plan.sheets.is_empty());
    assert_eq!(last.request.exclude.len(), 3);
    assert_eq!(last.plan.issues.iter().filter(|issue| issue.code == codes::SHEET_EXCLUDED).count(), 3);
    h.get_by_label("Rescan").click();
    h.run_steps(3);
    assert!(h.state().session.ubplot.last.as_ref().unwrap().plan.sheets.is_empty());
}

#[test]
fn publish_button_writes_file_via_pick_save() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false]);
    review(&mut app, &frames);
    approve_warnings(&mut app);
    let path = temporary_pdf();
    let picked = path.clone();
    let suggestions = Rc::new(RefCell::new(Vec::new()));
    let asked = suggestions.clone();
    app.services.pick_save = Some(Box::new(move |name| {
        asked.borrow_mut().push(name.to_string());
        Some(picked.to_string_lossy().into_owned())
    }));
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").click();
    h.run_steps(3);
    assert_eq!(*suggestions.borrow(), ["Quick Plot fixture.pdf"]);
    assert_eq!(probe_page_count(&path), 2);
    assert!(h.state().ui.dialog.is_none());
    assert!(h.state().session.log.iter().any(|line| line == "Published 1 file(s)"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn pick_window_replaces_ambiguous_sheet_in_place() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, true]);
    review(&mut app, &frames);
    let key = sheet_key(&app.session.ubplot.last.as_ref().unwrap().plan.sheets[1]);
    let mut h = harness(app);
    h.get_by_label("Pick window <").click();
    h.run_steps(3);
    assert!(h.state().ui.dialog.is_none());
    assert_eq!(h.state().session.running.as_ref().unwrap().id, "plotwindow");
    h.state_mut().session.input(Input::Point(Vec2::new(500.0, 0.0))).unwrap();
    h.state_mut().session.input(Input::Point(Vec2::new(920.0, 297.0))).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("ubplot"));
    let last = h.state().session.ubplot.last.as_ref().unwrap();
    assert_eq!(last.plan.sheets.len(), 2);
    let sheet = &last.plan.sheets[1];
    assert_eq!(sheet.mode, SheetMode::Window);
    assert_eq!(sheet.order, 2);
    assert_eq!(sheet_key(sheet), key);
    assert_ne!(sheet.status, SheetStatus::Blocked);
    assert!(matches!(sheet.source, SheetSource::Window { outline, .. } if outline == [[500.0, 0.0], [920.0, 0.0], [920.0, 297.0], [500.0, 297.0]]));
    assert_eq!(last.request.overrides.get(&key).unwrap().window, Some([[500.0, 0.0], [920.0, 297.0]]));
}

#[test]
fn shift_click_removes_frame() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false]);
    // The fixture opens at the default view; the frames must be on screen to be clicked.
    app.run("zoom", json!({"mode": "extents"})).unwrap();
    app.start("ubplot");
    app.session.input(Input::Enter).unwrap();
    app.session.input(Input::Pick(frames)).unwrap();
    let mut h = harness(app);
    assert_eq!(h.state().session.marks().len(), 2);
    let pos = h.state().canvas.xf.unwrap().to_screen(Vec2::new(210.0, 148.5));
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT });
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT });
    h.run_steps(3);
    let app = h.state();
    let log = &app.session.log[app.session.log.len().saturating_sub(4)..];
    assert_eq!(
        app.session.marks().len(),
        1,
        "prompt {:?}, pending window {}, cursor {:?} vs {pos:?}, log {log:?}",
        app.session.prompt_text(),
        app.session.pending_window.is_some(),
        app.canvas.cursor
    );
    assert_eq!(h.state().session.marks()[0].label, "1");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().session.marks().len(), 2);
}

#[test]
fn canvas_draws_marks_without_panic_on_resize() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false]);
    app.start("ubplot");
    app.session.input(Input::Enter).unwrap();
    app.session.input(Input::Pick(frames)).unwrap();
    let mut h = harness(app);
    assert_eq!(h.state().session.marks().len(), 3);
    assert_eq!(mark_circles(&h), 3);
    let before = h.state().canvas.rect.unwrap();
    h.set_size(vec2(800.0, 600.0));
    h.run_steps(3);
    assert_eq!(h.state().session.marks().len(), 3);
    assert_eq!(mark_circles(&h), 3);
    assert!(h.state().canvas.rect.unwrap().width() < before.width());
}

#[test]
fn cancelling_save_keeps_dialog_open() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false]);
    review(&mut app, &frames);
    approve_warnings(&mut app);
    app.services.pick_save = Some(Box::new(|_| None));
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("ubplot"));
    assert!(h.state().session.ubplot.last.as_ref().unwrap().plan.sheets.iter().all(|sheet| sheet.status != SheetStatus::Done));
    assert!(!h.state().session.log.iter().any(|line| line.starts_with("Published ")));
}

#[test]
fn cancelling_window_pick_preserves_the_plan() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, true]);
    review(&mut app, &frames);
    let original = app.session.ubplot.last.as_ref().unwrap().plan.clone();
    let mut h = harness(app);
    h.get_by_label("Pick window <").click();
    h.run_steps(3);
    h.state_mut().session.input(Input::Cancel).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("ubplot"));
    assert_eq!(h.state().session.ubplot.last.as_ref().unwrap().plan, original);
    assert!(h.state().session.ubplot.last.as_ref().unwrap().request.overrides.is_empty());
}

#[test]
fn bulk_paper_overrides_survive_rescan() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[true, true]);
    review(&mut app, &frames);
    let mut h = harness(app);
    h.get_by_label("Set paper for all").click();
    h.run_steps(3);
    h.get_by_label("A4").click();
    h.run_steps(3);
    h.get_by_label("Rescan").click();
    h.run_steps(3);
    let last = h.state().session.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.overrides.len(), 2);
    for sheet in &last.plan.sheets {
        assert_eq!(sheet.paper_mm, [210.0, 297.0]);
        assert_eq!(last.request.overrides.get(&sheet_key(sheet)).unwrap().paper.as_deref(), Some("A4"));
    }
}

#[test]
fn split_publish_keeps_failed_sheets_visible() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[true, true]);
    review(&mut app, &frames);
    approve_warnings(&mut app);
    let path = temporary_pdf();
    app.services.pick_save = Some(Box::new(move |_| Some(path.to_string_lossy().into_owned())));
    let mut h = harness(app);
    h.get_by_label("One PDF per sheet").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("ubplot"));
    assert!(h.query_by_label("Failed sheets").is_some());
    assert!(h.query_by_label("S-001@1").is_some());
    assert!(h.query_by_label("S-002@2").is_some());
    assert!(h.state().session.log.iter().any(|line| line == "Published 0 file(s)"));
}

fn approve_warnings(app: &mut CadApp) {
    let plan = &app.session.ubplot.last.as_ref().unwrap().plan;
    let approvals: Vec<_> = plan
        .issues
        .iter()
        .map(|issue| ("*".to_string(), issue))
        .chain(plan.sheets.iter().flat_map(|sheet| sheet.issues.iter().map(move |issue| (sheet_key(sheet), issue))))
        .filter(|(_, issue)| issue.severity == Severity::Warn)
        .map(|(key, issue)| json!({"sheetKey": key, "code": issue.code}))
        .collect();
    if !approvals.is_empty() {
        app.run("ubplot.approve", json!({"approvals": approvals, "by": "user"})).unwrap();
    }
}

fn warn_sheets(app: &mut CadApp) {
    let plan = &mut app.session.ubplot.last.as_mut().unwrap().plan;
    for sheet in &mut plan.sheets {
        sheet.issues.push(codes::issue(Severity::Warn, codes::FRAME_OVERLAP, "Synthetic overlapping frame.", Some(&sheet.sheet_id)));
        sheet.status = derive_status(sheet, &plan.audit);
    }
}

#[test]
fn approve_all_same_cause_records_audit() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false]);
    review(&mut app, &frames);
    warn_sheets(&mut app);
    let mut h = harness(app);
    assert!(h.query_by_label("FRAME_OVERLAP · 3 sheets · WARN").is_some());
    assert!(h.query_by_label("PASS 0 · INFO 0 · WARN 3 · BLOCK 0").is_some());
    h.get_by_label("Approve all (3)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Approve 3 warnings (FRAME_OVERLAP)? This is recorded in the audit log.").is_some());
    assert!(h.state().session.ubplot.last.as_ref().unwrap().plan.audit.is_empty());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Approve").click();
    h.run_steps(3);
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(plan.audit.len(), 3);
    for sheet in &plan.sheets {
        assert_eq!(sheet.status, SheetStatus::Ready);
        let issue = sheet.issues.iter().find(|issue| issue.code == codes::FRAME_OVERLAP).unwrap();
        assert!(issue.approved_by_user);
        assert!(plan.audit.iter().any(|entry| {
            entry.action == "approve"
                && entry.by == "user"
                && entry.sheet_key == sheet_key(sheet)
                && entry.code == codes::FRAME_OVERLAP
                && entry.fingerprint == fingerprint(issue, &sheet_key(sheet))
        }));
    }
    assert_eq!(h.query_all_by_label("Approved").count(), 3);
    assert!(
        h.query_all_by_label("Approved")
            .all(|node| { node.accesskit_node().is_disabled() && node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True) })
    );
    assert!(h.get_by_label("Approve all (0)").accesskit_node().is_disabled());
}

#[test]
fn cancel_in_modal_approves_nothing() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false]);
    review(&mut app, &frames);
    warn_sheets(&mut app);
    let original = app.session.ubplot.last.as_ref().unwrap().plan.clone();
    let mut h = harness(app);
    h.get_by_label("Approve all (3)").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label("Approve 3 warnings (FRAME_OVERLAP)? This is recorded in the audit log.").is_none());
    assert_eq!(h.state().session.ubplot.last.as_ref().unwrap().plan, original);
}

#[test]
fn block_group_has_no_approve_control() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[true]);
    review(&mut app, &frames);
    let h = harness(app);
    assert!(h.query_by_label("FRAME_AMBIGUOUS · 1 sheets · BLOCK").is_some());
    assert!(h.query_by_label("Cannot be approved: fix the cause or exclude the sheet").is_some());
    for role in [egui::accesskit::Role::Button, egui::accesskit::Role::CheckBox] {
        assert!(!h.query_all_by_role(role).any(|node| node.accesskit_node().label().is_some_and(|label| label.starts_with("Approve"))));
    }
    assert!(h.query_by_label("Pick window <").is_some());
    assert!(h.query_by_label("Set paper for all").is_some());
    assert!(h.query_by_label("Exclude all").is_some());
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
}

#[test]
fn publish_disabled_until_warnings_approved() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false]);
    review(&mut app, &frames);
    warn_sheets(&mut app);
    let picked = Rc::new(RefCell::new(0));
    let calls = picked.clone();
    app.services.pick_save = Some(Box::new(move |_| {
        *calls.borrow_mut() += 1;
        None
    }));
    let mut h = harness(app);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").click();
    h.run_steps(3);
    assert_eq!(*picked.borrow(), 0);
    h.get_by_label("Approved").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ubplot.last.as_ref().unwrap().plan.audit.len(), 1);
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").click();
    h.run_steps(3);
    assert_eq!(*picked.borrow(), 1);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn split_panel_needs_explicit_click() {
    let _guard = test_lock();
    let (mut app, _) = app(&[]);
    cadcraft_engine::cmd::plotters::set_plotter_store(cadcraft_engine::cmd::plotters::PlotterStore::memory());
    app.run("plottermanager", json!({"action": "new", "name": "B9 A1", "paper": {"name": "A1 test", "width": 594, "height": 841}})).unwrap();
    let config = app.run("plottermanager", json!({"action": "get", "name": "B9 A1"})).unwrap();
    for paper in config["papers"].as_array().unwrap().iter().filter(|paper| paper["name"] != "A1 test") {
        app.run("plottermanager", json!({"action": "set", "name": "B9 A1", "removePaper": paper["name"]})).unwrap();
    }
    app.run("ubplot.scan", json!({"mode": "window", "windows": [[[0, 0], [594, 2400]]], "paper": "594x2400", "scale": "1:1", "device": "B9 A1"}))
        .unwrap();
    app.session.ubplot.review = true;
    let mut h = harness(app);
    assert!(h.query_by_label("3 pieces").is_some());
    assert!(h.query_by_label("Overlap (mm)").is_some());
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert!(!plan.sheets[0].split.as_ref().unwrap().approved);
    assert!(plan.audit.is_empty());
    h.run_steps(5);
    assert!(!h.state().session.ubplot.last.as_ref().unwrap().plan.sheets[0].split.as_ref().unwrap().approved);
    h.get_by_label("Approve split").click();
    h.run_steps(3);
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert!(plan.sheets[0].split.as_ref().unwrap().approved);
    assert_eq!(plan.sheets[0].split.as_ref().unwrap().overlap_mm, 20.0);
    assert_eq!(plan.sheets[0].status, SheetStatus::Ready);
    assert!(plan.audit.iter().any(|entry| entry.action == "split" && entry.by == "user" && entry.sheet_key == "S-001@1"));
}

#[test]
fn info_only_sheets_are_hidden_by_default() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false]);
    review(&mut app, &frames);
    let sheet = &mut app.session.ubplot.last.as_mut().unwrap().plan.sheets[0];
    sheet.issues.push(codes::issue(Severity::Info, codes::REVISION_NEWER, "Newer revision.", Some(&sheet.sheet_id)));
    let mut h = harness(app);
    assert!(h.query_by_label("S-001 · 1").is_none());
    assert!(h.query_by_label("PASS 0 · INFO 1 · WARN 0 · BLOCK 0").is_some());
    h.get_by_label("Show all sheets").click();
    h.run_steps(3);
    assert!(h.query_by_label("S-001 · 1").is_some());
    assert!(h.query_by_label("REVISION_NEWER · 1 sheets · INFO").is_some());
}

#[test]
fn publish_row_visible_at_200_percent_with_100_sheets() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false; 100]);
    review(&mut app, &frames);
    let mut h = harness_size(app, vec2(683.0, 384.0));
    h.run_steps(5);
    let dialog = h.get_by_role_and_label(egui::accesskit::Role::Window, "Quick Plot").rect();
    let visible = dialog.intersect(h.ctx.content_rect());
    for (role, label) in [
        (egui::accesskit::Role::Button, "Publish"),
        (egui::accesskit::Role::RadioButton, "Merged PDF"),
        (egui::accesskit::Role::RadioButton, "One PDF per sheet"),
    ] {
        let rect = h.get_by_role_and_label(role, label).rect();
        assert!(visible.contains_rect(rect), "{label} at {rect:?} is outside the visible dialog {visible:?}");
    }
    assert_eq!(h.state().session.ubplot.last.as_ref().unwrap().plan.sheets.len(), 100);
}

#[test]
fn blocked_sheet_can_be_excluded_at_200_percent() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false]);
    // Reduced motion keeps the wheel event whole, exposing consumption by a nested scroll area.
    app.system_animations = Some(false);
    let drawing = app.session.doc_mut().unwrap();
    drawing.header.set_i64("INSUNITS", 4);
    frame_block(drawing, "MISSING-XREF", false);
    let block = Arc::make_mut(drawing.blocks.get_mut("MISSING-XREF").unwrap());
    block.xref_path = Some("missing/fixture.dxf".into());
    block.xref_status = XrefStatus::NotFound;
    place(&mut app.session, "MISSING-XREF", 500.0);
    review(&mut app, &frames);
    let plan = &app.session.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(plan.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Blocked).count(), 1);
    assert!(plan.sheets[1].issues.iter().any(|issue| issue.code == codes::XREF_MISSING && issue.severity == Severity::Block));
    let blocked_key = sheet_key(&plan.sheets[1]);
    let mut h = harness_size(app, vec2(683.0, 384.0));
    let publish = h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish");
    assert!(h.ctx.content_rect().contains_rect(publish.rect()));
    assert!(publish.accesskit_node().is_disabled());
    publish.hover();
    h.run_steps(60);
    for label in ["Fix or exclude blocked sheets before publishing a merged PDF", "XREF_MISSING"] {
        let rect = h.get_by_label(label).rect();
        assert!(h.ctx.content_rect().contains_rect(rect), "reason {label} at {rect:?} must be visible before scrolling");
    }
    let dialog = h.get_by_role_and_label(egui::accesskit::Role::Window, "Quick Plot");
    let visible = dialog.rect().intersect(h.ctx.content_rect());
    let scan = dialog.get_by_label("Scan").rect();
    let preview = dialog.get_by_label("Preview").rect();
    let exceptions = dialog.get_by_label("Exceptions").rect();
    let footer = dialog.get_by_role_and_label(egui::accesskit::Role::Label, "Publish").rect();
    let scrollbar = dialog
        .query_all_by_role(egui::accesskit::Role::ScrollBar)
        .max_by(|left, right| left.rect().height().total_cmp(&right.rect().height()))
        .unwrap()
        .rect();
    assert!(visible.contains_rect(scrollbar));
    assert!(scrollbar.bottom() < footer.top());
    let body =
        egui::Rect::from_min_max(egui::pos2(scan.left(), scrollbar.top()), egui::pos2(scrollbar.left(), scrollbar.bottom())).intersect(visible);
    let exclude = dialog.get_by_role_and_label(egui::accesskit::Role::Button, "Exclude all").rect();
    assert!(!body.contains_rect(exclude), "the fixture must require scrolling to reach {exclude:?}");
    let pointer = egui::pos2(preview.center().x, (preview.bottom() + exceptions.top()) / 2.0);
    assert!(body.contains(pointer));
    h.event(Event::PointerMoved(pointer));
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -240.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    let dialog = h.get_by_role_and_label(egui::accesskit::Role::Window, "Quick Plot");
    let exclude = dialog.get_by_role_and_label(egui::accesskit::Role::Button, "Exclude all");
    let rect = exclude.rect();
    assert!(body.contains_rect(rect), "Exclude all at {rect:?} must lie inside the visible body {body:?}");
    assert!(rect.bottom() < dialog.get_by_role_and_label(egui::accesskit::Role::Label, "Publish").rect().top());
    exclude.click();
    h.run_steps(3);
    let last = h.state().session.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.exclude, vec![blocked_key]);
    assert_eq!(last.plan.sheets.len(), 2);
    assert!(last.plan.sheets.iter().all(|sheet| sheet.status != SheetStatus::Blocked));
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
}

#[test]
fn plan_warning_uses_one_global_audit_entry() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false, false, false]);
    review(&mut app, &frames);
    let plan = &mut app.session.ubplot.last.as_mut().unwrap().plan;
    plan.issues.push(codes::issue(Severity::Warn, codes::FONT_FALLBACK, "Synthetic font fallback.", None));
    let mut h = harness(app);
    assert!(h.query_by_label("FONT_FALLBACK · 3 sheets · WARN").is_some());
    assert_eq!(h.query_all_by_label("Approved").count(), 1);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
    h.get_by_label("Approve all (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Approve 1 warnings (FONT_FALLBACK)? This is recorded in the audit log.").is_some());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Approve").click();
    h.run_steps(3);
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(plan.audit.len(), 1);
    assert_eq!(plan.audit[0].sheet_key, "*");
    assert_eq!(plan.audit[0].code, codes::FONT_FALLBACK);
    assert!(plan.issues.iter().find(|issue| issue.code == codes::FONT_FALLBACK).unwrap().approved_by_user);
    assert!(plan.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready));
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
}

#[test]
fn warning_checkbox_requires_matching_audit() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false]);
    review(&mut app, &frames);
    warn_sheets(&mut app);
    approve_warnings(&mut app);
    let sheet = &mut app.session.ubplot.last.as_mut().unwrap().plan.sheets[0];
    sheet.issues.iter_mut().find(|issue| issue.code == codes::FRAME_OVERLAP).unwrap().message = "Changed warning cause.".into();
    let mut h = harness(app);
    let checkbox = h.get_by_label("Approved");
    assert!(!checkbox.accesskit_node().is_disabled());
    assert_eq!(checkbox.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
    h.get_by_label("Approved").click();
    h.run_steps(3);
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(plan.audit.len(), 2);
    assert_ne!(plan.audit[0].fingerprint, plan.audit[1].fingerprint);
    assert!(h.get_by_label("Approved").accesskit_node().is_disabled());
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Publish").accesskit_node().is_disabled());
}

#[test]
fn bulk_modal_does_not_approve_changed_warning() {
    let _guard = test_lock();
    let (mut app, frames) = app(&[false]);
    review(&mut app, &frames);
    warn_sheets(&mut app);
    let mut h = harness(app);
    h.get_by_label("Approve all (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Approve 1 warnings (FRAME_OVERLAP)? This is recorded in the audit log.").is_some());
    let plan = &mut h.state_mut().session.ubplot.last.as_mut().unwrap().plan;
    plan.sheets[0].issues.iter_mut().find(|issue| issue.code == codes::FRAME_OVERLAP).unwrap().message = "Changed warning cause.".into();
    h.run_steps(3);
    assert!(h.query_by_label("Approve 1 warnings (FRAME_OVERLAP)? This is recorded in the audit log.").is_none());
    let plan = &h.state().session.ubplot.last.as_ref().unwrap().plan;
    assert!(plan.audit.is_empty());
    assert!(!plan.sheets[0].issues.iter().find(|issue| issue.code == codes::FRAME_OVERLAP).unwrap().approved_by_user);
}
