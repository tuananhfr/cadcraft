use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cadcraft_doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Insert, Line, LwPolyline, Space};
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::{Severity, SheetMode, SheetSource, SheetStatus, codes, ids::sheet_key};
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
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
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
    assert!(h.query_by_label("FRAME_OVERLAP · 1 sheets").is_some());
    assert!(h.query_by_label("FRAME_AMBIGUOUS · 1 sheets").is_some());
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
