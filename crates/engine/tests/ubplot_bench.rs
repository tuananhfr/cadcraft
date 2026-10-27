#![cfg(not(target_arch = "wasm32"))]

#[allow(dead_code)]
mod support;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use cadcraft_doc::{Drawing, Entity, EntityKind, Line};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::ids::sheet_key;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_engine::sheetplan::{FrameRegistry, ScanRequest, Sheet, SheetStatus, build_plan};
use cadcraft_geom::Vec3;
use serde_json::json;
use support::frames::{frames_drawing, install_budget_plot, text};
use support::pdf_probe::pages;

// Both ignored tests may be selected together; concurrent runs would distort RSS and timings.
static BENCH_LOCK: Mutex<()> = Mutex::new(());

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot-bench-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        let mut temp = Self(path);
        temp.0 = fs::canonicalize(&temp.0).unwrap();
        temp
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install_hooks() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        set_io(IoHooks {
            read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
            write: |drawing, name| cadcraft_io::write(drawing, name).map_err(|error| error.to_string()),
            plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
        });
        install_budget_plot();
    });
}

fn benchmark_drawing(count: usize, prefix: &str) -> Drawing {
    let mut drawing = frames_drawing(count);
    let mut block = (**drawing.block("FRAME-A3").unwrap()).clone();
    let texts: Vec<_> = block.entities.iter().filter(|entity| matches!(entity.kind, EntityKind::Text(_))).map(|entity| entity.handle).collect();
    for handle in texts {
        block.entities.remove(handle);
    }
    for index in 0..200 {
        let y = 20.0 + f64::from(index);
        let line = Line { a: Vec3::new(20.0, y, 0.0), b: Vec3::new(400.0, y, 0.0) };
        block.entities.push(Entity::new(drawing.new_handle(), EntityKind::Line(line)));
    }
    for index in 0..20 {
        let mut label = text(&format!("BENCH-{index:02}"));
        label.insert = Vec3::new(20.0, 230.0 + f64::from(index) * 2.5, 0.0);
        block.entities.push(Entity::new(drawing.new_handle(), EntityKind::Text(label)));
    }
    drawing.blocks.insert("FRAME-A3".into(), Arc::new(block));
    // Renderer callbacks lack a source index, so folder sheet IDs must be globally unique.
    let handles: Vec<_> = drawing.model.iter().map(|entity| entity.handle).collect();
    for handle in handles {
        drawing.model.modify(handle, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                for attribute in &mut insert.attribs {
                    if attribute.tag == "SHEET_NO" {
                        attribute.text.value = format!("{prefix}{}", attribute.text.value);
                    }
                }
            }
        });
    }
    drawing
}

#[derive(Default)]
struct TimedRenderer {
    counts: RefCell<BTreeMap<String, usize>>,
    render_times: RefCell<Vec<Duration>>,
    merge_times: RefCell<Vec<Duration>>,
}

