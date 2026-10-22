#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use cadcraft_engine::doc::{Block, Common, Drawing, Entity, EntityKind, Insert, Layout, LwPolyline, Space};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::{FrameRegistry, Severity, detect_frames};
use serde_json::Value;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot cli {}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn folder(&self) -> PathBuf {
        let path = self.0.join("source drawings");
        fs::create_dir(&path).unwrap();
        for index in 0..3 {
            let mut drawing = Drawing::new_metric();
            let mut layout = Layout::new(&format!("Page-{index}"), 1);
            layout.page.paper = "ISO A4 (210.00 x 297.00 MM)".into();
            layout.page.width_mm = 210.0;
            layout.page.height_mm = 297.0;
            layout.page.landscape = false;
            drawing.layouts = vec![layout];
            fs::write(path.join(format!("drawing-{index}.dxf")), cadcraft_io::write(&drawing, "fixture.dxf").unwrap()).unwrap();
        }
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn folder_command(folder: &Path, out: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cadcraft-cli"));
    command.arg("ubplot").arg("--folder").arg(folder).arg("--out").arg(out).arg("--layouts");
    command
}

fn manifest(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["status"], "DONE");
    manifest
}

#[test]
fn cli_ubplot_folder_writes_manifest_and_pdfs() {
    let temp = TempDir::new();
    let folder = temp.folder();
    let out = temp.0.join("published drawings");
    let jobs = temp.0.join("custom jobs");
    let before: Vec<_> = (0..3)
        .map(|index| {
            let path = folder.join(format!("drawing-{index}.dxf"));
            (path.clone(), fs::read(&path).unwrap(), fs::metadata(path).unwrap().modified().unwrap())
        })
        .collect();
    let response = folder_command(&folder, &out).arg("--jobs-dir").arg(&jobs).output().unwrap();
    let manifest = manifest(&response);
    assert_eq!(manifest["sources"].as_array().unwrap().len(), 3);
    assert_eq!(manifest["sheets"].as_array().unwrap().len(), 3);
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["pages"], 3);
    assert_eq!(files[0]["name"], "plot.pdf");
    let pdf = fs::read(out.join("plot.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(files[0]["sha256"], sha256_hex(&pdf));
    let id = manifest["job_id"].as_str().unwrap();
    assert!(jobs.join(id).join("manifest.json").is_file());
    assert!(!out.join(".ubplot-jobs").exists());
    for (path, bytes, modified) in before {
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn cli_ubplot_rerun_resumes_and_does_not_duplicate() {
    let temp = TempDir::new();
    let folder = temp.folder();
    let out = temp.0.join("published drawings");
    let first = manifest(&folder_command(&folder, &out).arg("--pages-per-file").arg("2").output().unwrap());
    let files = first["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let mut before = Vec::new();
    for file in files {
        let path = out.join(file["name"].as_str().unwrap());
        // A fixed older timestamp detects redelivery even on filesystems with coarse time resolution.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000)))
            .unwrap();
        before.push((path.clone(), fs::metadata(&path).unwrap().modified().unwrap(), sha256_hex(&fs::read(path).unwrap())));
    }
    let second = manifest(&folder_command(&folder, &out).arg("--pages-per-file").arg("2").output().unwrap());
    assert_eq!(second["job_id"], first["job_id"]);
    assert_eq!(second["files"], first["files"]);
    assert!(second["sheets"].as_array().unwrap().iter().all(|sheet| sheet["attempts"] == 1));
    for (path, modified, hash) in before {
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(sha256_hex(&fs::read(path).unwrap()), hash);
    }
    assert_eq!(fs::read_dir(out.join(".ubplot-jobs")).unwrap().count(), 1);
}

fn ambiguous_frame(path: &Path) {
    let mut drawing = Drawing::new_metric();
    let mut block = Block::new("FRAME-A3");
    for offset in [0.0, 500.0] {
        let vertices = [Vec2::ZERO, Vec2::new(420.0, 0.0), Vec2::new(420.0, 297.0), Vec2::new(0.0, 297.0)]
            .map(|point| PolyVertex::new(point + Vec2::new(offset, 0.0)))
            .into();
        block.entities.push(Entity::new(
            drawing.new_handle(),
            EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false }),
        ));
    }
    drawing.blocks.insert(block.name.clone(), Arc::new(block));
    let handle = drawing
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: "FRAME-A3".into(),
                insert: Vec3::ZERO,
                scale: Vec3::new(1.0, 1.0, 1.0),
                rotation: 0.0,
                attribs: Vec::new(),
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap();
    let candidates = detect_frames(&drawing, &[handle], &FrameRegistry::builtin());
    assert!(candidates[0].issues.iter().any(|issue| issue.code == "FRAME_AMBIGUOUS" && issue.severity == Severity::Block));
    fs::write(path, cadcraft_io::write(&drawing, "fixture.dxf").unwrap()).unwrap();
}

#[test]
fn cli_ubplot_exit_code_1_on_failed_job() {
    let temp = TempDir::new();
    let source = temp.0.join("ambiguous.dxf");
    ambiguous_frame(&source);
    let out = temp.0.join("published").join("ambiguous.pdf");
    let response = Command::new(env!("CARGO_BIN_EXE_cadcraft-cli")).arg("ubplot").arg(&source).arg("--out").arg(&out).output().unwrap();
    assert_eq!(response.status.code(), Some(1));
    let error = String::from_utf8_lossy(&response.stderr);
    assert!(error.contains("FRAME_AMBIGUOUS"), "{error}");
    assert!(error.contains("S-001"), "{error}");
    assert!(!out.exists());
}

#[test]
fn cli_ubplot_rejects_unknown_options() {
    let temp = TempDir::new();
    let folder = temp.0.join("missing drawings");
    let out = temp.0.join("published");
    let jobs = temp.0.join("custom jobs");
    let response = folder_command(&folder, &out).arg("--jobs-dir").arg(&jobs).arg("--foo").output().unwrap();
    assert_eq!(response.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&response.stderr).contains("unknown option --foo"));
    assert!(response.stdout.is_empty());
    assert!(!out.exists());
    assert!(!jobs.exists());

    let response = Command::new(env!("CARGO_BIN_EXE_cadcraft-cli"))
        .arg("ubplot")
        .arg(temp.0.join("missing.dxf"))
        .arg("--out")
        .arg(out.join("single.pdf"))
        .arg("--foo")
        .output()
        .unwrap();
    assert_eq!(response.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&response.stderr).contains("unknown option --foo"));
    assert!(!out.exists());
}
