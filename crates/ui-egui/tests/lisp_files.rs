//! AutoLISP in the app (#758): a dropped `.lsp` file loads (its `c:` functions become
//! commands), and a long routine runs a slice per frame so the window stays live.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_engine::lisp::{sandbox, trust};
use cadcraft_ui_egui::{CadApp, Services};

static PERMISSIONS: Mutex<()> = Mutex::new(());

struct Permissions {
    _lock: MutexGuard<'static, ()>,
    approved: Vec<String>,
    roots: Vec<PathBuf>,
}

impl Permissions {
    fn new() -> Self {
        let lock = PERMISSIONS.lock().unwrap_or_else(|error| error.into_inner());
        let previous = Self { _lock: lock, approved: trust::approved(), roots: sandbox::write_roots() };
        trust::set_approved(Vec::new());
        previous
    }
}

impl Drop for Permissions {
    fn drop(&mut self) {
        trust::set_approved(self.approved.clone());
        sandbox::set_write_roots(self.roots.clone());
    }
}

#[test]
fn a_dropped_lisp_file_loads_and_defines_its_commands() {
    let _permissions = Permissions::new();
    let mut app = CadApp::new(Session::new(), Services::default());
    app.open_bytes("Hello.LSP", b"(defun c:hello () (princ \"hi from lisp\") (princ))");
    assert!(app.session.lisp_review_pending());
    app.session.cmdline("Y").unwrap();
    assert!(app.session.log.iter().any(|l| l == "Hello.LSP successfully loaded."), "{:?}", app.session.log);
    // Still the same drawing: a LISP file isn't opened as one.
    assert_eq!(app.session.docs.len(), 1);
    app.session.cmdline("HELLO").unwrap();
    assert_eq!(app.session.log.last().map(String::as_str), Some("hi from lisp"));
}

#[test]
fn a_long_routine_runs_across_frames() {
    let _permissions = Permissions::new();
    let mut app = CadApp::new(Session::new(), Services::default());
    app.open_bytes("loop.lsp", b"(setq i 0) (while (< i 2000000) (setq i (1+ i))) (princ \"done\")");
    assert!(app.session.lisp_review_pending());
    app.session.cmdline("Y").unwrap();
    assert!(app.session.lisp_busy(), "a slice per frame");
    let ctx = egui::Context::default();
    let mut frames = 0;
    while app.session.lisp_running() && frames < 1000 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx()));
        out.textures_delta.clear(); // no renderer here
        frames += 1;
    }
    assert!(!app.session.lisp_running());
    assert!(frames > 1, "it took more than one frame");
    // The load is reported once the routine's own output is flushed.
    let tail: Vec<_> = app.session.log.iter().rev().take(2).rev().map(String::as_str).collect();
    assert_eq!(tail, ["done", "loop.lsp successfully loaded."]);
}

/// A click on the canvas answers a routine's `getpoint`. No command runs while a routine waits,
/// and clicks used to go to object selection instead (found running the app by hand).
#[test]
fn a_canvas_click_answers_a_routine_waiting_for_a_point() {
    use cadcraft_engine::geom::Vec2;
    use egui::{Event, Modifiers, PointerButton, vec2};
    let mut app = CadApp::new(Session::new(), Services::default());
    app.canvas.zoom_pending = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h.state_mut().session.cmdline("(setq p (getpoint \"\nPick: \"))").unwrap();
    h.run_steps(2);
    let target = Vec2::new(1.0, 2.0);
    let pos = h.state().canvas.xf.unwrap().to_screen(target);
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    let app = h.state();
    assert!(!app.session.lisp_running(), "the click answered the prompt");
    let p = app.session.lisp_var("P").as_point().expect("a point");
    assert!((p[0] - target.x).abs() < 0.05 && (p[1] - target.y).abs() < 0.05, "{p:?}");
    assert!(app.session.selection().is_empty(), "the click selected nothing");
}

