#![cfg(not(target_arch = "wasm32"))]

#[allow(dead_code)]
#[path = "../../engine/tests/support/frames.rs"]
mod frames;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use cadcraft_doc::Drawing;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::Sheet;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_ui_egui::ubplot::worker::{JobPump, JobWorker, WorkerEvent};
use cadcraft_ui_egui::{CadApp, Services};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("cadcraft-worker-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn jobs(&self) -> PathBuf {
        self.0.join("jobs")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct TestRenderer {
    calls: Mutex<Vec<(ThreadId, String)>>,
    hold: AtomicBool,
    wait_for_deadline: bool,
}

impl SheetRenderer for TestRenderer {
    fn render(&self, drawing: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        self.calls.lock().unwrap().push((thread::current().id(), sheet.sheet_id.clone()));
        let started = Instant::now();
        while (self.hold.load(Ordering::Relaxed) || self.wait_for_deadline) && !budget.cancel.load(Ordering::Relaxed) {
            if started.elapsed() > Duration::from_secs(10) {
                return Err(RenderFailure::Error("test renderer was not released".into()));
            }
            thread::sleep(Duration::from_millis(1));
        }
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(RenderFailure::Cancelled);
        }
        thread::sleep(Duration::from_millis(20));
        PdfSheetRenderer.render(drawing, sheet, budget)
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, RenderFailure> {
        PdfSheetRenderer.merge(parts, labels, title)
    }
}

fn session(count: usize) -> Session {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
        plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
    });
    frames::install_budget_plot();
    let mut session = Session::empty();
    session.open_drawing(frames::frames_drawing(count), "Print worker fixture", None);
    session
}

fn app(count: usize, dir: &TempDir, renderer: Arc<TestRenderer>) -> CadApp {
    let mut session = session(count);
    session.set_job_store(Box::new(DirJobStore::new(dir.jobs())));
    let mut app = CadApp::new(session, Services::default());
    app.skip_first_run_picker = true;
    app.ui.start_tab = false;
    app.job_pump = JobPump::with_renderer(dir.jobs(), renderer, SheetBudget::desktop());
    app
}

fn enqueue(app: &mut CadApp, dir: &TempDir, stem: &str) -> String {
    let handles: Vec<_> = app.session.doc().unwrap().model.iter().map(|entity| entity.handle.hex()).collect();
    app.run("ubplot.scan", json!({"mode": "block", "handles": handles})).unwrap();
    let path = dir.0.join("out").join(format!("{stem}.pdf"));
    let result = app.run("ubplot.publish", json!({"background": true, "path": path, "retryLimit": 0})).unwrap();
    assert_eq!(result["status"], "QUEUED");
    result["jobId"].as_str().unwrap().to_string()
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    )
}

fn drive_until(h: &mut Harness<'_, CadApp>, condition: impl Fn(&CadApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition(h.state()) {
        assert!(Instant::now() < deadline, "worker did not finish: {:?}", h.state().job_pump.errors);
        h.run_steps(1);
        thread::sleep(Duration::from_millis(2));
    }
}

fn manifest(root: &Path, id: &str) -> PublishManifest {
    let text = DirJobStore::new(root.into()).read_manifest(id).unwrap().unwrap();
    PublishManifest::from_json(&text).unwrap()
}

fn interrupt(app: &mut CadApp, dir: &TempDir, id: &str) {
    assert_eq!(app.session.take_queued_job().unwrap().job_id, id);
    let mut stored = manifest(&dir.jobs(), id);
    stored.status = JobStatus::Running;
    app.session.job_store_mut().unwrap().commit(id, &[], &stored.to_json()).unwrap();
}

#[test]
fn desktop_worker_runs_off_ui_thread_and_reports_progress() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut app = app(20, &dir, renderer.clone());
    let id = enqueue(&mut app, &dir, "twenty");
    app.run("ui.ubplot.jobs", json!(null)).unwrap();
    let ui_thread = thread::current().id();
    let mut h = harness(app);
    let mut saw_progress = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !h.state().job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done) {
        assert!(Instant::now() < deadline);
        let frame = Instant::now();
        h.run_steps(1);
        assert!(frame.elapsed() < Duration::from_secs(10), "UI frame blocked on worker rendering");
        saw_progress |= h.state().job_pump.progress.as_ref().is_some_and(|(_, progress)| progress.done > 0 && progress.done < progress.total);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(saw_progress);
    let calls = renderer.calls.lock().unwrap();
    assert_eq!(calls.len(), 20);
    assert!(calls.iter().all(|(thread, _)| *thread != ui_thread));
    assert!(calls.iter().all(|(thread, _)| *thread == calls[0].0));
    let stored = manifest(&dir.jobs(), &id);
    assert_eq!(stored.status, JobStatus::Done);
    assert_eq!(stored.files.len(), 1);
    assert_eq!(stored.files[0].pages, 20);
    assert!(h.state().job_pump.errors.is_empty());
}

