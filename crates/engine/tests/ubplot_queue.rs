#[allow(dead_code)]
mod support;

use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::queue::*;
use cadcraft_engine::sheetplan::{FrameRegistry, ScanRequest, SheetStatus, build_plan};
use serde_json::{Value, json};
use support::frames::frames_drawing;

fn source(count: usize) -> JobSource {
    let drawing = frames_drawing(count);
    let handles = drawing.model.iter().map(|entity| entity.handle.hex()).collect();
    let source_hash = sha256_hex(cadcraft_io::write_dxf(&drawing).as_bytes());
    let request = ScanRequest { handles, ..Default::default() };
    let plan = build_plan(&drawing, &request, &FrameRegistry::builtin(), &source_hash).unwrap();
    JobSource { path: None, source_hash, plan }
}

fn job() -> PrintJob {
    PrintJob::new(vec![source(3)], OutputSpec::default(), DEFAULT_RETRY_LIMIT).unwrap()
}

#[test]
fn manifest_json_has_contract_keys() {
    let manifest = PublishManifest::new(&job(), 1234);
    let value: Value = serde_json::from_str(&manifest.to_json()).unwrap();
    assert_eq!(value["schema_version"], "1.0");
    let id = value["job_id"].as_str().unwrap();
    assert_eq!(id.len(), 16);
    assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(value["status"], "QUEUED");
    assert_eq!(value["sheets"][0]["sheet_key"], "KT-01@1");
    assert_eq!(value["sheets"][0]["status"], "READY");
    assert_eq!(value["files"], json!([]));
    assert_eq!(manifest.created_ms, 1234);
    assert_eq!(manifest.updated_ms, 1234);
}

#[test]
fn manifest_round_trips() {
    let mut job = job();
    job.profile_id = Some("office".into());
    job.profile_version = Some("0123456789abcdef".into());
    let mut manifest = PublishManifest::new(&job, 42);
    manifest.status = JobStatus::Done;
    manifest.sheets[0].status = SheetStatus::Done;
    manifest.sheets[0].attempts = 1;
    manifest.sheets[0].sha256 = Some(sha256_hex(b"pdf"));
    manifest.sheets[0].output_file = Some("plot.pdf".into());
    manifest.sheets[0].page_index = Some(0);
    manifest.sheets[0].finished_ms = Some(43);
    manifest.files.push(OutputFile {
        name: "plot.pdf".into(),
        pages: 3,
        sha256: sha256_hex(b"merged pdf"),
        delivered_to: "download:plot.pdf".into(),
        delivered_ms: 44,
    });
    manifest.skipped_files.push(SkippedFile { path: "bad.dxf".into(), reason: "SOURCE_UNAVAILABLE".into() });
    assert_eq!(PublishManifest::from_json(&manifest.to_json()).unwrap(), manifest);
}

#[test]
fn job_id_is_stable_and_settings_sensitive() {
    let first = job();
    let again = PrintJob::new(first.sources.clone(), first.output.clone(), first.retry_limit).unwrap();
    assert_eq!(first.job_id, again.job_id);
    let output = OutputSpec { pages_per_file: 1, ..first.output.clone() };
    let changed = PrintJob::new(first.sources.clone(), output, first.retry_limit).unwrap();
    assert_ne!(first.job_id, changed.job_id);
    let expected = cadcraft_engine::sheetplan::ids::job_id(
        &first.sources[0].source_hash,
        &json!({"output": first.output, "retry_limit": first.retry_limit, "printer": null, "profile_version": null}),
    );
    assert_eq!(first.job_id, expected);
}

#[test]
fn blocked_sheet_refuses_job() {
    let mut source = source(1);
    source.plan.sheets[0].status = SheetStatus::Blocked;
    let errors = PrintJob::new(vec![source], OutputSpec::default(), DEFAULT_RETRY_LIMIT).unwrap_err();
    assert_eq!(errors[0].code, "PLAN_HAS_BLOCKED");
    assert_eq!(errors[0].sheet_id.as_deref(), Some("KT-01"));
}

