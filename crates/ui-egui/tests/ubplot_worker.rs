#![cfg(not(target_arch = "wasm32"))]

#[allow(dead_code)]
#[path = "../../engine/tests/support/frames.rs"]
mod frames;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use cadcraft_doc::{Drawing, EntityKind};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_engine::sheetplan::{FrameRegistry, Sheet, SheetStatus};
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
    drawing_hashes: Mutex<Vec<String>>,
    hold: AtomicBool,
    wait_for_deadline: bool,
}

impl SheetRenderer for TestRenderer {
    fn render(&self, drawing: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        self.calls.lock().unwrap().push((thread::current().id(), sheet.sheet_id.clone()));
        self.drawing_hashes.lock().unwrap().push(sha256_hex(cadcraft_io::write_dxf(drawing).as_bytes()));
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
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
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

fn enqueue_folder(app: &mut CadApp, dir: &TempDir) -> String {
    let input = dir.0.join("input");
    std::fs::create_dir(&input).unwrap();
    for (name, count) in [("a", 1), ("b", 2)] {
        let mut drawing = frames::frames_drawing(count);
        let handles: Vec<_> = drawing.model.iter().map(|entity| entity.handle).collect();
        for handle in handles {
            drawing.model.modify(handle, |entity| {
                if let EntityKind::Insert(insert) = &mut entity.kind {
                    for attribute in &mut insert.attribs {
                        if attribute.tag == "SHEET_NO" {
                            attribute.text.value = format!("{name}-{}", attribute.text.value);
                        }
                    }
                }
            });
        }
        std::fs::write(input.join(format!("{name}.dxf")), cadcraft_io::write(&drawing, "fixture.dxf").unwrap()).unwrap();
    }
    let mut registry = FrameRegistry::builtin();
    let mut template = registry.templates[0].clone();
    template.id = "test-a3".into();
    template.block_names = vec!["FRAME-A3".into()];
    registry.templates.insert(0, template);
    app.run("ubplot.templates", json!({"action": "import", "text": serde_json::to_string(&registry).unwrap()})).unwrap();
    let response = app.run("ubplot.folder", json!({"folder": input, "out": dir.0.join("out"), "background": true})).unwrap();
    assert_eq!(response["status"], "QUEUED");
    response["jobId"].as_str().unwrap().to_string()
}

fn assert_folder_result(dir: &TempDir, id: &str, renderer: &TestRenderer) {
    let done = manifest(&dir.jobs(), id);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.sources.len(), 2);
    let mut expected_hashes = Vec::new();
    for (source, (name, count)) in done.sources.iter().zip([("a", 1), ("b", 2)]) {
        let path = dir.0.join("input").join(format!("{name}.dxf"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(std::fs::canonicalize(source.path.as_deref().unwrap()).unwrap(), std::fs::canonicalize(&path).unwrap());
        assert_eq!(source.source_hash, sha256_hex(&bytes));
        assert_eq!(source.sheets, count);
        let drawing = cadcraft_io::read(&bytes, "fixture.dxf").unwrap();
        let hash = sha256_hex(cadcraft_io::write_dxf(&drawing).as_bytes());
        expected_hashes.extend(std::iter::repeat_n(hash, count as usize));
    }
    assert_eq!(done.sheets.len(), 3);
    assert!(done.sheets.iter().all(|sheet| sheet.status == SheetStatus::Done && sheet.attempts == 1));
    assert_eq!(done.sheets.iter().map(|sheet| sheet.source_index).collect::<Vec<_>>(), [0, 1, 1]);
    assert_eq!(done.sheets.iter().map(|sheet| sheet.sheet_id.as_str()).collect::<Vec<_>>(), ["a-KT-01", "b-KT-01", "b-KT-02"]);
    assert_eq!(done.files.len(), 1);
    assert_eq!(done.files[0].pages, 3);
    assert_eq!(renderer.calls.lock().unwrap().len(), 3);
    assert_eq!(*renderer.drawing_hashes.lock().unwrap(), expected_hashes);
}

#[test]
fn folder_job_resumes_with_folder_loader() {
    for without_drawing in [false, true] {
        let dir = TempDir::new();
        let renderer = Arc::new(TestRenderer::default());
        let mut previous = app(7, &dir, renderer.clone());
        let id = enqueue_folder(&mut previous, &dir);
        let job = previous.session.take_queued_job().unwrap();
        assert_eq!(job.job_id, id);
        assert!(is_folder_job(previous.session.job_store_mut().unwrap(), &id).unwrap());
        let store = previous.session.job_store_mut().unwrap();
        let mut runner = JobRunner::open(job, store, 0).unwrap();
        assert_eq!(runner.step(store, &mut FolderLoader::default(), renderer.as_ref(), &SheetBudget::desktop(), 1), StepOutcome::Worked);
        assert_eq!(runner.progress().done, 1);
        assert_eq!(runner.manifest().status, JobStatus::Running);
        let completed = runner.manifest().sheets[0].clone();
        drop(runner);
        drop(previous);
        let mut resumed = app(7, &dir, renderer.clone());
        let open_hash = sha256_hex(cadcraft_io::write_dxf(resumed.session.doc().unwrap()).as_bytes());
        if without_drawing {
            resumed.session.execute("closeall", &json!({})).unwrap();
            assert!(resumed.session.doc().is_err());
        }
        let mut h = harness(resumed);
        h.run_steps(3);
        assert!(h.state().job_pump.interrupted.contains(&id));
        assert_eq!(renderer.calls.lock().unwrap().len(), 1);
        h.get_by_role_and_label(Role::Window, "Plot Jobs").get_by_label("Resume").click();
        h.run_steps(2);
        drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done));
        assert_folder_result(&dir, &id, &renderer);
        assert!(!renderer.drawing_hashes.lock().unwrap().contains(&open_hash));
        let done = manifest(&dir.jobs(), &id);
        assert_eq!(done.sheets[0].sha256, completed.sha256);
        assert_eq!(done.sheets[0].finished_ms, completed.finished_ms);
        assert!(h.state().job_pump.errors.is_empty());
        assert!(!h.state().job_pump.interrupted.contains(&id));
    }
}

#[test]
fn folder_job_queues_without_an_open_drawing() {
    let dir = TempDir::new();
    let renderer = Arc::new(TestRenderer::default());
    let mut app = app(7, &dir, renderer.clone());
    app.session.execute("closeall", &json!({})).unwrap();
    assert!(app.session.doc().is_err());
    let id = enqueue_folder(&mut app, &dir);
    let mut h = harness(app);
    drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done));
    assert_folder_result(&dir, &id, &renderer);
    assert!(h.state().job_pump.errors.is_empty());
}

