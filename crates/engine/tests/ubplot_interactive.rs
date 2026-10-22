mod support;

use cadcraft_doc::{EntityKind, Handle, Layout};
use cadcraft_engine::sheetplan::{ScanMode, SheetSource, detect_frames};
use cadcraft_engine::{Accept, Input, Session};
use cadcraft_geom::Vec2;
use serde_json::json;
use support::frames::{anon_dynamic, frame_block, nest, place, title_only_block};

fn drawing() -> (Session, [Handle; 3], Handle) {
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "FRAME-A3", 420.0, 297.0);
    let frames = [0.0, 500.0, 1000.0].map(|x| place(&mut s, "FRAME-A3", [x, 0.0], 1.0, 0.0, false, &[]));
    let result = s.execute("line", &json!({"points": [[0, -100], [100, -100]]})).unwrap();
    let line = Handle(u64::from_str_radix(result["handles"][0].as_str().unwrap(), 16).unwrap());
    s.set_selection(Vec::new());
    (s, frames, line)
}

fn block(s: &mut Session) {
    s.start("ubplot").unwrap();
    assert_eq!(s.current_prompt().unwrap().accept, Accept::TEXT);
    assert_eq!(s.current_prompt().unwrap().display(), "Plot frames by [Block/Window/Layout/Last] <Block>:");
    s.input(Input::Enter).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Select frame blocks");
}

fn outline(s: &Session, handle: Handle) -> Vec<Vec2> {
    detect_frames(s.doc().unwrap(), &[handle], &s.ubplot.registry)[0].outline.unwrap().map(|[x, y]| Vec2::new(x, y)).into()
}

#[test]
fn ubplot_typed_block_mode_filters_non_frames() {
    let (mut s, [f1, f2, f3], line) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1, line, f2, f3])).unwrap();
    s.input(Input::Enter).unwrap();
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan.sheets.len(), 3);
    assert!(s.ubplot.review);
    assert!(s.log.iter().any(|entry| entry == "3 frame(s) added, 1 ignored (not frames), 3 total"));
}

#[test]
fn ubplot_marks_number_in_pick_order() {
    let (mut s, [f1, _, f3], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f3])).unwrap();
    s.input(Input::Pick(vec![f1])).unwrap();
    let marks = s.marks();
    assert_eq!(marks.iter().map(|mark| mark.label.as_str()).collect::<Vec<_>>(), ["1", "2"]);
    assert_eq!(marks[0].outline, outline(&s, f3));
    assert_eq!(marks[1].outline, outline(&s, f1));
    assert!(marks.iter().all(|mark| !mark.warn));
}

#[test]
fn ubplot_shift_pick_removes() {
    let (mut s, [f1, f2, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1, f2])).unwrap();
    s.input_removing(Input::Pick(vec![f1])).unwrap();
    let marks = s.marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].label, "1");
    assert_eq!(marks[0].outline, outline(&s, f2));
    s.input(Input::Enter).unwrap();
    let source = &s.ubplot.last.as_ref().unwrap().plan.sheets[0].source;
    assert!(matches!(source, SheetSource::Block { block_handle, .. } if block_handle == &f2.hex()));
}

#[test]
fn ubplot_repick_does_not_duplicate() {
    let (mut s, [f1, _, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1, f1])).unwrap();
    s.input(Input::Pick(vec![f1])).unwrap();
    assert_eq!(s.marks().len(), 1);
    s.input(Input::Enter).unwrap();
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan.sheets.len(), 1);
}

#[test]
fn ubplot_enter_finishes_and_stores_plan_for_review() {
    let (mut s, [f1, f2, _], _) = drawing();
    let state = s.state().unwrap();
    let (doc, revision, undo_len) = (state.doc.clone(), state.revision, state.undo.len());
    block(&mut s);
    s.input(Input::Pick(vec![f2, f1])).unwrap();
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert!(s.marks().is_empty());
    assert!(s.ubplot.review);
    let last = s.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.mode, ScanMode::Block);
    assert_eq!(last.request.handles, [f2.hex(), f1.hex()]);
    assert_eq!(last.plan.sheets.len(), 2);
    assert!(s.log.iter().any(|entry| entry == "UBPLOT: 2 sheet(s), 0 blocked, 0 warning(s)"));
    let state = s.state().unwrap();
    assert!(std::sync::Arc::ptr_eq(&state.doc, &doc));
    assert_eq!(state.revision, revision);
    assert_eq!(state.undo.len(), undo_len);
}