#[test]
fn hostile_manifest_never_panics() {
    let manifest = PublishManifest::new(&job(), 0);
    let valid = manifest.to_json();
    for text in ["{", "null", "[]", &valid[..valid.len() / 2]] {
        assert!(PublishManifest::from_json(text).is_err());
    }
    let mut value: Value = serde_json::from_str(&valid).unwrap();
    value["sheets"][0]["order"] = json!(-1);
    assert!(PublishManifest::from_json(&value.to_string()).is_err());
    let mut value: Value = serde_json::from_str(&valid).unwrap();
    value["sheets"] = Value::Array(vec![value["sheets"][0].clone(); MAX_JOB_SHEETS + 1]);
    assert!(PublishManifest::from_json(&value.to_string()).is_err());
    let text = " ".repeat(65 * 1024 * 1024);
    assert!(PublishManifest::from_json(&text).is_err());
    let output = OutputSpec { pages_per_file: 0, ..Default::default() };
    assert!(PrintJob::new(vec![source(1)], output.clone(), 0).is_err());
    assert!(serde_json::from_value::<OutputSpec>(serde_json::to_value(output).unwrap()).is_err());
}

#[test]
fn job_limits_and_duplicate_keys_are_rejected() {
    let one = source(1);
    for pages_per_file in [0, u32::try_from(cadcraft_doc::MAX_PLOT_PAGES + 1).unwrap()] {
        assert!(PrintJob::new(vec![one.clone()], OutputSpec { pages_per_file, ..Default::default() }, 0).is_err());
    }
    let at_limit = PrintJob::new(
        vec![one.clone()],
        OutputSpec { pages_per_file: u32::try_from(cadcraft_doc::MAX_PLOT_PAGES).unwrap(), ..Default::default() },
        MAX_RETRY_LIMIT,
    );
    assert!(at_limit.is_ok());
    assert!(PrintJob::new(vec![one.clone()], OutputSpec::default(), MAX_RETRY_LIMIT + 1).is_err());
    assert!(PrintJob::new(vec![one.clone(); MAX_JOB_SOURCES + 1], OutputSpec::default(), 0).is_err());
    let mut duplicate = one.clone();
    duplicate.plan.sheets.push(duplicate.plan.sheets[0].clone());
    let errors = PrintJob::new(vec![duplicate], OutputSpec::default(), 0).unwrap_err();
    assert_eq!(errors[0].code, "DUPLICATE_SHEET_KEY");
    let mut too_many = one;
    too_many.plan.sheets = vec![too_many.plan.sheets[0].clone(); MAX_JOB_SHEETS + 1];
    assert!(PrintJob::new(vec![too_many], OutputSpec::default(), 0).is_err());
}

#[test]
fn multiple_sources_have_scoped_keys_and_ordered_hashes() {
    let mut first = source(3);
    first.plan.sheets.reverse();
    first.plan.sheets[0].status = SheetStatus::NeedsPreview;
    let mut second = source(1);
    second.source_hash = sha256_hex(b"second source");
    let expected_hash = sha256_hex(format!("{}{}", first.source_hash, second.source_hash).as_bytes());
    let job = PrintJob::new(vec![first, second], OutputSpec::default(), 0).unwrap();
    let manifest = PublishManifest::new(&job, 0);
    assert_eq!(manifest.source_hash, expected_hash);
    assert_eq!(manifest.sheets.iter().map(|sheet| sheet.sheet_key.as_str()).collect::<Vec<_>>(), ["KT-01@1", "KT-02@2", "KT-03@3", "1:KT-01@1"]);
    assert!(manifest.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready));
    assert_eq!(PublishManifest::from_json(&manifest.to_json()).unwrap(), manifest);
}

