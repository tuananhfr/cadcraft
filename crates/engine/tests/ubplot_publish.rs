#[allow(dead_code)]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cadcraft_doc::{Common, Handle, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use cadcraft_engine::sheetplan::{SheetPlan, SheetStatus, codes, ids};
use serde_json::{Value, json};
use support::frames::{anon_dynamic, frame_block, nest, place, title_only_block};
use support::pdf_probe::{ProbePage, pages};

fn setup(count: usize) -> (Session, Vec<String>) {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|error| error.to_string())),
    });
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "A1", 841.0, 594.0);
    let handles = (0..count)
        .map(|index| place(&mut s, "A1", [(index % 10) as f64 * 2000.0, (index / 10) as f64 * 1500.0], 1.0, 0.0, false, &[]).hex())
        .collect();
    (s, handles)
}

fn scan(s: &mut Session, handles: &[String]) -> Value {
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap()
}

fn file_bytes(file: &Value) -> Vec<u8> {
    match file["path"].as_str() {
        Some(path) => std::fs::read(path).unwrap(),
        None => base64_decode(file["data"].as_str().unwrap()).unwrap(),
    }
}

fn publish(s: &mut Session) -> Value {
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    assert!(s.running.is_none());
    assert!(result["failed"].as_array().unwrap().is_empty());
    assert!(result["plan"]["sheets"].as_array().unwrap().iter().all(|sheet| sheet["status"] == "DONE"));
    let stored = &s.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(serde_json::to_value(stored).unwrap(), result["plan"]);
    result
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("ubplot-publish-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_box(page: &ProbePage, expected: [f64; 4]) {
    assert_eq!(page.user_unit, 1.0);
    for (got, want) in page.media_box.into_iter().zip(expected) {
        assert!((got - want).abs() < 0.05, "{got} != {want}");
    }
}

fn labels(pages: &[ProbePage]) -> Vec<&str> {
    pages.iter().map(|page| page.label.as_deref().unwrap()).collect()
}

#[test]
fn sp01_one_frame_one_page() {
    let (mut s, handles) = setup(1);
    scan(&mut s, &handles);
    let result = publish(&mut s);
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    let pdf = pages(&file_bytes(&result["files"][0]));
    assert_eq!(pdf.len(), 1);
    assert_box(&pdf[0], [0.0, 0.0, 2383.94, 1683.78]);
    assert_eq!(labels(&pdf), ["S-001"]);
}

#[test]
fn sp02_hundred_frames_pick_ten_exactly_ten_pages() {
    let (mut s, handles) = setup(100);
    let picked = [37, 2, 98, 11, 56, 43, 70, 5, 26, 89].map(|index| handles[index].clone());
    let plan = scan(&mut s, &picked);
    for (sheet, handle) in plan["sheets"].as_array().unwrap().iter().zip(&picked) {
        assert_eq!(sheet["source"]["block_handle"].as_str(), Some(handle.as_str()));
    }
    let result = publish(&mut s);
    let pdf = pages(&file_bytes(&result["files"][0]));
    assert_eq!(pdf.len(), 10);
    let expected: Vec<_> = (1..=10).map(|index| format!("S-{index:03}")).collect();
    assert_eq!(labels(&pdf), expected);
}

#[test]
fn sp03_nested_dynamic_rotated_pages_match_frames() {
    let (mut s, _) = setup(0);
    let d = s.doc_mut().unwrap();
    nest(d, "MIDDLE", "A1", [5.0, 5.0], 10.0);
    nest(d, "OUTER", "MIDDLE", [5.0, 5.0], 10.0);
    anon_dynamic(d, "A1", "*U1");
    let handles = [
        place(&mut s, "OUTER", [0.0, 0.0], 1.0, 10.0, false, &[]).hex(),
        place(&mut s, "*U1", [3000.0, 0.0], 1.0, 0.0, false, &[]).hex(),
        place(&mut s, "A1", [6000.0, 0.0], 1.0, 90.0, false, &[]).hex(),
        place(&mut s, "A1", [500_000.0, 2_100_000.0], 1.0, 30.0, false, &[]).hex(),
        place(&mut s, "A1", [9000.0, 0.0], 1.0, 0.0, true, &[]).hex(),
    ];
    scan(&mut s, &handles);
    let result = publish(&mut s);
    let pdf = pages(&file_bytes(&result["files"][0]));
    assert_eq!(pdf.len(), handles.len());
    for page in &pdf {
        assert_box(page, [0.0, 0.0, 2383.94, 1683.78]);
    }
    assert!(result["plan"]["sheets"][4]["issues"].as_array().unwrap().iter().any(|issue| issue["code"] == codes::FRAME_MIRRORED));
}

#[test]
fn sp04_window_and_layout_pages_in_order() {
    let (mut s, _) = setup(0);
    let windows = s.execute("ubplot.scan", &json!({"mode": "window", "windows": [[[0, 0], [420, 297]], [[500, 0], [920, 297]]]})).unwrap();
    let mut plan: SheetPlan = serde_json::from_value(windows).unwrap();
    let layouts = s.execute("ubplot.scan", &json!({"mode": "layout", "layouts": ["Layout1"]})).unwrap();
    let layout: SheetPlan = serde_json::from_value(layouts).unwrap();
    plan.sheets.push(layout.sheets[0].clone());
    for (sheet, (order, name)) in plan.sheets.iter_mut().zip([(3, "Window last"), (1, "Window first"), (2, "Layout middle")]) {
        sheet.order = order;
        sheet.sheet_id = name.into();
    }
    let result = s.execute("ubplot.publish", &json!({"plan": plan})).unwrap();
    let pdf = pages(&file_bytes(&result["files"][0]));
    assert_eq!(pdf.len(), 3);
    assert_eq!(labels(&pdf), ["Window first", "Layout middle", "Window last"]);
}

#[test]
fn sp12_merged_and_split_have_same_pages_order_and_checksums() {
    let dir = TempDir::new();
    let (mut s, handles) = setup(10);
    scan(&mut s, &handles);
    let merged = s.execute("ubplot.publish", &json!({"path": dir.path("merged.pdf")})).unwrap();
    let expected = pages(&file_bytes(&merged["files"][0]));
    let split = s.execute("ubplot.publish", &json!({"output": "split", "path": dir.path("split.pdf")})).unwrap();
    let files = split["files"].as_array().unwrap();
    assert_eq!(files.len(), 10);
    for (index, file) in files.iter().enumerate() {
        let bytes = file_bytes(file);
        assert_eq!(file["sha256"], ids::sha256_hex(&bytes));
        assert_eq!(file["bytes"].as_u64().unwrap(), bytes.len() as u64);
        let pdf = pages(&bytes);
        assert_eq!(pdf.len(), 1);
        assert_eq!(pdf[0].media_box, expected[index].media_box);
        assert_eq!(pdf[0].label, expected[index].label);
        assert_eq!(file["sheets"], json!([format!("S-{:03}@{}", index + 1, index + 1)]));
    }
    let merged_bytes = file_bytes(&merged["files"][0]);
    assert_eq!(merged["files"][0]["sha256"], ids::sha256_hex(&merged_bytes));
    assert!(std::fs::read_dir(&dir.0).unwrap().all(|entry| !entry.unwrap().path().to_string_lossy().ends_with(".part")));
}

#[test]
fn publish_blocked_sheet_is_reported_not_dropped() {
    let dir = TempDir::new();
    let (mut s, mut handles) = setup(9);
    title_only_block(s.doc_mut().unwrap(), "AMBIGUOUS");
    handles.push(place(&mut s, "AMBIGUOUS", [30_000.0, 0.0], 1.0, 0.0, false, &[]).hex());
    let plan = scan(&mut s, &handles);
    assert_eq!(plan["sheets"][9]["status"], "BLOCKED");
    assert!(plan["sheets"][9]["issues"].as_array().unwrap().iter().any(|issue| issue["code"] == codes::FRAME_AMBIGUOUS));
    let path = dir.path("blocked.pdf");
    let error = s.execute("ubplot.publish", &json!({"path": path})).unwrap_err().to_string();
    assert!(error.contains("S-010@10"), "{error}");
    assert!(!path.exists());
    let result = s.execute("ubplot.publish", &json!({"output": "split", "path": path})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 9);
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    assert_eq!(result["failed"][0]["sheetKey"], "S-010@10");
    assert_eq!(result["plan"]["sheets"][9]["status"], "BLOCKED");
}

#[test]
fn publish_stale_plan_rejected() {
    let dir = TempDir::new();
    let (mut s, handles) = setup(1);
    let plan = scan(&mut s, &handles);
    s.execute("move", &json!({"handles": handles, "delta": [10, 20]})).unwrap();
    let error = s.execute("ubplot.publish", &json!({"plan": plan, "path": dir.path("stale.pdf")})).unwrap_err().to_string();
    assert!(error.contains("PLAN_STALE"), "{error}");
    assert!(!dir.path("stale.pdf").exists());
}

#[test]
fn publish_output_is_source_rejected() {
    let dir = TempDir::new();
    let source = dir.path("drawing.dwg");
    let original = b"original drawing bytes";
    std::fs::write(&source, original).unwrap();
    let (mut s, handles) = setup(1);
    s.state_mut().unwrap().path = Some(source.to_string_lossy().into_owned());
    scan(&mut s, &handles);
    std::fs::create_dir(dir.path("child")).unwrap();
    let aliases = vec![source.clone(), dir.path("./drawing.dwg"), dir.path("child/../drawing.dwg")];
    #[cfg(target_os = "windows")]
    let aliases = {
        let mut aliases = aliases;
        aliases.push(PathBuf::from(source.to_string_lossy().to_uppercase()));
        aliases
    };
    #[cfg(unix)]
    let aliases = {
        let mut aliases = aliases;
        let alias = dir.path("source-alias.pdf");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        aliases.push(alias);
        aliases
    };
    for alias in aliases {
        let error = s.execute("ubplot.publish", &json!({"path": alias})).unwrap_err().to_string();
        assert!(error.contains("OUTPUT_IS_SOURCE"), "{error}");
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
    let output = dir.path("sheet.pdf");
    let split_source = dir.path("sheet-S-001.pdf");
    std::fs::write(&split_source, original).unwrap();
    s.state_mut().unwrap().path = Some(split_source.to_string_lossy().into_owned());
    let error = s.execute("ubplot.publish", &json!({"output": "split", "path": output})).unwrap_err().to_string();
    assert!(error.contains("OUTPUT_IS_SOURCE"), "{error}");
    assert_eq!(std::fs::read(split_source).unwrap(), original);
}

#[test]
fn publish_temp_name_uses_job_id() {
    let dir = TempDir::new();
    let (mut s, handles) = setup(1);
    let plan = scan(&mut s, &handles);
    let output = dir.path("out.pdf");
    let temporary = PathBuf::from(format!("{}.{}.part", output.display(), plan["job_id"].as_str().unwrap()));
    std::fs::create_dir(&temporary).unwrap();
    assert!(s.execute("ubplot.publish", &json!({"path": output})).is_err());
    assert!(!output.exists());
    assert!(temporary.is_dir());
}

#[test]
fn split_file_names_are_safe_and_unique() {
    let dir = TempDir::new();
    let (mut s, handles) = setup(4);
    let mut plan: SheetPlan = serde_json::from_value(scan(&mut s, &handles)).unwrap();
    for (sheet, id) in plan.sheets.iter_mut().zip(["KT/01", "CON", "Mặt bằng", "KT/01"]) {
        sheet.sheet_id = id.into();
    }
    let result = s.execute("ubplot.publish", &json!({"plan": plan, "output": "split", "path": dir.path("set.pdf")})).unwrap();
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 4);
    let names: std::collections::BTreeSet<_> = files.iter().map(|file| file["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 4);
    assert!(names.iter().all(|name| !name.contains(['/', '\\', ':'])));
    assert!(names.contains("set-CON-2.pdf"));
    assert!(names.contains("set-Mặt bằng.pdf"));
    for (file, id) in files.iter().zip(["KT/01", "CON", "Mặt bằng", "KT/01"]) {
        assert_eq!(labels(&pages(&file_bytes(file))), [id]);
    }
}

#[test]
fn publish_block_outline_in_layout_and_model() {
    let (mut s, handles) = setup(1);
    let model = Handle::parse_hex(&handles[0]).unwrap();
    let entity = s.doc().unwrap().entity(model).unwrap().clone();
    let paper = s.doc_mut().unwrap().add(&Space::Paper("Layout1".into()), Common::default(), entity.kind.clone()).unwrap();
    scan(&mut s, &[model.hex(), paper.hex()]);
    let result = publish(&mut s);
    assert_eq!(pages(&file_bytes(&result["files"][0])).len(), 2);
}

#[test]
fn publish_hostile_params_are_errors_without_writes() {
    let (mut s, handles) = setup(1);
    let mut plan = scan(&mut s, &handles);
    for params in [json!({"output": "other"}), json!({"path": 7}), json!({"path": ""}), json!({"title": []})] {
        assert!(s.execute("ubplot.publish", &params).is_err());
    }
    plan["job_id"] = json!("../escape");
    assert!(s.execute("ubplot.publish", &json!({"plan": plan})).unwrap_err().to_string().contains("PLAN_INVALID"));
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan.sheets[0].status, SheetStatus::Ready);
}

#[test]
fn pdf_probe_follows_kids_and_inherits_page_properties() {
    let pdf = br"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /P (First) >> 1 << /P <FEFF004D1EB70074> >>] >> >>
endobj
2 0 obj
<< /Type /Pages /Kids [4 0 R 3 0 R] /MediaBox [0 0 100 200] /UserUnit 2 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 60] >>
endobj
4 0 obj
<< /Type /Page /Parent 2 0 R >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF";
    let pdf = pages(pdf);
    assert_eq!(pdf.len(), 2);
    assert_eq!(pdf[0].media_box, [0.0, 0.0, 100.0, 200.0]);
    assert_eq!(pdf[1].media_box, [0.0, 0.0, 50.0, 60.0]);
    assert_eq!(pdf[0].user_unit, 2.0);
    assert_eq!(labels(&pdf), ["First", "Mặt"]);
}

#[test]
#[ignore]
fn write_qa_fixture_dxf() {
    let Some(path) = std::env::var_os("UBPLOT_QA_OUT") else {
        eprintln!("skipped: set UBPLOT_QA_OUT to the synthetic DXF output path");
        return;
    };
    let (s, handles) = setup(100);
    assert_eq!(handles.len(), 100);
    std::fs::write(Path::new(&path), cadcraft_io::write_dxf(s.doc().unwrap())).unwrap();
}