#[test]
fn lisp_approval_round_trips_through_host_profile() {
    use cadcraft_engine::lisp::trust::{approve, approved, is_approved, revoke, source_sha};

    let _permissions = Permissions::new();

    let text = "(setq profile_review 701)";
    let sha = source_sha(text);
    let was_approved = is_approved(&sha);
    let mut app = CadApp::new(Session::new(), Services::default());
    assert!(approve(&sha));
    let saved = app.profile_json();
    assert!(serde_json::from_str::<serde_json::Value>(&saved).unwrap()["lispApproved"].as_array().unwrap().contains(&serde_json::json!(sha)));
    assert!(revoke(&sha));
    app.load_profile(&saved);
    assert!(is_approved(&sha));
    app.open_bytes("profile-review.lsp", text.as_bytes());
    assert!(!app.session.lisp_review_pending());
    assert_eq!(app.session.lisp_var("PROFILE_REVIEW").as_number(), Some(701.0));
    let before = approved();
    app.load_profile(r#"{"lispApproved": "bad"}"#);
    app.load_profile(r#"{"fonts": {}}"#);
    assert_eq!(approved(), before);
    if !was_approved {
        revoke(&sha);
    }
}

#[test]
fn panel_shows_review_state() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    let _permissions = Permissions::new();
    let reviewed = "(defun c:panel-edit () (command \"line\" \"0,0\" \"10,0\" \"\"))";
    let unreviewed = "(setq panel_unreviewed 13)";
    let reviewed_sha = trust::source_sha(reviewed);
    let unreviewed_sha = trust::source_sha(unreviewed);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.run("lisptrust", json!({"action": "approve", "sha256": reviewed_sha})).unwrap();
    app.run("appload", json!({"name": "reviewed.lsp", "text": reviewed})).unwrap();
    app.open_bytes("unreviewed.lsp", unreviewed.as_bytes());
    assert!(app.session.lisp_review_pending());
    app.session.cmdline("Y").unwrap();
    assert!(!trust::is_approved(&unreviewed_sha));
    assert!(app.session.lisp_audit().is_empty());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(720.0, 768.0))
        .build_ui_state(|ui, app: &mut CadApp| cadcraft_ui_egui::i18n::with_language("en", || cadcraft_ui_egui::lisp_panel::body(ui, app)), app);
    harness.run_steps(3);
    for label in ["reviewed.lsp", "unreviewed.lsp", "Reviewed", "Unreviewed", &reviewed_sha[..8], &unreviewed_sha[..8]] {
        assert!(harness.query_by_label(label).is_some(), "missing {label}");
    }
    assert!(harness.query_by_label("No LISP edits in this session").is_some());
    assert_eq!(harness.query_all_by_label("Created: 1").count(), 0);

    let invalid = "(defun";
    harness.state_mut().run("lisptrust", json!({"action": "approve", "sha256": trust::source_sha(invalid)})).unwrap();
    assert!(harness.state_mut().run("appload", json!({"name": "reviewed.lsp", "text": invalid})).is_err());
    harness.run_steps(3);
    assert!(harness.query_by_label(&reviewed_sha[..8]).is_some());
    assert!(harness.query_by_label(&trust::source_sha(invalid)[..8]).is_none());

    harness.state_mut().session.cmdline("PANEL-EDIT").unwrap();
    assert_eq!(harness.state().session.lisp_audit().len(), 1);
    let record = harness.state().session.lisp_audit().into_iter().next().unwrap();
    assert_eq!((record.created, record.modified, record.deleted), (1, 0, 0));
    harness.run_steps(3);
    assert!(harness.query_by_label("No LISP edits in this session").is_none());
    assert_eq!(harness.query_all_by_label("Created: 1").count(), 1);
    assert!(harness.query_by_label("Modified: 0").is_some());
    assert!(harness.query_by_label("Deleted: 0").is_some());
    assert_eq!(harness.query_all_by_label("reviewed.lsp").count(), 2);
    assert!(harness.query_by_label(format!("Time (Unix ms): {}", record.at_ms).as_str()).is_some());
    assert!(harness.query_by_label(format!("Undo: {}", record.undo_label).as_str()).is_some());

    harness.state_mut().run("lisptrust", json!({"action": "revoke", "sha256": reviewed_sha})).unwrap();
    harness.run_steps(3);
    assert!(harness.query_by_label("Reviewed").is_none());
    assert_eq!(harness.query_all_by_label("Unreviewed").count(), 2);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn panel_folder_controls_use_sandbox_commands() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let _permissions = Permissions::new();
    sandbox::set_write_roots(Vec::new());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(720.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| cadcraft_ui_egui::i18n::with_language("en", || cadcraft_ui_egui::lisp_panel::body(ui, app)),
        CadApp::new(Session::new(), Services::default()),
    );
    harness.run_steps(3);
    harness.get_by_label("Allowed folders…").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("No allowed folders").is_some());
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Folder path").type_text(&root.to_string_lossy());
    harness.run_steps(3);
    harness.get_by_label("Add").click();
    harness.run_steps(3);
    assert_eq!(sandbox::write_roots(), std::slice::from_ref(&root));
    assert!(harness.query_by_label(root.to_string_lossy().as_ref()).is_some());
    harness.get_by_label("Remove").click();
    harness.run_steps(3);
    assert!(sandbox::write_roots().is_empty());
    assert!(harness.query_by_label("No allowed folders").is_some());
    assert!(harness.state().session.lisp_audit().is_empty());
}