#[test]
fn memory_store_commit_is_all_or_nothing() {
    let mut store = MemoryJobStore::default();
    let job = job();
    let previous = PublishManifest::new(&job, 1).to_json();
    let changed = PublishManifest::new(&job, 2).to_json();
    let first = artifact_name(1, 0);
    let second = artifact_name(2, 0);
    store.commit(&job.job_id, &[(first.clone(), vec![1])], &previous).unwrap();
    store.fail_after = Some(1);
    assert!(store.commit(&job.job_id, &[(first.clone(), vec![2]), (second.clone(), vec![3])], &changed).is_err());
    assert_eq!(store.read_manifest(&job.job_id).unwrap().as_deref(), Some(previous.as_str()));
    assert_eq!(store.read_artifact(&job.job_id, &first).unwrap(), Some(vec![1]));
    assert_eq!(store.read_artifact(&job.job_id, &second).unwrap(), None);
    store.fail_after = None;
    store.fail_writes = true;
    assert!(store.commit(&job.job_id, &[], &changed).is_err());
    assert_eq!(store.read_manifest(&job.job_id).unwrap().as_deref(), Some(previous.as_str()));
}

#[test]
fn store_rejects_path_like_names() {
    let mut memory = MemoryJobStore::default();
    assert_invalid_names(&mut memory);
}

fn assert_invalid_names(store: &mut dyn JobStore) {
    let job = job();
    for invalid in ["../x", "a/b", "C:x", "", ".", "..", "a\\b"] {
        assert!(store.read_manifest(invalid).is_err());
        assert!(store.commit(invalid, &[], "{}").is_err());
        assert!(store.read_artifact(&job.job_id, invalid).is_err());
        assert!(store.commit(&job.job_id, &[(invalid.into(), vec![])], "{}").is_err());
        assert!(store.delete_artifacts(&job.job_id, &[invalid.into()]).is_err());
        assert!(store.deliver(&job, invalid, &[]).is_err());
        assert!(store.remove_job(invalid).is_err());
    }
}

#[test]
fn memory_store_lifecycle() {
    let job = job();
    let manifest = PublishManifest::new(&job, 0).to_json();
    let mut store = MemoryJobStore::default();
    assert!(store.list_jobs().is_empty());
    store.commit(&job.job_id, &[(artifact_name(1, 0), vec![1, 2])], &manifest).unwrap();
    assert_eq!(store.list_jobs(), vec![job.job_id.clone()]);
    assert_eq!(store.deliver(&job, "out.pdf", b"pdf").unwrap(), "download:out.pdf");
    store.delete_artifacts(&job.job_id, &[artifact_name(1, 0)]).unwrap();
    assert_eq!(store.read_artifact(&job.job_id, &artifact_name(1, 0)).unwrap(), None);
    store.remove_job(&job.job_id).unwrap();
    assert!(store.list_jobs().is_empty());
    assert_eq!(artifact_name(1, 0), "s00000-0001.pdf");
    assert_eq!(artifact_name(12, 3), "s00003-0012.pdf");
}

