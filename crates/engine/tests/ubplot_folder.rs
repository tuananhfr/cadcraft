#![cfg(not(target_arch = "wasm32"))]

#[allow(dead_code)]
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cadcraft_doc::EntityKind;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_engine::sheetplan::{FrameRegistry, Severity, SheetStatus};
use serde_json::{Value, json};
use support::frames::{CountingRenderer, frames_drawing, install_budget_plot};
use support::pdf_probe::pages;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot-folder-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn input(&self) -> PathBuf {
        let path = self.0.join("input");
        fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_frames(path: &Path, count: usize) {
    let mut drawing = frames_drawing(count);
    let prefix = path.file_stem().unwrap().to_string_lossy();
    let handles: Vec<_> = drawing.model.iter().map(|entity| entity.handle).collect();
    for handle in handles {
        drawing.model.modify(handle, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                for attribute in &mut insert.attribs {
                    if attribute.tag == "SHEET_NO" {
                        attribute.text.value = format!("{prefix}-{}", attribute.text.value);
                    }
                }
            }
        });
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, cadcraft_io::write(&drawing, "fixture.dxf").unwrap()).unwrap();
}

fn new_session() -> Session {
    set_io(IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
        write: |drawing, name| cadcraft_io::write(drawing, name).map_err(|error| error.to_string()),
        plot: None,
    });
    install_budget_plot();
    let mut session = Session::new();
    let mut registry = FrameRegistry::builtin();
    let mut template = registry.templates[0].clone();
    template.id = "test-a3".into();
    template.block_names = vec!["FRAME-A3".into()];
    registry.templates.insert(0, template);
    session.execute("ubplot.templates", &json!({"action": "import", "text": serde_json::to_string(&registry).unwrap()})).unwrap();
    session
}

fn queued(session: &mut Session, root: &Path, out: &Path, options: Value) -> PrintJob {
    let mut params = json!({"folder": root, "out": out, "background": true});
    params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
    let response = session.execute("ubplot.folder", &params).unwrap();
    assert_eq!(response["status"], "QUEUED");
    let job = session.take_queued_job().unwrap();
    assert_eq!(response["jobId"], job.job_id);
    job
}

fn manifest(response: Value) -> PublishManifest {
    serde_json::from_value(response["manifest"].clone()).unwrap()
}

fn finish(session: &mut Session, job: &PrintJob) -> PublishManifest {
    manifest(session.execute("ubplot.jobs", &json!({"action": "resume", "jobId": job.job_id})).unwrap())
}

#[test]
fn folder_scan_bounded_depth_and_count() {
    let temp = TempDir::new();
    let root = temp.input();
    for index in 0..30 {
        write_frames(&root.join(format!("file-{index:02}.dxf")), 1);
    }
    let mut deep = root.clone();
    for _ in 0..20 {
        deep.push("deep");
        fs::create_dir(&deep).unwrap();
    }
    write_frames(&deep.join("last.dxf"), 1);
    let scan = scan_folder(&root, 3, 10, &[]);
    assert_eq!(scan.files.len(), 10);
    assert!(scan.skipped.iter().any(|file| file.reason.contains("max_depth")));
    assert!(scan.skipped.iter().any(|file| file.reason.contains("max_files")));
    assert!(scan.files[0].ends_with("file-00.dxf"));
    assert!(scan.files[9].ends_with("file-09.dxf"));
    let capped = scan_folder(&root, usize::MAX, usize::MAX, &[]);
    assert_eq!(capped.files.len(), 31);
    write_frames(&root.join("Z.DXF"), 1);
    assert!(scan_folder(&root, 32, 5000, &[]).files.last().unwrap().ends_with("Z.DXF"));

    let mut session = new_session();
    for options in [
        json!({"maxDepth": 33}),
        json!({"maxFiles": 5001}),
        json!({"maxFiles": 0}),
        json!({"maxDepth": -1}),
        json!({"pagesPerFile": 0}),
        json!({"pagesPerFile": 257}),
        json!({"recursive": "true"}),
        json!({"layouts": []}),
        json!({"mode": "window"}),
    ] {
        let mut params = json!({"folder": root, "out": temp.0.join("out")});
        params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        assert!(session.execute("ubplot.folder", &params).is_err());
    }
    let job = queued(&mut session, &root, &temp.0.join("out"), json!({"recursive": false, "maxFiles": 10}));
    assert_eq!(job.sources.len(), 10);
    assert!(job.sources.iter().all(|source| Path::new(source.path.as_ref().unwrap()).parent() == Some(root.as_path())));
}