#[test]
fn ubplot_esc_cancels_without_plan() {
    let (mut s, [f1, _, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1])).unwrap();
    s.input(Input::Cancel).unwrap();
    assert!(s.ubplot.last.is_none());
    assert!(!s.ubplot.review);
    assert!(s.running.is_none());
    assert!(s.marks().is_empty());
    s.set_selection(Vec::new());
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": [f1.hex()]})).unwrap();
    let previous = s.ubplot.last.as_ref().unwrap().plan.clone();
    block(&mut s);
    s.cancel();
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, previous);
    assert!(!s.ubplot.review);
}

#[test]
fn ubplot_window_mode_multiple_windows() {
    let (mut s, _, _) = drawing();
    s.start("ubplot").unwrap();
    s.cmdline("W").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Specify first corner of sheet window");
    for [first, opposite] in [[Vec2::new(420.0, 297.0), Vec2::ZERO], [Vec2::new(500.0, 0.0), Vec2::new(920.0, 297.0)]] {
        s.input(Input::Point(first)).unwrap();
        let prompt = s.current_prompt().unwrap();
        assert_eq!(prompt.message, "Specify opposite corner");
        assert_eq!(prompt.base, Some(first));
        assert!(prompt.corner);
        s.input(Input::Point(opposite)).unwrap();
    }
    s.input(Input::Enter).unwrap();
    let last = s.ubplot.last.as_ref().unwrap();
    assert_eq!(last.plan.sheets.len(), 2);
    assert_eq!(last.request.mode, ScanMode::Window);
    for (index, x) in [0.0, 500.0].into_iter().enumerate() {
        assert_eq!(last.plan.sheets[index].order, (index + 1) as u32);
        let SheetSource::Window { outline, .. } = last.plan.sheets[index].source else { panic!("window source required") };
        assert_eq!(outline, [[x, 0.0], [x + 420.0, 0.0], [x + 420.0, 297.0], [x, 297.0]]);
    }
    assert!(s.ubplot.review);
    assert!(s.running.is_none());
}

#[test]
fn ubplot_layout_mode() {
    let (mut s, _, _) = drawing();
    s.doc_mut().unwrap().layouts = vec![Layout::new("Second", 2), Layout::new("First", 1)];
    s.start("ubplot").unwrap();
    s.cmdline("L").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Enter layout names (comma-separated) or *");
    s.cmdline("*").unwrap();
    let last = s.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.mode, ScanMode::Layout);
    assert_eq!(last.plan.sheets.len(), 2);
    assert_eq!(last.plan.sheets[0].source, SheetSource::Layout { name: "First".into() });
    assert_eq!(last.plan.sheets[1].source, SheetSource::Layout { name: "Second".into() });
    assert!(s.ubplot.review);
    assert!(s.running.is_none());
    s.start("ubplot").unwrap();
    s.cmdline("Layout").unwrap();
    s.cmdline("Second, First").unwrap();
    let last = s.ubplot.last.as_ref().unwrap();
    assert_eq!(last.request.layouts, ["Second", "First"]);
    assert_eq!(last.plan.sheets[0].source, SheetSource::Layout { name: "Second".into() });
}

#[test]
fn ubplot_last_keyword() {
    let (mut s, [f1, _, _], _) = drawing();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": [f1.hex()]})).unwrap();
    let previous = s.ubplot.last.as_ref().unwrap().plan.clone();
    s.start("ubplot").unwrap();
    s.cmdline("Last").unwrap();
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, previous);
    assert_eq!(s.ubplot.last.as_ref().unwrap().request.mode, ScanMode::Block);
    assert!(s.ubplot.review);
    assert!(s.running.is_none());
}

#[test]
fn ubplot_pickfirst_frames_skip_mode_prompt() {
    let (mut s, [f1, f2, _], line) = drawing();
    s.set_selection(vec![f2, line, f1]);
    s.start("ubplot").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Select frame blocks");
    assert_eq!(s.marks().len(), 2);
    assert_eq!(s.marks()[0].outline, outline(&s, f2));
    s.input(Input::Enter).unwrap();
    assert_eq!(s.ubplot.last.as_ref().unwrap().request.handles, [f2.hex(), f1.hex()]);
    assert!(s.ubplot.review);
}