#[cfg(not(target_arch = "wasm32"))]
mod directory {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cadcraft-g5-t1-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dir_store_commit_writes_artifacts_before_manifest() {
        let temp = TempDir::new();
        let mut store = DirJobStore::new(temp.0.clone());
        let job = job();
        let manifest = PublishManifest::new(&job, 0).to_json();
        let name = artifact_name(1, 0);
        store.commit(&job.job_id, &[(name.clone(), b"pdf".to_vec())], &manifest).unwrap();
        let dir = temp.0.join(&job.job_id);
        assert_eq!(std::fs::read(dir.join("sheets").join(&name)).unwrap(), b"pdf");
        assert_eq!(std::fs::read_to_string(dir.join("manifest.json")).unwrap(), manifest);
        for folder in [&dir, &dir.join("sheets")] {
            assert!(std::fs::read_dir(folder).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().ends_with(".part")));
        }
        assert_eq!(store.list_jobs(), vec![job.job_id.clone()]);
        assert_invalid_names(&mut store);
        store.remove_job(&job.job_id).unwrap();
        assert!(store.list_jobs().is_empty());
    }

    #[test]
    fn two_jobs_never_share_temp_names() {
        let temp = TempDir::new();
        let mut store = DirJobStore::new(temp.0.join("jobs"));
        let mut first = job();
        first.output.dir = Some(temp.0.to_string_lossy().into_owned());
        let mut second = first.clone();
        second.job_id = "0123456789abcdef".into();
        assert_ne!(DirJobStore::temp_name(&first.job_id, "out.pdf"), DirJobStore::temp_name(&second.job_id, "out.pdf"));
        std::fs::write(temp.0.join(DirJobStore::temp_name(&first.job_id, "out.pdf")), b"in progress").unwrap();
        store.deliver(&second, "out.pdf", b"second").unwrap();
        assert_eq!(std::fs::read(temp.0.join("out.pdf")).unwrap(), b"second");
        std::fs::remove_file(temp.0.join(DirJobStore::temp_name(&first.job_id, "out.pdf"))).unwrap();
        store.deliver(&first, "out.pdf", b"first").unwrap();
        store.deliver(&second, "out.pdf", b"second again").unwrap();
        assert_eq!(std::fs::read(temp.0.join("out.pdf")).unwrap(), b"second again");
    }

    #[test]
    fn corrupt_manifest_is_reported_not_overwritten() {
        let temp = TempDir::new();
        let job = job();
        let dir = temp.0.join(&job.job_id);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), "{").unwrap();
        let store = DirJobStore::new(temp.0.clone());
        assert_eq!(store.read_manifest(&job.job_id).unwrap(), Some("{".into()));
        assert_eq!(std::fs::read_to_string(dir.join("manifest.json")).unwrap(), "{");
    }

    #[test]
    fn failed_artifact_write_keeps_previous_manifest() {
        let temp = TempDir::new();
        let mut store = DirJobStore::new(temp.0.clone());
        let job = job();
        let previous = PublishManifest::new(&job, 1).to_json();
        let changed = PublishManifest::new(&job, 2).to_json();
        store.commit(&job.job_id, &[], &previous).unwrap();
        let name = artifact_name(1, 0);
        let temporary = temp.0.join(&job.job_id).join("sheets").join(DirJobStore::temp_name(&job.job_id, &name));
        std::fs::write(&temporary, b"in progress").unwrap();
        assert!(store.commit(&job.job_id, &[(name, b"pdf".to_vec())], &changed).is_err());
        assert_eq!(store.read_manifest(&job.job_id).unwrap(), Some(previous));
        assert_eq!(std::fs::read(temporary).unwrap(), b"in progress");
    }

    #[test]
    fn deliver_refuses_output_is_source() {
        let temp = TempDir::new();
        let mut store = DirJobStore::new(temp.0.join("jobs"));
        let mut job = job();
        job.output.dir = Some(temp.0.to_string_lossy().into_owned());
        job.sources[0].path = Some(temp.0.join("out.pdf").to_string_lossy().into_owned());
        std::fs::write(temp.0.join("out.pdf"), b"source").unwrap();
        assert!(store.deliver(&job, "out.pdf", b"replacement").unwrap_err().contains("OUTPUT_IS_SOURCE"));
        assert_eq!(std::fs::read(temp.0.join("out.pdf")).unwrap(), b"source");
        job.sources[0].path = Some(temp.0.join(DirJobStore::temp_name(&job.job_id, "other.pdf")).to_string_lossy().into_owned());
        assert!(store.deliver(&job, "other.pdf", b"replacement").unwrap_err().contains("OUTPUT_IS_SOURCE"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn watchdog_cancels_after_deadline() {
    let drawing = frames_drawing(1);
    let source = source(1);
    let sheet = source.plan.sheets.first().unwrap();
    let budget = SheetBudget { time_ms: 50, ..SheetBudget::desktop() };
    let started = std::time::Instant::now();
    let error = budget.render(&support::frames::WaitingSheetRenderer, &drawing, sheet).unwrap_err();
    assert_eq!(error, RenderFailure::TimedOut);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(budget.cancel.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn output_over_max_bytes_is_too_large() {
    let drawing = frames_drawing(1);
    let source = source(1);
    let sheet = source.plan.sheets.first().unwrap();
    let budget = SheetBudget { max_bytes: 1024, ..SheetBudget::desktop() };
    let error = budget.render(&support::frames::FixedSheetRenderer(2048), &drawing, sheet).unwrap_err();
    assert!(matches!(error, RenderFailure::TooLarge(_)), "{error:?}");
    assert_eq!(budget.render(&support::frames::FixedSheetRenderer(1024), &drawing, sheet).unwrap().len(), 1024);
}

#[test]
fn render_budgets_have_platform_limits() {
    let desktop = SheetBudget::desktop();
    assert_eq!(desktop.max_prims, 50_000_000);
    assert_eq!(desktop.max_bytes, 512 * 1024 * 1024);
    assert_eq!(desktop.time_ms, 300_000);
    let web = SheetBudget::web();
    assert_eq!(web.max_prims, 10_000_000);
    assert_eq!(web.max_bytes, 128 * 1024 * 1024);
    assert!(!desktop.cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert!(!web.cancel.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn cancelled_budget_discards_renderer_output() {
    let drawing = frames_drawing(1);
    let source = source(1);
    let sheet = source.plan.sheets.first().unwrap();
    let budget = SheetBudget::desktop();
    assert_eq!(budget.render(&support::frames::CancellingSheetRenderer, &drawing, sheet), Err(RenderFailure::Cancelled));
    assert_eq!(budget.render(&support::frames::FixedSheetRenderer(1024), &drawing, sheet), Err(RenderFailure::Cancelled));
    assert_eq!(PdfSheetRenderer.render(&drawing, sheet, &budget), Err(RenderFailure::Cancelled));
}

#[test]
fn pdf_sheet_renderer_passes_primitive_budget_and_writes_one_page() {
    support::frames::install_budget_plot();
    let drawing = frames_drawing(1);
    let source = source(1);
    let sheet = source.plan.sheets.first().unwrap();
    let budget = SheetBudget::desktop();
    let bytes = budget.render(&PdfSheetRenderer, &drawing, sheet).unwrap();
    let pages = support::pdf_probe::pages(&bytes);
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].label.as_deref(), Some(sheet.sheet_id.as_str()));
    let tiny = SheetBudget { max_prims: 0, ..SheetBudget::desktop() };
    assert!(matches!(tiny.render(&PdfSheetRenderer, &drawing, sheet), Err(RenderFailure::TooLarge(_))));
    let tiny = SheetBudget { max_bytes: 1, ..SheetBudget::desktop() };
    assert!(matches!(PdfSheetRenderer.render(&drawing, sheet, &tiny), Err(RenderFailure::TooLarge(_))));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn watchdog_disarm_keeps_completed_sheet_uncancelled() {
    let budget = SheetBudget::desktop();
    let started = std::time::Instant::now();
    let watchdog = Watchdog::arm(budget.cancel.clone(), std::time::Duration::from_secs(300)).unwrap();
    assert!(!watchdog.disarm());
    assert!(!budget.cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn pdf_sheet_renderer_rejects_split_instead_of_dropping_pages() {
    let drawing = frames_drawing(1);
    let source = source(1);
    let mut sheet = source.plan.sheets.first().unwrap().clone();
    sheet.split = Some(cadcraft_engine::sheetplan::SplitPlan { pieces: Vec::new(), overlap_mm: 0.0, approved: true });
    let error = PdfSheetRenderer.render(&drawing, &sheet, &SheetBudget::desktop()).unwrap_err();
    assert!(matches!(error, RenderFailure::Error(message) if message.contains("split")));
}