#[test]
fn folder_skips_symlink_loops() {
    let temp = TempDir::new();
    let root = temp.input();
    write_frames(&root.join("valid.dxf"), 1);
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&temp.0, root.join("loop"));
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(&temp.0, root.join("loop"));
    #[cfg(not(any(unix, windows)))]
    let result: std::io::Result<()> = Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "symlink creation is unavailable on this OS"));
    if let Err(error) = result {
        eprintln!("Skipping folder_skips_symlink_loops: OS refused symlink creation: {error}");
        return;
    }
    let scan = scan_folder(&root, 32, 5000, &[]);
    assert_eq!(scan.files.len(), 1);
    assert!(scan.skipped.iter().any(|file| file.path.ends_with("loop") && file.reason.contains("symlink")));
}

#[test]
fn folder_lists_skipped_files_with_reason() {
    let temp = TempDir::new();
    let root = temp.input();
    fs::write(root.join("empty.dwg"), b"").unwrap();
    fs::write(root.join("fake.dxf"), b"\x89PNG\r\n\x1a\n").unwrap();
    write_frames(&root.join("valid.DXF"), 1);
    let scan = scan_folder(&root, 16, 1000, &[]);
    assert_eq!(scan.skipped.len(), 2);
    assert!(scan.skipped.iter().any(|file| file.path.ends_with("empty.dwg") && file.reason.contains("empty")));
    assert!(scan.skipped.iter().any(|file| file.path.ends_with("fake.dxf") && file.reason.contains("invalid_magic")));
    let mut session = new_session();
    let job = queued(&mut session, &root, &temp.0.join("out"), json!({}));
    let done = finish(&mut session, &job);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.skipped_files.len(), 2);
    assert_eq!(done.sheets.len(), 1);

    write_frames(&root.join("no-frames.dxf"), 0);
    let job = queued(&mut session, &root, &temp.0.join("out-empty"), json!({}));
    let done = finish(&mut session, &job);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.sources.len(), 2);
    let empty = done.sources.iter().find(|source| source.sheets == 0).unwrap();
    assert!(empty.issues.iter().any(|issue| issue.code == "NO_FRAMES_FOUND" && issue.severity == Severity::Warn));

    let mut session = new_session();
    session.execute("ubplot.templates", &json!({"action": "reset"})).unwrap();
    let job = queued(&mut session, &root, &temp.0.join("out-generic"), json!({}));
    assert!(job.sources.iter().all(|source| source.plan.sheets.is_empty()));
    assert_eq!(finish(&mut session, &job).status, JobStatus::Done);
}

#[test]
fn folder_one_manifest_order_by_path_then_sheet() {
    let temp = TempDir::new();
    let root = temp.input();
    write_frames(&root.join("b/x.dxf"), 2);
    write_frames(&root.join("a/y.dxf"), 3);
    let mut session = new_session();
    session.execute("closeall", &json!({})).unwrap();
    assert!(session.doc().is_err());
    let job = queued(&mut session, &root, &temp.0.join("out"), json!({}));
    assert!(job.sources[0].path.as_ref().unwrap().replace('\\', "/").ends_with("a/y.dxf"));
    assert!(job.sources[1].path.as_ref().unwrap().replace('\\', "/").ends_with("b/x.dxf"));
    let done = finish(&mut session, &job);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.sheets.len(), 5);
    assert_eq!(done.files.len(), 1);
    assert_eq!(done.sheets.iter().map(|sheet| sheet.source_index).collect::<Vec<_>>(), [0, 0, 0, 1, 1]);
    assert_eq!(done.sheets.iter().map(|sheet| sheet.order).collect::<Vec<_>>(), [1, 2, 3, 1, 2]);
    assert_eq!(done.sheets.iter().map(|sheet| sheet.page_index.unwrap()).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(session.job_store_mut().unwrap().list_jobs().len(), 1);
    let id = done.job_id.clone();
    assert_eq!(finish(&mut session, &job), done);
    assert_eq!(session.job_store_mut().unwrap().list_jobs(), [id]);

    let bytes = cadcraft_io::write(&frames_drawing(2), "fixture.dxf").unwrap();
    fs::write(root.join("a/y.dxf"), &bytes).unwrap();
    fs::write(root.join("b/x.dxf"), &bytes).unwrap();
    let job = queued(&mut session, &root, &temp.0.join("out-split"), json!({"mode": "split"}));
    let done = finish(&mut session, &job);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.files.len(), 4);
    let names: std::collections::BTreeSet<_> = done.files.iter().map(|file| &file.name).collect();
    assert_eq!(names.len(), 4);
}