#[test]
fn ubplot_json_call_opens_no_prompt() {
    let (mut s, [f1, _, _], _) = drawing();
    let spec = cadcraft_engine::find_command("ubplot").unwrap();
    assert_eq!(spec.label, "Quick Plot...");
    assert_eq!(spec.menu, ["File", "Quick Plot..."]);
    assert_eq!(spec.aliases, ["ubp"]);
    assert_eq!(spec.params, "as ubplot.scan (JSON: scans and stores the plan, opens nothing); typed: prompts");
    assert!(!spec.undoable);
    assert!(spec.interactive.is_some());
    s.ubplot.review = true;
    let result = s.execute("ubplot", &json!({"mode": "block", "handles": [f1.hex()]})).unwrap();
    assert_eq!(result["summary"]["sheets"], 1);
    assert!(s.ubplot.last.is_some());
    assert!(s.running.is_none());
    assert!(s.current_prompt().is_none());
    assert!(s.marks().is_empty());
    assert!(!s.ubplot.review);
    s.start("ubp").unwrap();
    assert_eq!(s.running.as_ref().unwrap().id, "ubplot");
}

#[test]
fn ubplot_marks_survive_guard_wrapper() {
    let (mut s, [f1, _, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1])).unwrap();
    let marks = s.marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].outline, outline(&s, f1));
    let value = serde_json::to_value(&marks[0]).unwrap();
    assert_eq!(value["label"], "1");
    assert_eq!(value["warn"], false);
    assert_eq!(value["outline"].as_array().unwrap().len(), 4);
}

#[test]
fn ubplot_marks_warn_on_mirrored_nested_dynamic_frame() {
    let (mut s, _, _) = drawing();
    let d = s.doc_mut().unwrap();
    anon_dynamic(d, "FRAME-A3", "*U7");
    nest(d, "NESTED", "*U7", [5.0, 5.0], 30.0);
    let frame = place(&mut s, "NESTED", [3000.0, 0.0], 1.0, 0.0, true, &[]);
    block(&mut s);
    s.input(Input::Pick(vec![frame])).unwrap();
    let marks = s.marks();
    assert_eq!(marks.len(), 1);
    assert!(marks[0].warn);
    assert_eq!(marks[0].outline, outline(&s, frame));
    s.input(Input::Enter).unwrap();
    assert!(s.ubplot.last.as_ref().unwrap().plan.sheets[0].issues.iter().any(|issue| issue.code == "FRAME_MIRRORED"));
}

#[test]
fn ubplot_marks_use_bounds_only_for_missing_outline() {
    let (mut s, [f1, _, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1])).unwrap();
    title_only_block(s.doc_mut().unwrap(), "FRAME-A3");
    let marks = s.marks();
    assert_eq!(marks.len(), 1);
    assert!(marks[0].warn);
    assert_eq!(marks[0].outline, [Vec2::ZERO, Vec2::new(180.0, 0.0), Vec2::new(180.0, 60.0), Vec2::new(0.0, 60.0)]);
    s.input(Input::Enter).unwrap();
    let sheet = &s.ubplot.last.as_ref().unwrap().plan.sheets[0];
    assert!(sheet.issues.iter().any(|issue| issue.code == "FRAME_AMBIGUOUS"));
    assert!(matches!(sheet.source, SheetSource::Block { outline, .. } if outline == [[0.0; 2]; 4]));
}

#[test]
fn ubplot_empty_enter_cancels_and_invalid_window_reprompts() {
    let (mut s, _, line) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![line])).unwrap();
    assert!(s.marks().is_empty());
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert!(s.ubplot.last.is_none());
    assert!(s.log.iter().any(|entry| entry == "No frames selected"));
    s.set_selection(Vec::new());
    s.start("ubplot").unwrap();
    s.cmdline("Window").unwrap();
    s.input(Input::Point(Vec2::new(f64::NAN, 0.0))).unwrap();
    s.input(Input::Point(Vec2::ZERO)).unwrap();
    s.input(Input::Point(Vec2::new(0.0, 10.0))).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Specify opposite corner");
    s.input(Input::Point(Vec2::new(420.0, 297.0))).unwrap();
    s.input(Input::Enter).unwrap();
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan.sheets.len(), 1);
}

#[test]
fn ubplot_hostile_selection_is_bounded() {
    let (mut s, [f1, _, _], _) = drawing();
    block(&mut s);
    s.input(Input::Pick(vec![f1; 10_001])).unwrap();
    assert!(s.marks().is_empty());
    assert!(s.ubplot.last.is_none());
    assert!(s.log.iter().any(|entry| entry.contains("selection exceeds 10000 entries")));
    s.input(Input::Pick(vec![f1])).unwrap();
    assert_eq!(s.marks().len(), 1);
    s.cancel();
    assert!(s.doc().unwrap().entity(f1).is_some_and(|entity| matches!(entity.kind, EntityKind::Insert(_))));
}
