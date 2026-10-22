#[allow(dead_code)]
mod support;

use std::path::PathBuf;

use cadcraft_doc::{Drawing, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::sheetplan::{SheetPlan, SheetStatus, codes};
use serde_json::{Value, json};
use support::frames::{frame_block, place};

fn approve_warnings(s: &mut Session) {
    let plan = &s.ubplot.last.as_ref().unwrap().plan;
    let approvals: Vec<_> = plan
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|entry| entry.severity == cadcraft_engine::sheetplan::Severity::Warn)
                .map(|entry| json!({"sheetKey": cadcraft_engine::sheetplan::ids::sheet_key(sheet), "code": entry.code}))
        })
        .chain(
            plan.issues
                .iter()
                .filter(|entry| entry.severity == cadcraft_engine::sheetplan::Severity::Warn)
                .map(|entry| json!({"sheetKey": "*", "code": entry.code})),
        )
        .collect();
    s.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap();
}

fn failing_plot(_: &Drawing, _: &Space, options: &Value) -> Result<Vec<u8>, String> {
    for page in options["pages"].as_array().unwrap() {
        let expected = match page["label"].as_str() {
            Some("Paper frame") => Some("Layout1"),
            Some("Model frame") => Some("Model"),
            _ => None,
        };
        if expected.is_some_and(|space| page["space"] != space) {
            return Err("frame page uses the wrong space".into());
        }
    }
    if options["pages"].as_array().unwrap().iter().any(|page| page["label"] == "S-002") {
        return Err("synthetic render error for S-002".into());
    }
    Ok(b"test plot bytes".to_vec())
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn render_failure_marks_failed_and_writes_nothing() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(failing_plot),
    });
    let dir = TempDir(std::env::temp_dir().join(format!("ubplot-render-failure-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "A1", 841.0, 594.0);
    let handles: Vec<_> = (0..10).map(|index| place(&mut s, "A1", [f64::from(index) * 2000.0, 0.0], 1.0, 0.0, false, &[]).hex()).collect();
    let plan = s.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap();
    let merged = dir.0.join("merged.pdf");
    let error = s.execute("ubplot.publish", &json!({"path": merged})).unwrap_err().to_string();
    assert!(error.contains("synthetic render error for S-002"), "{error}");
    let (_, returned) = error.split_once("plan=").unwrap();
    let returned: SheetPlan = serde_json::from_str(returned).unwrap();
    assert_eq!(returned.sheets.len(), 10);
    for sheet in &returned.sheets {
        assert_eq!(sheet.status, SheetStatus::Failed);
        assert!(sheet.issues.iter().any(|issue| issue.code == codes::RENDER_FAILED && issue.message.contains("S-002")));
    }
    assert_eq!(returned, s.ubplot.last.as_ref().unwrap().plan);
    assert!(!merged.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    let result = s.execute("ubplot.publish", &json!({"plan": plan, "output": "split", "path": dir.0.join("split.pdf")})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 9);
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    assert_eq!(result["failed"][0]["sheetKey"], "S-002@2");
    assert_eq!(result["failed"][0]["issues"].as_array().unwrap().last().unwrap()["code"], codes::RENDER_FAILED);
    let plan = &s.ubplot.last.as_ref().unwrap().plan;
    assert_eq!(plan.sheets[1].status, SheetStatus::Failed);
    assert_eq!(plan.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Done).count(), 9);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 9);
    assert!(!dir.0.join("split-S-002.pdf").exists());
    assert!(std::fs::read_dir(&dir.0).unwrap().all(|entry| !entry.unwrap().path().to_string_lossy().ends_with(".part")));
}

#[test]
fn publish_resolves_block_space_per_page() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(failing_plot),
    });
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "A1", 841.0, 594.0);
    let model = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[("sheet_no", "Model frame")]);
    let paper = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[("sheet_no", "Paper frame")]);
    let entity = s.doc_mut().unwrap().remove_entity(paper).unwrap();
    let paper = s.doc_mut().unwrap().add(&Space::Paper("Layout1".into()), entity.common.clone(), entity.kind.clone()).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "block", "handles": [model.hex(), paper.hex()]})).unwrap();
    approve_warnings(&mut s);
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    assert!(result["failed"].as_array().unwrap().is_empty());
    assert!(result["plan"]["sheets"].as_array().unwrap().iter().all(|sheet| sheet["status"] == "DONE"));
}