#[test]
fn folder_never_writes_sources() {
    let temp = TempDir::new();
    let root = temp.input();
    let out = root.join("out");
    let paths = [root.join("a.dxf"), root.join("sub/b.DWG")];
    for path in &paths {
        write_frames(path, 2);
    }
    write_frames(&out.join("decoy.dxf"), 5);
    let before: Vec<_> = paths.iter().map(|path| (sha256_hex(&fs::read(path).unwrap()), fs::metadata(path).unwrap().modified().unwrap())).collect();
    let mut session = new_session();
    session.set_job_store(Box::new(DirJobStore::new(temp.0.join("store"))));
    let response = session.execute("ubplot.folder", &json!({"folder": root, "out": out})).unwrap();
    let done = manifest(response);
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.sources.len(), 2);
    assert_eq!(done.sheets.len(), 4);
    let after: Vec<_> = paths.iter().map(|path| (sha256_hex(&fs::read(path).unwrap()), fs::metadata(path).unwrap().modified().unwrap())).collect();
    assert_eq!(before, after);
    let pdf = fs::read(out.join(&done.files[0].name)).unwrap();
    assert_eq!(sha256_hex(&pdf), done.files[0].sha256);
    assert_eq!(pages(&pdf).iter().map(|page| page.label.as_deref().unwrap()).collect::<Vec<_>>(), ["a-KT-01", "a-KT-02", "b-KT-01", "b-KT-02"]);
    let scan = scan_folder(&root, 16, 1000, &[out]);
    assert_eq!(scan.files.len(), 2);
    assert!(scan.files.iter().all(|path| !path.contains("decoy")));
}

#[test]
fn folder_source_changed_fails_that_file() {
    let temp = TempDir::new();
    let root = temp.input();
    write_frames(&root.join("a.dxf"), 3);
    write_frames(&root.join("b.dxf"), 3);
    let mut session = new_session();
    let job = queued(&mut session, &root, &temp.0.join("out"), json!({"pagesPerFile": 3}));
    let renderer = CountingRenderer::new();
    let budget = SheetBudget::desktop();
    let mut loader = FolderLoader::default();
    let store = session.job_store_mut().unwrap();
    let mut runner = JobRunner::open(job.clone(), store, 0).unwrap();
    assert_eq!(runner.step(store, &mut loader, &renderer, &budget, 1), StepOutcome::Worked);
    assert_eq!(runner.progress().done, 1);
    drop(runner);
    write_frames(&root.join("a.dxf"), 4);
    assert!(loader.load(0, &job.sources[0]).unwrap_err().contains("SOURCE_CHANGED"));
    let done = finish(&mut session, &job);
    assert_eq!(done.status, JobStatus::Failed);
    let first: Vec<_> = done.sheets.iter().filter(|sheet| sheet.source_index == 0).collect();
    assert_eq!(first[0].status, SheetStatus::Done);
    assert!(first.iter().skip(1).all(|sheet| sheet.status == SheetStatus::Failed && sheet.issues.iter().any(|issue| issue.code == "SOURCE_CHANGED")));
    assert!(done.sources[0].issues.iter().any(|issue| issue.code == "SOURCE_CHANGED"));
    assert!(done.sheets.iter().filter(|sheet| sheet.source_index == 1).all(|sheet| sheet.status == SheetStatus::Done));
    assert_eq!(done.files.len(), 1);
    assert!(done.issues.iter().any(|issue| issue.code == "FILE_WITHHELD"));
    let mut loader = FolderLoader::default();
    fs::write(root.join("a.dxf"), b"changed to invalid content").unwrap();
    assert!(loader.load(0, &job.sources[0]).unwrap_err().contains("SOURCE_CHANGED"));
    fs::remove_file(root.join("a.dxf")).unwrap();
    assert!(loader.load(0, &job.sources[0]).unwrap_err().contains("SOURCE_UNAVAILABLE"));
}

#[test]
fn sp06_small_folder_resume() {
    let temp = TempDir::new();
    let root = temp.input();
    for index in 0..20 {
        write_frames(&root.join(format!("file-{index:02}.dxf")), 5);
    }
    let mut session = new_session();
    let job = queued(&mut session, &root, &temp.0.join("out"), json!({"pagesPerFile": 16}));
    let renderer = CountingRenderer::new();
    let budget = SheetBudget::desktop();
    let mut loader = FolderLoader::default();
    let store = session.job_store_mut().unwrap();
    let mut runner = JobRunner::open(job.clone(), store, 0).unwrap();
    for step in 1..=37 {
        assert_eq!(runner.step(store, &mut loader, &renderer, &budget, step), StepOutcome::Worked);
    }
    assert_eq!(renderer.total(), runner.progress().done);
    assert!(runner.progress().done < 100);
    drop(runner);
    let done = run_to_end(job, store, &mut FolderLoader::default(), &renderer, &budget, &|| 100).unwrap();
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.sheets.len(), 100);
    assert!(done.sheets.iter().all(|sheet| sheet.status == SheetStatus::Done && sheet.attempts == 1));
    assert_eq!(renderer.total(), 100);
    assert_eq!(renderer.counts.borrow().len(), 100);
    assert!(renderer.counts.borrow().values().all(|count| *count == 1));
    assert_eq!(done.files.len(), 7);
    for file in &done.files {
        let indexes: Vec<_> = done
            .sheets
            .iter()
            .filter(|sheet| sheet.output_file.as_deref() == Some(file.name.as_str()))
            .map(|sheet| sheet.page_index.unwrap())
            .collect();
        assert_eq!(indexes, (1..=file.pages).collect::<Vec<_>>());
    }
}
