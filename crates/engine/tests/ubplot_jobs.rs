#[allow(dead_code)]
mod support;

use std::cell::RefCell;
use std::rc::Rc;

use cadcraft_doc::Drawing;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_engine::sheetplan::{Severity, SheetStatus, codes::issue};
use cadcraft_engine::{EngineError, Session};
use serde_json::{Value, json};
use support::frames::{frames_drawing, install_budget_plot};
use support::pdf_probe::pages;

#[derive(Default)]
struct CapturedStore {
    inner: MemoryJobStore,
    delivered: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl JobStore for CapturedStore {
    fn list_jobs(&self) -> Vec<String> {
        self.inner.list_jobs()
    }

    fn read_manifest(&self, id: &str) -> Result<Option<String>, String> {
        self.inner.read_manifest(id)
    }

    fn commit(&mut self, id: &str, artifacts: &[(String, Vec<u8>)], manifest: &str) -> Result<(), String> {
        self.inner.commit(id, artifacts, manifest)
    }

    fn read_artifact(&self, id: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.inner.read_artifact(id, name)
    }

    fn delete_artifacts(&mut self, id: &str, names: &[String]) -> Result<(), String> {
        self.inner.delete_artifacts(id, names)
    }

    fn deliver(&mut self, job: &PrintJob, name: &str, bytes: &[u8]) -> Result<String, String> {
        let target = self.inner.deliver(job, name, bytes)?;
        self.delivered.borrow_mut().push(bytes.to_vec());
        Ok(target)
    }

    fn remove_job(&mut self, id: &str) -> Result<(), String> {
        self.inner.remove_job(id)
    }
}

fn session(drawing: Drawing) -> Session {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|error| error.to_string())),
    });
    install_budget_plot();
    let mut s = Session::empty();
    s.open_drawing(drawing, "frames", None);
    s
}

fn scan(s: &mut Session) -> Value {
    let handles: Vec<_> = s.doc().unwrap().model.iter().map(|entity| entity.handle.hex()).collect();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap()
}

fn enqueue(s: &mut Session) -> String {
    let value = s.execute("ubplot.publish", &json!({"background": true})).unwrap();
    assert_eq!(value["status"], "QUEUED");
    value["jobId"].as_str().unwrap().to_string()
}

#[test]
fn queued_job_resumes_to_done_with_manifest() {
    let mut s = session(frames_drawing(3));
    let store = CapturedStore::default();
    let delivered = store.delivered.clone();
    s.set_job_store(Box::new(store));
    let plan = scan(&mut s);
    let queued = s.execute("ubplot.publish", &json!({"background": true, "plan": plan})).unwrap();
    let result = s.execute("ubplot.jobs", &json!({"action": "resume", "jobId": queued["jobId"]})).unwrap();
    assert_eq!(result["manifest"]["status"], "DONE");
    assert_eq!(result["manifest"]["files"].as_array().unwrap().len(), 1);
    assert_eq!(result["manifest"]["files"][0]["pages"], 3);
    assert_eq!(delivered.borrow().len(), 1);
    assert_eq!(pages(&delivered.borrow()[0]).len(), 3);
    assert!(s.take_queued_job().is_none());
    assert!(s.running.is_none());
    let again = s.execute("ubplot.jobs", &json!({"action": "resume", "jobId": queued["jobId"]})).unwrap();
    assert_eq!(again, result);
    assert_eq!(delivered.borrow().len(), 1);
}