#[test]
fn deadline_cancels_slow_sheet() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer { wait_for_deadline: true, ..Default::default() });
    let mut app = app(1, &dir, renderer.clone());
    let id = enqueue(&mut app, &dir, "deadline");
    let job = app.session.take_queued_job().unwrap();
    let loader = SnapshotLoader(Arc::new(app.session.doc().unwrap().clone()), job.sources[0].source_hash.clone());
    let budget = SheetBudget { time_ms: 100, ..SheetBudget::desktop() };
    let mut worker = JobWorker::spawn_with_renderer(job, Box::new(loader), Box::new(DirJobStore::new(dir.jobs())), budget, renderer);
    let deadline = Instant::now() + Duration::from_secs(30);
    let finished = loop {
        assert!(Instant::now() < deadline, "watchdog did not release renderer");
        let mut finished = None;
        for event in worker.poll() {
            match event {
                WorkerEvent::Finished(manifest) => finished = Some(manifest),
                WorkerEvent::Failed(error) => panic!("worker failed: {error}"),
                WorkerEvent::Progress(_) => {}
            }
        }
        if let Some(manifest) = finished {
            break manifest;
        }
        thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(finished.status, JobStatus::Failed);
    assert_eq!(finished.sheets[0].status, cadcraft_engine::sheetplan::SheetStatus::Failed);
    assert_eq!(finished.sheets[0].attempts, 1);
    assert!(finished.sheets[0].issues.iter().any(|issue| issue.code == "RENDER_TIMEOUT"));
    assert_eq!(manifest(&dir.jobs(), &id), finished);
}

#[test]
fn startup_offers_resume_for_unfinished_job() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut previous = app(3, &dir, renderer.clone());
    let id = enqueue(&mut previous, &dir, "interrupted");
    interrupt(&mut previous, &dir, &id);
    drop(previous);
    let mut h = harness(app(3, &dir, renderer.clone()));
    h.run_steps(5);
    assert!(h.state().job_pump.interrupted.contains(&id));
    assert!(h.state().status.as_ref().is_some_and(|(status, _)| status.contains("was interrupted")));
    assert!(renderer.calls.lock().unwrap().is_empty());
    assert_eq!(manifest(&dir.jobs(), &id).status, JobStatus::Running);
    let panel = h.get_by_role_and_label(Role::Window, "Plot Jobs");
    assert!(panel.query_by_label(&format!("Print job {id} was interrupted")).is_some());
    assert!(!panel.get_by_label("Resume").accesskit_node().is_disabled());
    panel.get_by_label("Resume").click();
    h.run_steps(2);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done));
    assert_eq!(renderer.calls.lock().unwrap().len(), 3);
    assert!(!h.state().job_pump.interrupted.contains(&id));
}

#[test]
fn cancel_button_cancels_job() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    renderer.hold.store(true, Ordering::Relaxed);
    let mut app = app(20, &dir, renderer.clone());
    let id = enqueue(&mut app, &dir, "cancel");
    app.run("ui.ubplot.jobs", json!(null)).unwrap();
    let mut h = harness(app);
    drive_until(&mut h, |_| !renderer.calls.lock().unwrap().is_empty());
    h.get_by_role_and_label(Role::Window, "Plot Jobs").get_by_label("Cancel").click();
    h.run_steps(2);
    renderer.hold.store(false, Ordering::Relaxed);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Cancelled));
    assert_eq!(manifest(&dir.jobs(), &id).status, JobStatus::Cancelled);
    assert_eq!(renderer.calls.lock().unwrap().len(), 1);
    assert!(h.get_by_role_and_label(Role::Window, "Plot Jobs").query_by_label("Resume").is_some());
}

