#![cfg(not(target_arch = "wasm32"))]

#[allow(dead_code)]
mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cadcraft_doc::Drawing;
use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::Sheet;
use cadcraft_engine::sheetplan::queue::store::validate_output_file_name;
use cadcraft_engine::sheetplan::queue::*;
use support::frames::queue_job;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot-output-names-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct NameRenderer;

const MERGED_BYTES: &[u8] = b"synthetic merged PDF bytes";

impl SheetRenderer for NameRenderer {
    fn render(&self, _: &Drawing, _: &Sheet, _: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        Ok(b"synthetic sheet PDF bytes".to_vec())
    }

    fn merge(&self, _: &[&[u8]], _: &[String], _: &str) -> Result<Vec<u8>, RenderFailure> {
        Ok(MERGED_BYTES.to_vec())
    }
}

#[test]
fn vietnamese_and_spaced_output_names_are_delivered() {
    let temp = TempDir::new();
    let (base, snapshot) = queue_job(1);
    let mut store = DirJobStore::new(temp.0.join("jobs"));
    let mut memory = MemoryJobStore::default();
    let names = [
        "Bản vẽ KT 01.pdf".to_string(),
        "plan_A 2.pdf".to_string(),
        "図面 🏠.v2.pdf".to_string(),
        "Ba\u{0309}n ve\u{0303} 01.pdf".to_string(),
        "floor..revision.pdf".to_string(),
        "floor .pdf".to_string(),
        format!("{}.pdf", "a".repeat(251)),
        format!("{}ab.pdf", "ế".repeat(83)),
    ];
    for name in names {
        validate_output_file_name(&name).unwrap();
        let output = OutputSpec {
            stem: name.strip_suffix(".pdf").unwrap().into(),
            dir: Some(temp.0.join("out").to_string_lossy().into_owned()),
            ..Default::default()
        };
        let job = PrintJob::new(base.sources.clone(), output, 0).unwrap();
        let mut loader = SnapshotLoader(snapshot.0.clone(), snapshot.1.clone());
        let manifest = run_to_end(job.clone(), &mut store, &mut loader, &NameRenderer, &SheetBudget::desktop(), &|| 7).unwrap();
        assert_eq!(manifest.status, JobStatus::Done);
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].name, name);
        assert_eq!(manifest.files[0].sha256, sha256_hex(MERGED_BYTES));
        assert_eq!(manifest.sheets[0].output_file.as_deref(), Some(name.as_str()));
        assert_eq!(std::fs::read(temp.0.join("out").join(&name)).unwrap(), MERGED_BYTES);
        let persisted = PublishManifest::from_json(&store.read_manifest(&job.job_id).unwrap().unwrap()).unwrap();
        assert_eq!(persisted, manifest);
        assert!(!temp.0.join("out").join(DirJobStore::temp_name(&job.job_id, &name)).exists());
        assert_eq!(memory.deliver(&job, &name, MERGED_BYTES).unwrap(), format!("download:{name}"));
    }
}