#[test]
fn drawing_job_still_uses_the_open_drawing_snapshot() {
    for resume in [false, true] {
        let dir = TempDir::new();
        let renderer = Arc::new(TestRenderer::default());
        renderer.hold.store(true, Ordering::Relaxed);
        let mut app = app(2, &dir, renderer.clone());
        let hash = sha256_hex(cadcraft_io::write_dxf(app.session.doc().unwrap()).as_bytes());
        let id = enqueue(&mut app, &dir, "snapshot");
        assert!(!is_folder_job(app.session.job_store_mut().unwrap(), &id).unwrap());
        if resume {
            interrupt(&mut app, &dir, &id);
            app.job_pump = JobPump::with_renderer(dir.jobs(), renderer.clone(), SheetBudget::desktop());
            app.run("ui.ubplot.jobs", json!({"action": "resume", "jobId": id})).unwrap();
        }
        let mut h = harness(app);
        drive_until(&mut h, |_| !renderer.calls.lock().unwrap().is_empty());
        h.state_mut().session.open_drawing(frames::frames_drawing(7), "Different drawing", None);
        renderer.hold.store(false, Ordering::Relaxed);
        drive_until(&mut h, |app| app.job_pump.manifests.get(&id).is_some_and(|manifest| manifest.status == JobStatus::Done));
        assert_eq!(*renderer.drawing_hashes.lock().unwrap(), [hash.clone(), hash]);
        assert_eq!(manifest(&dir.jobs(), &id).sheets.len(), 2);
        assert!(h.state().job_pump.errors.is_empty());
    }
}