impl SheetRenderer for TimedRenderer {
    fn render(&self, drawing: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        *self.counts.borrow_mut().entry(sheet_key(sheet)).or_default() += 1;
        let started = Instant::now();
        let result = PdfSheetRenderer::default().render(drawing, sheet, budget);
        self.render_times.borrow_mut().push(started.elapsed());
        result
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, RenderFailure> {
        let started = Instant::now();
        let result = PdfSheetRenderer::default().merge(parts, labels, title);
        self.merge_times.borrow_mut().push(started.elapsed());
        result
    }
}

fn advance(
    runner: &mut JobRunner,
    store: &mut dyn JobStore,
    loader: &mut dyn SourceLoader,
    renderer: &TimedRenderer,
    budget: &SheetBudget,
    stop_at: Option<u32>,
) {
    for step in 0..MAX_JOB_SHEETS * 2 {
        let outcome = runner.step(store, loader, renderer, budget, u64::try_from(step).unwrap());
        if stop_at.is_some_and(|count| runner.progress().done == count) || matches!(outcome, StepOutcome::Finished(_)) {
            return;
        }
    }
    panic!("benchmark queue exceeded its bounded step count");
}

fn verify(job: &PrintJob, manifest: &PublishManifest, renderer: &TimedRenderer, sheets: usize, files: usize) -> u64 {
    if manifest.status != JobStatus::Done {
        let failed: Vec<_> = manifest.sheets.iter().filter(|sheet| sheet.status != SheetStatus::Done).take(3).collect();
        panic!("job {:?}: issues {:?}; first unfinished sheets {failed:?}", manifest.status, manifest.issues);
    }
    assert_eq!(manifest.sheets.len(), sheets);
    assert_eq!(manifest.files.len(), files);
    assert!(manifest.sheets.iter().all(|sheet| sheet.status == SheetStatus::Done && sheet.attempts == 1));
    assert_eq!(manifest.sheets.iter().map(|sheet| &sheet.sheet_key).collect::<BTreeSet<_>>().len(), sheets);
    let counts = renderer.counts.borrow();
    assert_eq!(counts.len(), sheets);
    assert!(counts.values().all(|count| *count == 1));
    assert_eq!(renderer.render_times.borrow().len(), sheets);
    assert_eq!(renderer.merge_times.borrow().len(), files);
    let mut expected_keys = Vec::new();
    for (source_index, source) in job.sources.iter().enumerate() {
        let mut ordered: Vec<_> = source.plan.sheets.iter().collect();
        ordered.sort_by_key(|sheet| sheet.order);
        for sheet in ordered {
            assert_eq!(counts.get(&sheet_key(sheet)), Some(&1));
            expected_keys.push(job_sheet_key(u32::try_from(source_index).unwrap(), sheet));
        }
    }
    assert_eq!(manifest.sheets.iter().map(|sheet| sheet.sheet_key.clone()).collect::<Vec<_>>(), expected_keys);
    let mut total_bytes = 0;
    let mut offset = 0;
    for file in &manifest.files {
        let count = usize::try_from(file.pages).unwrap();
        let ordered = manifest.sheets.get(offset..offset + count).unwrap();
        for (page, sheet) in (1..=file.pages).zip(ordered) {
            assert_eq!(sheet.output_file.as_deref(), Some(file.name.as_str()));
            assert_eq!(sheet.page_index, Some(page));
        }
        let bytes = fs::read(&file.delivered_to).unwrap();
        assert_eq!(sha256_hex(&bytes), file.sha256);
        let pdf_pages = pages(&bytes);
        assert_eq!(pdf_pages.len(), count);
        for (page, sheet) in pdf_pages.iter().zip(ordered) {
            assert_eq!(page.label.as_deref(), Some(sheet.sheet_id.as_str()));
        }
        total_bytes += u64::try_from(bytes.len()).unwrap();
        offset += count;
    }
    assert_eq!(offset, sheets);
    assert_eq!(fs::read_dir(job.output.dir.as_ref().unwrap()).unwrap().count(), files);
    total_bytes
}

fn rss() -> String {
    // statm reports resident pages; retaining that unit avoids assuming the OS page size.
    fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|text| text.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or_else(|| "n/a".into(), |pages| pages.to_string())
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn print_measurements(name: &str, renderer: &TimedRenderer, total: Duration, bytes: u64, rss_start: &str, rss_end: &str) {
    let mut times = renderer.render_times.borrow().clone();
    times.sort_unstable();
    let percentile = |percent: usize| times.get((times.len() * percent).div_ceil(100).saturating_sub(1)).copied().unwrap_or_default();
    let merges = renderer.merge_times.borrow();
    println!(
        "bench name={name} total_ms={:.3} p50_ms={:.3} p95_ms={:.3} max_ms={:.3} total_bytes={bytes} sheets={} files={}",
        milliseconds(total),
        milliseconds(percentile(50)),
        milliseconds(percentile(95)),
        milliseconds(times.last().copied().unwrap_or_default()),
        times.len(),
        merges.len(),
    );
    println!("bench name={name} rss_phase=start rss={rss_start} rss_unit=pages");
    println!("bench name={name} rss_phase=end rss={rss_end} rss_unit=pages");
    for (index, elapsed) in merges.iter().enumerate() {
        println!("bench name={name} merge_file={} merge_ms={:.3}", index + 1, milliseconds(*elapsed));
    }
}

#[test]
#[ignore = "manual release benchmark"]
fn queue_bench_1000_sheets() {
    let _lock = BENCH_LOCK.lock().unwrap();
    install_hooks();
    let temp = TempDir::new();
    let drawing = benchmark_drawing(1000, "");
    let rss_start = rss();
    let started = Instant::now();
    let hash = sha256_hex(cadcraft_io::write_dxf(&drawing).as_bytes());
    let handles = drawing.model.iter().map(|entity| entity.handle.hex()).collect();
    let request = ScanRequest { handles, paper: Some("A3".into()), ..Default::default() };
    let plan = build_plan(&drawing, &request, &FrameRegistry::builtin(), &hash).unwrap();
    let source = JobSource { path: None, source_hash: hash.clone(), plan };
    let output = OutputSpec { dir: Some(temp.0.join("out").to_string_lossy().into_owned()), pages_per_file: 256, ..Default::default() };
    let job = PrintJob::new(vec![source], output, DEFAULT_RETRY_LIMIT).unwrap();
    let mut loader = SnapshotLoader(Arc::new(drawing), hash);
    let renderer = TimedRenderer::default();
    let budget = SheetBudget::desktop();
    let mut store = DirJobStore::new(temp.0.join("jobs"));
    let mut runner = JobRunner::open(job.clone(), &mut store, 0).unwrap();
    advance(&mut runner, &mut store, &mut loader, &renderer, &budget, None);
    let elapsed = started.elapsed();
    let rss_end = rss();
    let bytes = verify(&job, runner.manifest(), &renderer, 1000, 4);
    print_measurements("queue_bench_1000_sheets", &renderer, elapsed, bytes, &rss_start, &rss_end);
}

#[test]
#[ignore = "manual release benchmark"]
fn sp06_folder_1000_files_5000_sheets() {
    let _lock = BENCH_LOCK.lock().unwrap();
    install_hooks();
    let temp = TempDir::new();
    let input = temp.0.join("input");
    fs::create_dir(&input).unwrap();
    for index in 0..1000 {
        let prefix = format!("F{index:04}-");
        let drawing = benchmark_drawing(5, &prefix);
        fs::write(input.join(format!("file-{index:04}.dxf")), cadcraft_io::write_dxf(&drawing)).unwrap();
    }
    let mut session = Session::new();
    let mut registry = FrameRegistry::builtin();
    let mut template = registry.templates.first().unwrap().clone();
    template.id = "bench-a3".into();
    template.block_names = vec!["FRAME-A3".into()];
    registry.templates.insert(0, template);
    session.execute("ubplot.templates", &json!({"action": "import", "text": serde_json::to_string(&registry).unwrap()})).unwrap();
    session.set_job_store(Box::new(DirJobStore::new(temp.0.join("jobs"))));
    let rss_start = rss();
    let started = Instant::now();
    let queued =
        session.execute("ubplot.folder", &json!({"folder": input, "out": temp.0.join("out"), "pagesPerFile": 256, "background": true})).unwrap();
    let job = session.take_queued_job().unwrap();
    assert_eq!(queued["status"], "QUEUED");
    assert_eq!(job.sources.len(), 1000);
    assert!(job.sources.iter().all(|source| source.plan.sheets.len() == 5));
    let renderer = TimedRenderer::default();
    let budget = SheetBudget::desktop();
    let mut loader = FolderLoader::default();
    let store = session.job_store_mut().unwrap();
    let mut runner = JobRunner::open(job.clone(), store, 0).unwrap();
    advance(&mut runner, store, &mut loader, &renderer, &budget, Some(2500));
    assert_eq!(runner.progress().done, 2500);
    assert_eq!(renderer.render_times.borrow().len(), 2500);
    let job_id = job.job_id.clone();
    drop(runner);
    drop(loader);
    drop(job);
    drop(session);

    let mut store = DirJobStore::new(temp.0.join("jobs"));
    let job: PrintJob = serde_json::from_slice(&store.read_artifact(&job_id, "job.json").unwrap().unwrap()).unwrap();
    let persisted = PublishManifest::from_json(&store.read_manifest(&job_id).unwrap().unwrap()).unwrap();
    assert_eq!(persisted.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Done).count(), 2500);
    drop(persisted);
    let mut runner = JobRunner::open(job.clone(), &mut store, 0).unwrap();
    let mut loader = FolderLoader::default();
    advance(&mut runner, &mut store, &mut loader, &renderer, &budget, None);
    let elapsed = started.elapsed();
    let rss_end = rss();
    let bytes = verify(&job, runner.manifest(), &renderer, 5000, 20);
    print_measurements("sp06_folder_1000_files_5000_sheets", &renderer, elapsed, bytes, &rss_start, &rss_end);
}