#[test]
fn jobs_renderer_uses_profile_table() {
    use cadcraft_engine::sheetplan::profile::{PRINT_PROFILE_SCHEMA, ProfileDoc, ProfileLevel};
    use support::pdf_probe::{colour_ops, streams_text};
    let mut drawing = frames_drawing(2);
    let frames: Vec<_> = drawing.model.iter().map(|entity| entity.handle.hex()).collect();
    let common = cadcraft_doc::Common { color: cadcraft_color::Color::Index(1), ..Default::default() };
    let line = cadcraft_doc::Line { a: cadcraft_geom::Vec3::new(20.0, 20.0, 0.0), b: cadcraft_geom::Vec3::new(400.0, 200.0, 0.0) };
    drawing.add(&cadcraft_doc::Space::Model, common, cadcraft_doc::EntityKind::Line(line)).unwrap();
    let mut s = session(drawing);
    let store = CapturedStore::default();
    let delivered = store.delivered.clone();
    s.set_job_store(Box::new(store));
    let user = ProfileDoc {
        schema_version: PRINT_PROFILE_SCHEMA,
        level: ProfileLevel::User,
        id: "default".into(),
        project_id: None,
        revision: 1,
        settings: json!({"plot_style_table": "monochrome.ctb", "lineweights": false}),
        locks: Vec::new(),
        updated_ms: 1,
    };
    s.write_print_profile(&user).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": frames})).unwrap();
    let queued = s.execute("ubplot.publish", &json!({"background": true})).unwrap();
    let job = s.take_queued_job().unwrap();
    assert_eq!(job.plot_style_table.as_deref(), Some("monochrome.ctb"));
    assert!(!job.lineweights);
    assert_eq!(PdfSheetRenderer::for_job(&job), PdfSheetRenderer { plot_style_table: Some("monochrome.ctb".into()), lineweights: false });
    let result = s.execute("ubplot.jobs", &json!({"action": "resume", "jobId": queued["jobId"]})).unwrap();
    assert_eq!(result["manifest"]["status"], "DONE");
    let content = streams_text(&delivered.borrow()[0]);
    assert!(colour_ops(&content).is_empty(), "{:?}", colour_ops(&content));
    assert!(content.contains("0 0 0 RG"));

    // Without a table a job keeps its colours and its definition omits the default fields.
    s.write_print_profile(&ProfileDoc { settings: json!({}), revision: 2, ..user }).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": frames})).unwrap();
    s.execute("ubplot.publish", &json!({"background": true})).unwrap();
    let job = s.take_queued_job().unwrap();
    assert_eq!(PdfSheetRenderer::for_job(&job), PdfSheetRenderer::default());
    let definition = serde_json::to_value(&job).unwrap();
    assert!(definition.get("plot_style_table").is_none() && definition.get("lineweights").is_none(), "{definition}");
}