#[test]
fn startup_discard_removes_job_without_rendering() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut previous = app(1, &dir, renderer.clone());
    let id = enqueue(&mut previous, &dir, "discard");
    interrupt(&mut previous, &dir, &id);
    drop(previous);
    let mut h = harness(app(1, &dir, renderer.clone()));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Window, "Plot Jobs").get_by_label("Discard").click();
    h.run_steps(3);
    assert!(DirJobStore::new(dir.jobs()).read_manifest(&id).unwrap().is_none());
    assert!(!h.state().job_pump.manifests.contains_key(&id));
    assert!(renderer.calls.lock().unwrap().is_empty());
}

#[test]
fn desktop_jobs_run_fifo_on_one_worker_at_a_time() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    renderer.hold.store(true, Ordering::Relaxed);
    let mut app = app(2, &dir, renderer.clone());
    let first = enqueue(&mut app, &dir, "first");
    let second = enqueue(&mut app, &dir, "second");
    let mut h = harness(app);
    drive_until(&mut h, |_| !renderer.calls.lock().unwrap().is_empty());
    h.run_steps(5);
    assert_eq!(renderer.calls.lock().unwrap().len(), 1);
    assert_eq!(manifest(&dir.jobs(), &second).status, JobStatus::Queued);
    for command in ["ubplot.jobs", "UBPLOT.JOBS", "cad.other.ubplot.jobs"] {
        for action in ["resume", "remove"] {
            assert!(h.state_mut().run(command, json!({"action": action, "jobId": first})).is_err());
            let text = format!("{command} {}", json!({"action": action, "jobId": first}));
            assert!(h.state_mut().cmdline(&text).is_some());
        }
    }
    renderer.hold.store(false, Ordering::Relaxed);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&second).is_some_and(|manifest| manifest.status == JobStatus::Done));
    assert_eq!(manifest(&dir.jobs(), &first).status, JobStatus::Done);
    let calls = renderer.calls.lock().unwrap();
    assert_eq!(calls.iter().map(|(_, sheet)| sheet.as_str()).collect::<Vec<_>>(), ["KT-01", "KT-02", "KT-01", "KT-02"]);
    assert_eq!(calls[0].0, calls[1].0);
    assert_eq!(calls[2].0, calls[3].0);
    assert_ne!(calls[0].0, calls[2].0);
}

#[test]
fn startup_queued_job_waits_for_explicit_resume() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut previous = app(2, &dir, renderer.clone());
    let id = enqueue(&mut previous, &dir, "queued");
    drop(previous);
    let mut h = harness(app(2, &dir, renderer.clone()));
    h.run_steps(5);
    assert!(h.state().job_pump.interrupted.contains(&id));
    assert!(!h.state().job_pump.busy());
    assert!(renderer.calls.lock().unwrap().is_empty());
    assert_eq!(manifest(&dir.jobs(), &id).status, JobStatus::Queued);
    h.get_by_role_and_label(Role::Window, "Plot Jobs").get_by_label("Resume").click();
    h.run_steps(2);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done));
    assert_eq!(renderer.calls.lock().unwrap().len(), 2);
}

#[test]
fn cancelling_pending_job_never_starts_its_renderer() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    renderer.hold.store(true, Ordering::Relaxed);
    let mut app = app(2, &dir, renderer.clone());
    let first = enqueue(&mut app, &dir, "first");
    let second = enqueue(&mut app, &dir, "second");
    let mut h = harness(app);
    drive_until(&mut h, |_| !renderer.calls.lock().unwrap().is_empty());
    h.state_mut().run("ubplot.jobs", json!({"action": "cancel", "jobId": second})).unwrap();
    h.run_steps(3);
    assert_eq!(manifest(&dir.jobs(), &second).status, JobStatus::Cancelled);
    assert_eq!(manifest(&dir.jobs(), &first).status, JobStatus::Running);
    assert_eq!(renderer.calls.lock().unwrap().len(), 1);
    renderer.hold.store(false, Ordering::Relaxed);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&first).is_some_and(|manifest| manifest.status == JobStatus::Done));
    assert_eq!(renderer.calls.lock().unwrap().len(), 2);
    assert_eq!(manifest(&dir.jobs(), &second).status, JobStatus::Cancelled);
    assert!(!h.state().job_pump.busy());
}

#[test]
fn typed_jobs_list_keeps_json_reply() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut app = app(1, &dir, renderer);
    let id = enqueue(&mut app, &dir, "list");
    assert!(app.cmdline(r#"UBPLOT.JOBS {"action":"list"}"#).is_none());
    let result: serde_json::Value = serde_json::from_str(app.session.log.last().unwrap()).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["jobId"], id);
    assert_eq!(result[0]["status"], "QUEUED");
}
