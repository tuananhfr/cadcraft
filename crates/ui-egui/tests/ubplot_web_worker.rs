#[allow(dead_code)]
#[path = "../../engine/tests/support/frames.rs"]
mod frames;

use std::cell::Cell;
use std::rc::Rc;

use cadcraft_doc::Drawing;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::Sheet;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_ui_egui::ubplot::worker::FrameJobPump;
use cadcraft_ui_egui::{CadApp, Services};
use egui_kittest::Harness;
use serde_json::json;

thread_local! {
    static CLOCK: Cell<u64> = const { Cell::new(0) };
}

fn clock() -> u64 {
    CLOCK.with(Cell::get)
}

struct Renderer {
    calls: Cell<usize>,
    elapsed_ms: u64,
}

impl SheetRenderer for Renderer {
    fn render(&self, _: &Drawing, sheet: &Sheet, _: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        self.calls.set(self.calls.get() + 1);
        CLOCK.with(|clock| clock.set(clock.get().saturating_add(self.elapsed_ms)));
        Ok(sheet.sheet_id.as_bytes().to_vec())
    }

    fn merge(&self, parts: &[&[u8]], _: &[String], _: &str) -> Result<Vec<u8>, RenderFailure> {
        Ok(parts.concat())
    }
}

struct State {
    app: CadApp,
    pump: FrameJobPump,
}

fn state(renderer: Rc<Renderer>) -> State {
    CLOCK.with(|clock| clock.set(0));
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
        plot: None,
    });
    frames::install_budget_plot();
    let mut session = Session::empty();
    session.open_drawing(frames::frames_drawing(8), "Web pump fixture", None);
    session.set_job_store(Box::new(MemoryJobStore::default()));
    let pump = FrameJobPump::with_renderer(&mut session, renderer, SheetBudget::web(), clock);
    let mut app = CadApp::new(session, Services::default());
    app.skip_first_run_picker = true;
    app.ui.start_tab = false;
    State { app, pump }
}

fn enqueue(state: &mut State) -> String {
    let handles: Vec<_> = state.app.session.doc().unwrap().model.iter().map(|entity| entity.handle.hex()).collect();
    state.app.run("ubplot.scan", json!({"mode": "block", "handles": handles})).unwrap();
    let result = state.app.run("ubplot.publish", json!({"background": true, "retryLimit": 0})).unwrap();
    result["jobId"].as_str().unwrap().into()
}

#[test]
fn web_pump_renders_at_most_one_sheet_per_frame() {
    let renderer = Rc::new(Renderer { calls: Cell::new(0), elapsed_ms: 0 });
    let mut h = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
        |ui, state: &mut State| {
            state.pump.advance(&mut state.app.session, ui.ctx());
            state.app.logic(&ui.ctx().clone());
            state.app.ui(ui);
        },
        state(renderer.clone()),
    );
    let id = enqueue(h.state_mut());
    for frame in 1..=5 {
        let before = renderer.calls.get();
        h.run_steps(1);
        assert!(renderer.calls.get().saturating_sub(before) <= 1);
        assert!(renderer.calls.get() <= frame);
    }
    assert_eq!(renderer.calls.get(), 5);
    for _ in 0..30 {
        h.run_steps(1);
        if !h.state().pump.busy() {
            break;
        }
    }
    assert_eq!(renderer.calls.get(), 8);
    assert_eq!(h.state().pump.manifests[&id].status, JobStatus::Done);
    assert!(h.state().pump.errors.is_empty());
}

#[test]
fn web_pump_records_slow_sheet_warning_with_injected_clock() {
    let renderer = Rc::new(Renderer { calls: Cell::new(0), elapsed_ms: 300_001 });
    let mut state = state(renderer.clone());
    let id = enqueue(&mut state);
    let ctx = egui::Context::default();
    state.pump.advance(&mut state.app.session, &ctx);
    assert_eq!(renderer.calls.get(), 1);
    state.pump.advance(&mut state.app.session, &ctx);
    assert_eq!(renderer.calls.get(), 1);
    let manifest = &state.pump.manifests[&id];
    let sheet = manifest.sheets.first().unwrap();
    assert_eq!(sheet.status, cadcraft_engine::sheetplan::SheetStatus::Done);
    assert!(sheet.issues.iter().any(|issue| issue.code == "SHEET_SLOW" && issue.severity == cadcraft_engine::sheetplan::Severity::Warn));
    let persisted = state.app.session.job_store_mut().unwrap().read_manifest(&id).unwrap().unwrap();
    assert_eq!(PublishManifest::from_json(&persisted).unwrap(), *manifest);
}

#[test]
fn web_pump_reload_waits_for_resume_and_rejects_changed_source() {
    let renderer = Rc::new(Renderer { calls: Cell::new(0), elapsed_ms: 0 });
    let mut state = state(renderer.clone());
    let id = enqueue(&mut state);
    state.app.session.take_queued_job().unwrap();
    state.pump = FrameJobPump::with_renderer(&mut state.app.session, renderer.clone(), SheetBudget::web(), clock);
    let ctx = egui::Context::default();
    assert!(state.pump.interrupted.contains(&id));
    for _ in 0..5 {
        state.pump.advance(&mut state.app.session, &ctx);
    }
    assert_eq!(renderer.calls.get(), 0);
    state.app.run("line", json!({"points": [[0,0],[1,1]]})).unwrap();
    let error = state.pump.resume(&mut state.app.session, &id).unwrap_err();
    assert!(error.contains("SOURCE_CHANGED"));
    assert_eq!(renderer.calls.get(), 0);
    assert!(state.pump.interrupted.contains(&id));
}