#[test]
fn background_publish_queues_and_host_takes_it() {
    let mut s = session(frames_drawing(3));
    scan(&mut s);
    let id = enqueue(&mut s);
    let manifest = s.execute("ubplot.jobs", &json!({"action": "manifest", "jobId": id})).unwrap();
    assert_eq!(manifest["manifest"]["status"], "QUEUED");
    assert!(
        manifest["manifest"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "JOBS_NOT_PERSISTED" && issue["severity"] == "INFO" })
    );
    let job = s.take_queued_job().unwrap();
    assert_eq!(job.job_id, id);
    assert_eq!(job.sources[0].plan.sheets.len(), 3);
    assert!(s.take_queued_job().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
struct TempDir(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl TempDir {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot-jobs-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn jobs_resume_after_new_session() {
    use std::sync::Arc;
    use support::frames::CountingRenderer;

    let temp = TempDir::new();
    let drawing = frames_drawing(5);
    let mut first = session(drawing.clone());
    first.set_job_store(Box::new(DirJobStore::new(temp.0.join("jobs"))));
    scan(&mut first);
    let queued = first.execute("ubplot.publish", &json!({"background": true, "path": temp.0.join("out.pdf")})).unwrap();
    let job = first.take_queued_job().unwrap();
    let id = job.job_id.clone();
    assert_eq!(queued["jobId"], id);
    let mut loader = SnapshotLoader(Arc::new(drawing.clone()), job.sources[0].source_hash.clone());
    let renderer = CountingRenderer::new();
    let store = first.job_store_mut().unwrap();
    let mut runner = JobRunner::open(job, store, 0).unwrap();
    for _ in 0..2 {
        assert_eq!(runner.step(store, &mut loader, &renderer, &SheetBudget::desktop(), 0), StepOutcome::Worked);
    }
    assert_eq!(renderer.total(), 2);
    drop(runner);
    drop(first);

    let mut second = session(drawing);
    second.set_job_store(Box::new(DirJobStore::new(temp.0.join("jobs"))));
    let list = second.execute("ubplot.jobs", &json!({"action": "list"})).unwrap();
    assert_eq!(list[0]["jobId"], id);
    assert_eq!(list[0]["status"], "RUNNING");
    assert_eq!(list[0]["done"], 2);
    let result = second.execute("ubplot.jobs", &json!({"action": "resume", "jobId": id})).unwrap();
    let manifest: PublishManifest = serde_json::from_value(result["manifest"].clone()).unwrap();
    assert_eq!(manifest.status, JobStatus::Done);
    assert_eq!(manifest.sheets.iter().map(|sheet| u32::from(sheet.attempts)).sum::<u32>(), 5);
    assert!(manifest.issues.iter().all(|issue| issue.code != "JOBS_NOT_PERSISTED"));
    assert_eq!(pages(&std::fs::read(temp.0.join("out.pdf")).unwrap()).len(), 5);
}

#[test]
fn background_publish_refuses_blocked_plan() {
    for blocked in ["status", "sheet issue", "plan issue"] {
        let mut s = session(frames_drawing(3));
        scan(&mut s);
        let plan = &mut s.ubplot.last.as_mut().unwrap().plan;
        match blocked {
            "status" => plan.sheets[0].status = SheetStatus::Blocked,
            "sheet issue" => {
                plan.sheets[0].issues.push(issue(Severity::Block, "TEST_BLOCK", "Blocked synthetic sheet.", Some("KT-01")));
            }
            _ => plan.issues.push(issue(Severity::Block, "TEST_BLOCK", "Blocked synthetic plan.", None)),
        }
        let error = s.execute("ubplot.publish", &json!({"background": true})).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { .. }));
        assert!(error.to_string().contains("PLAN_HAS_BLOCKED"));
        assert!(s.take_queued_job().is_none());
        assert_eq!(s.execute("ubplot.jobs", &json!({"action": "list"})).unwrap(), json!([]));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn output_is_source_refused() {
    let mut s = session(frames_drawing(1));
    s.state_mut().unwrap().path = Some("a.dxf".into());
    scan(&mut s);
    for background in [false, true] {
        let error = s.execute("ubplot.publish", &json!({"background": background, "path": "a.dxf"})).unwrap_err();
        assert!(error.to_string().contains("OUTPUT_IS_SOURCE"));
    }
    assert!(s.take_queued_job().is_none());
}

#[test]
fn hostile_jobs_params_never_panic() {
    let mut s = session(frames_drawing(1));
    scan(&mut s);
    let cases = [
        ("ubplot.jobs", json!({"action": "unknown"})),
        ("ubplot.jobs", json!({"action": "status", "jobId": "x".repeat(10_000)})),
        ("ubplot.jobs", json!({"action": "manifest", "jobId": "../escape"})),
        ("ubplot.jobs", json!({"action": "resume", "jobId": 42})),
        ("ubplot.jobs", json!({"action": "cancel"})),
        ("ubplot.jobs", json!({"action": false})),
        ("ubplot.jobs", json!([])),
        ("ubplot.publish", json!({"background": true, "pagesPerFile": -1})),
        ("ubplot.publish", json!({"background": true, "pagesPerFile": 0})),
        ("ubplot.publish", json!({"background": true, "pagesPerFile": 1001})),
        ("ubplot.publish", json!({"background": true, "retryLimit": 6})),
        ("ubplot.publish", json!({"background": true, "retryLimit": -1})),
        ("ubplot.publish", json!({"background": true, "mode": "other"})),
        ("ubplot.publish", json!({"background": true, "plan": "not a plan"})),
        ("ubplot.publish", json!({"background": "yes"})),
    ];
    for (command, params) in cases {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute(command, &params)));
        assert!(matches!(outcome, Ok(Err(EngineError::BadParams { .. }))), "{command} {params}: {outcome:?}");
    }
    assert!(s.take_queued_job().is_none());
}

#[test]
fn jobs_cancel_remove_and_status() {
    let mut s = session(frames_drawing(2));
    scan(&mut s);
    let id = enqueue(&mut s);
    let status = s.execute("ubplot.jobs", &json!({"action": "status", "jobId": id})).unwrap();
    assert_eq!(status["total"], 2);
    assert_eq!(status["done"], 0);
    s.execute("ubplot.jobs", &json!({"action": "cancel", "jobId": id})).unwrap();
    assert_eq!(s.take_cancel_requests(), std::slice::from_ref(&id));
    assert!(s.take_cancel_requests().is_empty());
    assert!(s.take_queued_job().is_none());
    let manifest = s.execute("ubplot.jobs", &json!({"action": "manifest", "jobId": id})).unwrap();
    assert_eq!(manifest["manifest"]["status"], "CANCELLED");
    s.execute("ubplot.jobs", &json!({"action": "remove", "jobId": id})).unwrap();
    assert_eq!(s.execute("ubplot.jobs", &json!({"action": "list"})).unwrap(), json!([]));
    assert!(s.execute("ubplot.jobs", &json!({"action": "status", "jobId": id})).is_err());
}

#[test]
fn running_job_cannot_be_removed() {
    let mut s = session(frames_drawing(1));
    scan(&mut s);
    let id = enqueue(&mut s);
    let store = s.job_store_mut().unwrap();
    let mut manifest = PublishManifest::from_json(&store.read_manifest(&id).unwrap().unwrap()).unwrap();
    manifest.status = JobStatus::Running;
    store.commit(&id, &[], &manifest.to_json()).unwrap();
    let error = s.execute("ubplot.jobs", &json!({"action": "remove", "jobId": id})).unwrap_err();
    assert!(matches!(error, EngineError::BadParams { .. }));
    assert!(error.to_string().contains("RUNNING"));
    s.execute("ubplot.jobs", &json!({"action": "cancel", "jobId": id})).unwrap();
    assert_eq!(s.take_cancel_requests(), [id]);
}

#[test]
fn queued_split_and_page_groups_are_delivered() {
    for (mode, cap, expected) in [("merged", 2, vec![2, 1]), ("split", 3, vec![1, 1, 1])] {
        let mut s = session(frames_drawing(3));
        let store = CapturedStore::default();
        let delivered = store.delivered.clone();
        s.set_job_store(Box::new(store));
        scan(&mut s);
        let queued = s.execute("ubplot.publish", &json!({"background": true, "mode": mode, "pagesPerFile": cap, "retryLimit": 0})).unwrap();
        let result = s.execute("ubplot.jobs", &json!({"action": "resume", "jobId": queued["jobId"]})).unwrap();
        assert_eq!(result["manifest"]["status"], "DONE");
        assert_eq!(delivered.borrow().iter().map(|bytes| pages(bytes).len()).collect::<Vec<_>>(), expected);
    }
}

#[test]
fn failed_enqueue_does_not_reach_host_queue() {
    let mut s = session(frames_drawing(1));
    let mut store = MemoryJobStore::default();
    store.fail_writes = true;
    s.set_job_store(Box::new(store));
    scan(&mut s);
    assert!(s.execute("ubplot.publish", &json!({"background": true})).is_err());
    assert!(s.take_queued_job().is_none());
}

#[test]
fn synchronous_publish_retains_legacy_result() {
    let mut s = session(frames_drawing(1));
    scan(&mut s);
    let result = s.execute("ubplot.publish", &json!({"background": false})).unwrap();
    assert_eq!(result.as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ["failed", "files", "manifest", "plan"]);
    assert!(result["files"][0]["data"].is_string());
    assert!(s.take_queued_job().is_none());
    assert!(s.job_store_mut().is_none());
}

#[test]
fn repeated_publish_keeps_progress_and_rejects_different_plan() {
    let mut s = session(frames_drawing(3));
    scan(&mut s);
    let id = enqueue(&mut s);
    assert_eq!(enqueue(&mut s), id);
    assert_eq!(s.take_queued_job().unwrap().job_id, id);
    assert!(s.take_queued_job().is_none());
    s.ubplot.last.as_mut().unwrap().plan.sheets.pop();
    let error = s.execute("ubplot.publish", &json!({"background": true})).unwrap_err();
    assert!(error.to_string().contains("different plan"));
    let status = s.execute("ubplot.jobs", &json!({"action": "status", "jobId": id})).unwrap();
    assert_eq!(status["total"], 3);
}

#[test]
fn jobs_resume_changed_drawing_fails_pending_sheets() {
    let mut s = session(frames_drawing(3));
    scan(&mut s);
    let id = enqueue(&mut s);
    s.execute("line", &json!({"points": [[0, 0], [1, 1]]})).unwrap();
    let result = s.execute("ubplot.jobs", &json!({"action": "resume", "jobId": id})).unwrap();
    assert_eq!(result["manifest"]["status"], "FAILED");
    assert!(result["manifest"]["files"].as_array().unwrap().is_empty());
    for sheet in result["manifest"]["sheets"].as_array().unwrap() {
        assert_eq!(sheet["status"], "FAILED");
        assert_eq!(sheet["attempts"], 0);
        assert!(sheet["issues"].as_array().unwrap().iter().any(|issue| issue["code"] == "SOURCE_CHANGED"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod printers {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cadcraft_engine::cmd::plotters::{PlotterStore, find_device, set_plotter_store};
    use cadcraft_engine::sheetplan::device::{PrintOutcome, PrinterHost, PrinterInfo, set_printer_host};
    use cadcraft_engine::sheetplan::{SheetPlan, codes};

    use super::*;

    static SPOOLED: AtomicUsize = AtomicUsize::new(0);
    static MISDIRECTED: AtomicUsize = AtomicUsize::new(0);

    fn setup() {
        static HOST: OnceLock<()> = OnceLock::new();
        HOST.get_or_init(|| {
            set_plotter_store(PlotterStore::memory());
            set_printer_host(PrinterHost {
                list: || {
                    Ok(["Office A3", "Bare", "Offline", "Handlerless", "Spooler", "CADCraft PDF", "File wins"]
                        .into_iter()
                        .map(|name| PrinterInfo {
                            name: name.into(),
                            is_default: name == "Spooler",
                            offline: name == "Offline",
                            papers: if name == "Bare" { Vec::new() } else { vec![("A3".into(), 297.0, 420.0)] },
                        })
                        .collect())
                },
                print_pdf: |printer, path| {
                    assert!(Path::new(path).is_file());
                    assert!(std::fs::read(path).unwrap().starts_with(b"%PDF-"));
                    match printer {
                        "Spooler" => {
                            SPOOLED.fetch_add(1, Ordering::Relaxed);
                            PrintOutcome::Spooled { printer: printer.into() }
                        }
                        "Handlerless" => PrintOutcome::NotSupported("No PDF PrintTo handler.".into()),
                        _ => {
                            MISDIRECTED.fetch_add(1, Ordering::Relaxed);
                            PrintOutcome::NotSupported("Unexpected printer submission.".into())
                        }
                    }
                },
            });
        });
    }

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!("cadcraft-printers-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn window(s: &mut Session, width: f64, height: f64, device: &str) -> SheetPlan {
        s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
        serde_json::from_value(
            s.execute(
                "ubplot.scan",
                &json!({"mode": "window", "windows": [[[0, 0], [width, height]]],
                    "paper": format!("{width}x{height}"), "scale": "1:1", "device": device}),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn approve_warnings(s: &mut Session, plan: &SheetPlan) {
        let approvals: Vec<_> = plan
            .sheets
            .iter()
            .flat_map(|sheet| {
                sheet
                    .issues
                    .iter()
                    .filter(|issue| issue.severity == Severity::Warn)
                    .map(|issue| json!({"sheetKey": format!("{}@{}", sheet.sheet_id, sheet.order), "code": issue.code}))
            })
            .collect();
        s.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap();
    }

    #[test]
    fn system_printer_limits_use_existing_device_checks() {
        setup();
        let mut s = session(frames_drawing(1));
        let oversized = window(&mut s, 594.0, 841.0, "Office A3");
        assert!(oversized.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_EXCEEDED && issue.severity == Severity::Block));
        assert_eq!(oversized.sheets[0].status, SheetStatus::Blocked);
        let error = s.execute("ubplot.publish", &json!({"background": true})).unwrap_err();
        assert!(error.to_string().contains("PLAN_HAS_BLOCKED"));
        assert!(s.take_queued_job().is_none());
        let a3 = window(&mut s, 297.0, 420.0, "Office A3");
        assert!(!a3.sheets[0].issues.iter().any(|issue| issue.severity == Severity::Block));
        assert!(a3.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_LIMIT_NEAR));
        approve_warnings(&mut s, &a3);
        enqueue(&mut s);
        assert_eq!(s.take_queued_job().unwrap().printer.as_deref(), Some("Office A3"));
        let bare = window(&mut s, 210.0, 297.0, "Bare");
        assert!(bare.sheets[0].issues.iter().any(|issue| issue.code == codes::DEVICE_UNAVAILABLE && issue.severity == Severity::Warn));
        assert!(!bare.sheets[0].issues.iter().any(|issue| issue.severity == Severity::Block));
        let device = find_device("Office A3").unwrap();
        assert_eq!(device.default_paper, "A3");
        assert_eq!(device.papers[0].margins, [0.0; 4]);
        assert_eq!(device.output, cadcraft_doc::PlotOutput::Pdf);
    }

    #[test]
    fn plotter_file_wins_over_same_named_printer() {
        setup();
        let mut s = session(frames_drawing(1));
        s.execute("plottermanager", &json!({"action": "new", "name": "File wins", "output": "png"})).unwrap();
        assert_eq!(find_device("File wins").unwrap().output, cadcraft_doc::PlotOutput::Png);
        assert!(find_device("CADCraft PDF").unwrap().papers.len() > 1);
        window(&mut s, 210.0, 297.0, "File wins");
        enqueue(&mut s);
        assert!(s.take_queued_job().unwrap().printer.is_none());
    }

    fn run_printer_job(printer: &str) -> (Temp, PublishManifest, PrintJob) {
        setup();
        let temp = Temp::new();
        let mut s = session(frames_drawing(1));
        s.set_job_store(Box::new(DirJobStore::new(temp.0.join("jobs"))));
        if printer == "Gone" {
            scan(&mut s);
        } else {
            let plan = window(&mut s, 210.0, 297.0, printer);
            approve_warnings(&mut s, &plan);
        }
        let path = temp.0.join("printed.pdf").to_string_lossy().into_owned();
        s.execute("ubplot.publish", &json!({"background": true, "path": path})).unwrap();
        let mut job = s.take_queued_job().unwrap();
        job.printer = Some(printer.into());
        let profile = s.execute("ubplot.profile", &json!({"action": "effective"})).unwrap();
        let mut loader = SnapshotLoader(std::sync::Arc::new(s.doc().unwrap().clone()), job.sources[0].source_hash.clone());
        let manifest =
            run_to_end(job.clone(), s.job_store_mut().unwrap(), &mut loader, &PdfSheetRenderer::default(), &SheetBudget::desktop(), &|| 1).unwrap();
        assert_eq!(s.execute("ubplot.profile", &json!({"action": "effective"})).unwrap(), profile);
        assert_eq!(manifest.status, JobStatus::Done);
        assert!(temp.0.join("printed.pdf").is_file());
        assert!(std::fs::read(temp.0.join("printed.pdf")).unwrap().starts_with(b"%PDF-"));
        (temp, manifest, job)
    }

    #[test]
    fn missing_printer_falls_back_to_pdf_with_warn() {
        for printer in ["Gone", "Offline"] {
            let (temp, manifest, _) = run_printer_job(printer);
            assert!(manifest.issues.iter().any(|issue| issue.code == codes::PRINTER_MISSING && issue.severity == Severity::Warn));
            assert_eq!(manifest.files[0].delivered_to, temp.0.join("printed.pdf").to_string_lossy());
        }
        assert_eq!(MISDIRECTED.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn missing_handler_keeps_pdf_with_warn() {
        let (temp, manifest, _) = run_printer_job("Handlerless");
        assert!(manifest.issues.iter().any(|issue| issue.code == codes::PRINTER_HANDLER_MISSING && issue.severity == Severity::Warn));
        assert_eq!(manifest.files[0].delivered_to, temp.0.join("printed.pdf").to_string_lossy());
    }

    #[test]
    fn printer_without_papers_keeps_pdf() {
        let (temp, manifest, _) = run_printer_job("Bare");
        assert!(manifest.issues.iter().any(|issue| issue.code == codes::DEVICE_UNAVAILABLE && issue.severity == Severity::Warn));
        assert_eq!(manifest.files[0].delivered_to, temp.0.join("printed.pdf").to_string_lossy());
        assert_eq!(MISDIRECTED.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn runner_rechecks_system_printer_limits_before_rendering() {
        setup();
        let mut s = session(frames_drawing(1));
        let plan = window(&mut s, 594.0, 841.0, "CADCraft PDF");
        let mut job =
            PrintJob::new(vec![JobSource { path: None, source_hash: plan.source_hash.clone(), plan }], OutputSpec::default(), DEFAULT_RETRY_LIMIT)
                .unwrap();
        job.printer = Some("Office A3".into());
        let error = JobRunner::open(job, &mut MemoryJobStore::default(), 0).err().unwrap();
        assert!(error.contains("PLAN_HAS_BLOCKED"));
        assert_eq!(MISDIRECTED.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn printer_delivery_records_destination_and_resume_skips_submission() {
        let (temp, manifest, job) = run_printer_job("Spooler");
        assert_eq!(manifest.files[0].delivered_to, "printer:Spooler");
        assert_eq!(SPOOLED.load(Ordering::Relaxed), 1);
        let mut s = session(frames_drawing(1));
        scan(&mut s);
        let mut store = DirJobStore::new(temp.0.join("jobs"));
        let mut loader = SnapshotLoader(std::sync::Arc::new(s.doc().unwrap().clone()), manifest.source_hash.clone());
        let resumed = run_to_end(job, &mut store, &mut loader, &PdfSheetRenderer::default(), &SheetBudget::desktop(), &|| 2).unwrap();
        assert_eq!(resumed, manifest);
        assert_eq!(SPOOLED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn printers_command_lists_host_and_validates_refresh() {
        setup();
        let mut s = session(frames_drawing(1));
        for params in [json!({}), json!({"refresh": true})] {
            let printers = s.execute("ubplot.printers", &params).unwrap();
            assert!(printers.as_array().unwrap().iter().any(|printer| printer["name"] == "Office A3"));
        }
        assert!(s.execute("ubplot.printers", &json!({"refresh": "yes"})).is_err());
    }
}