#[test]
fn hostile_output_names_are_refused() {
    let temp = TempDir::new();
    let (mut job, _) = queue_job(1);
    job.output.dir = Some(temp.0.join("out").to_string_lossy().into_owned());
    let mut store = DirJobStore::new(temp.0.join("jobs"));
    let mut memory = MemoryJobStore::default();
    let mut names: Vec<String> = [
        "",
        ".",
        "..",
        "../escape.pdf",
        "dir/../escape.pdf",
        "dir/file.pdf",
        "dir\\file.pdf",
        "dir\\..\\escape.pdf",
        "file.pdf ",
        "file.pdf.",
        "CON .pdf",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    names.extend(["a".repeat(256), format!("{}.pdf", "é".repeat(126))]);
    for ch in (0_u8..=31).map(char::from).chain(['\u{007f}', '\u{0085}', '<', '>', ':', '"', '|', '?', '*']) {
        names.push(format!("file{ch}.pdf"));
    }
    let mut reserved: Vec<_> = ["CON", "PRN", "AUX", "NUL"].into_iter().map(str::to_string).collect();
    for number in 1..=9 {
        reserved.extend([format!("COM{number}"), format!("LPT{number}")]);
    }
    for stem in reserved {
        names.extend([stem.clone(), stem.to_lowercase(), format!("{stem}.pdf"), format!("{}.v2.pdf", stem.to_lowercase())]);
    }
    for name in names {
        let error = validate_output_file_name(&name).unwrap_err();
        assert!(error.starts_with("invalid output file name: "), "{name:?}: {error}");
        assert_eq!(store.deliver(&job, &name, b"bytes").unwrap_err(), error);
        assert_eq!(memory.deliver(&job, &name, b"bytes").unwrap_err(), error);
        assert!(!temp.0.join("out").exists(), "{name:?} wrote an output directory");
        assert!(!temp.0.join("jobs").exists(), "{name:?} wrote job state");
    }
}

#[test]
fn output_names_are_validated_in_jobs_and_manifests() {
    let (job, _) = queue_job(1);
    for stem in ["", "CON", "com9", "bad/name", "bad\\name", "bad:name", "bad\0name", &"a".repeat(252)] {
        let output = OutputSpec { stem: stem.into(), ..Default::default() };
        let errors = PrintJob::new(job.sources.clone(), output, 0).unwrap_err();
        assert_eq!(errors[0].code, "OUTPUT_NAME_INVALID");
        assert!(errors[0].message.starts_with("invalid output file name: "));
    }
    for name in ["../file.pdf", "NUL.pdf", "bad?.pdf", "bad.pdf "] {
        let mut manifest = PublishManifest::new(&job, 0);
        manifest.files.push(OutputFile { name: name.into(), pages: 1, sha256: String::new(), delivered_to: String::new(), delivered_ms: 0 });
        assert!(PublishManifest::from_json(&manifest.to_json()).unwrap_err().starts_with("invalid output file name: "));
        manifest.files.clear();
        manifest.sheets[0].output_file = Some(name.into());
        assert!(PublishManifest::from_json(&manifest.to_json()).unwrap_err().starts_with("invalid output file name: "));
    }
}

#[test]
fn generated_output_names_keep_unicode_in_split_and_grouped_jobs() {
    let temp = TempDir::new();
    let (base, snapshot) = queue_job(2);
    for mode in [OutputMode::PerSheet, OutputMode::Merged] {
        let mut sources = base.sources.clone();
        sources[0].plan.sheets[0].sheet_id = "Tầng_1 bản vẽ".into();
        sources[0].plan.sheets[1].sheet_id = "Tầng_2 bản vẽ".into();
        let output =
            OutputSpec { mode, stem: "Bản vẽ KT 01".into(), pages_per_file: 1, dir: Some(temp.0.join("out").to_string_lossy().into_owned()) };
        let job = PrintJob::new(sources, output, 0).unwrap();
        let mut store = DirJobStore::new(temp.0.join("jobs"));
        let mut loader = SnapshotLoader(snapshot.0.clone(), snapshot.1.clone());
        let manifest = run_to_end(job, &mut store, &mut loader, &NameRenderer, &SheetBudget::desktop(), &|| 1).unwrap();
        assert_eq!(manifest.status, JobStatus::Done);
        assert_eq!(manifest.files.len(), 2);
        for (index, file) in manifest.files.iter().enumerate() {
            let expected = if mode == OutputMode::PerSheet {
                format!("Bản vẽ KT 01-{:04}-Tầng_{} bản vẽ.pdf", index + 1, index + 1)
            } else {
                format!("Bản vẽ KT 01-{:03}.pdf", index + 1)
            };
            assert_eq!(file.name, expected);
            assert_eq!(std::fs::read(temp.0.join("out").join(expected)).unwrap(), MERGED_BYTES);
        }
    }
}

#[test]
fn expanded_output_names_over_255_bytes_are_refused_without_writes() {
    let temp = TempDir::new();
    let (base, _) = queue_job(2);
    for mode in [OutputMode::Merged, OutputMode::PerSheet] {
        let output = OutputSpec { mode, stem: "a".repeat(251), pages_per_file: 1, dir: Some(temp.0.join("out").to_string_lossy().into_owned()) };
        let job = PrintJob::new(base.sources.clone(), output, 0).unwrap();
        let mut store = DirJobStore::new(temp.0.join("jobs"));
        match JobRunner::open(job, &mut store, 0) {
            Ok(_) => panic!("the expanded output name exceeds 255 bytes"),
            Err(error) => assert!(error.starts_with("invalid output file name: ")),
        }
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
    }
}

#[test]
fn internal_job_and_artifact_names_stay_ascii() {
    let temp = TempDir::new();
    let (job, _) = queue_job(1);
    let mut disk = DirJobStore::new(temp.0.join("jobs"));
    let mut memory = MemoryJobStore::default();
    for name in ["Bản vẽ.pdf", "plan_A.pdf", "plan 2.pdf", "../escape"] {
        for store in [&mut disk as &mut dyn JobStore, &mut memory as &mut dyn JobStore] {
            assert!(store.read_manifest(name).is_err());
            assert!(store.commit(&job.job_id, &[(name.into(), b"bytes".to_vec())], "{}").is_err());
            assert!(store.read_artifact(&job.job_id, name).is_err());
        }
        assert!(!temp.0.join("jobs").exists());
    }
    let artifact = artifact_name(1, 0);
    disk.commit(&job.job_id, &[(artifact.clone(), b"page".to_vec())], &PublishManifest::new(&job, 0).to_json()).unwrap();
    assert_eq!(disk.read_artifact(&job.job_id, &artifact).unwrap().unwrap(), b"page");
}

#[test]
fn long_unicode_temp_names_are_bounded_and_distinct() {
    let id = "0123456789abcdef";
    assert_eq!(DirJobStore::temp_name(id, "Bản vẽ.pdf"), format!("Bản vẽ.pdf.{id}.part"));
    let first = format!("{}a.pdf", "ế".repeat(83));
    let second = format!("{}b.pdf", "ế".repeat(83));
    assert_eq!(first.len(), 254);
    let first_temp = DirJobStore::temp_name(id, &first);
    let second_temp = DirJobStore::temp_name(id, &second);
    validate_output_file_name(&first_temp).unwrap();
    validate_output_file_name(&second_temp).unwrap();
    assert!(first_temp.starts_with('ế'));
    assert!(first_temp.ends_with(&format!(".{id}.part")));
    assert_ne!(first_temp, second_temp);
    assert_ne!(first_temp, DirJobStore::temp_name("fedcba9876543210", &first));

    let temp = TempDir::new();
    let (mut job, _) = queue_job(1);
    job.output.dir = Some(temp.0.join("out").to_string_lossy().into_owned());
    job.sources[0].path = Some(temp.0.join("out").join(DirJobStore::temp_name(&job.job_id, &first)).to_string_lossy().into_owned());
    let mut store = DirJobStore::new(temp.0.join("jobs"));
    assert!(store.deliver(&job, &first, b"bytes").unwrap_err().contains("OUTPUT_IS_SOURCE"));
    assert!(!temp.0.join("out").exists());
}

#[test]
fn publish_validates_picked_basename_before_enqueue() {
    use cadcraft_engine::Session;
    use cadcraft_engine::cmd::file::{IoHooks, set_io};
    use serde_json::json;

    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
        plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
    });
    support::frames::install_budget_plot();
    let temp = TempDir::new();
    let mut session = Session::empty();
    session.open_drawing(support::frames::frames_drawing(1), "frames", None);
    session.set_job_store(Box::new(DirJobStore::new(temp.0.join("jobs"))));
    let handles: Vec<_> = session.doc().unwrap().model.iter().map(|entity| entity.handle.hex()).collect();
    session.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap();
    for background in [false, true] {
        for name in ["file.pdf.", "file.pdf ", "CON.pdf", "bad?.pdf"] {
            let result = session.execute("ubplot.publish", &json!({"background": background, "path": temp.0.join(name)}));
            assert!(result.unwrap_err().to_string().contains("invalid output file name:"), "{name}");
            assert!(session.take_queued_job().is_none());
            assert!(!temp.0.join("jobs").exists());
            assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
        }
    }
    let path = temp.0.join("Bản vẽ KT 01.pdf");
    let queued = session.execute("ubplot.publish", &json!({"background": true, "path": path})).unwrap();
    let result = session.execute("ubplot.jobs", &json!({"action": "resume", "jobId": queued["jobId"]})).unwrap();
    assert_eq!(result["manifest"]["status"], "DONE");
    assert_eq!(result["manifest"]["files"][0]["name"], "Bản vẽ KT 01.pdf");
    assert_eq!(support::pdf_probe::pages(&std::fs::read(path).unwrap()).len(), 1);
}
