#[allow(dead_code)]
mod support;

use std::sync::Arc;

use cadcraft_doc::{EntityKind, TextStyle};
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::cmd::ubplot::set_clock;
use cadcraft_engine::sheetplan::approval::{approve, fingerprint};
use cadcraft_engine::sheetplan::codes::{self, issue};
use cadcraft_engine::sheetplan::ids::sheet_key;
use cadcraft_engine::sheetplan::{Severity, SheetPlan, SheetStatus};
use cadcraft_engine::{EngineError, Session};
use serde_json::{Value, json};
use support::frames::{frame_block, place};

fn setup() -> (Session, SheetPlan) {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, options| cadcraft_io::plotter::plot(d, space, options).map_err(|error| error.to_string())),
    });
    let mut s = Session::new();
    let value = s.execute("ubplot.scan", &json!({"mode": "window", "windows": [[[0, 0], [420, 297]]], "scale": "fit"})).unwrap();
    (s, serde_json::from_value(value).unwrap())
}

fn warning_plan() -> (Session, SheetPlan) {
    let (s, mut plan) = setup();
    let sheet = &mut plan.sheets[0];
    sheet.issues.push(issue(Severity::Warn, codes::FRAME_OVERLAP, "Frame overlaps another selected frame.", Some(&sheet.sheet_id)));
    (s, plan)
}

fn error_message(error: EngineError) -> String {
    match error {
        EngineError::BadParams { msg, .. } => msg,
        other => other.to_string(),
    }
}

fn approved_plan(s: &mut Session, plan: &SheetPlan, by: &str) -> SheetPlan {
    let items: Vec<_> = plan
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|entry| entry.severity == Severity::Warn)
                .map(|entry| json!({"sheetKey": sheet_key(sheet), "code": entry.code}))
        })
        .chain(plan.issues.iter().filter(|entry| entry.severity == Severity::Warn).map(|entry| json!({"sheetKey": "*", "code": entry.code})))
        .collect();
    serde_json::from_value(s.execute("ubplot.approve", &json!({"plan": plan, "approvals": items, "by": by})).unwrap()).unwrap()
}

#[test]
fn warn_approval_records_audit_and_unblocks_publish() {
    set_clock(|| 123_456);
    let (mut s, plan) = warning_plan();
    let key = sheet_key(&plan.sheets[0]);
    let error = error_message(s.execute("ubplot.publish", &json!({"plan": plan})).unwrap_err());
    assert!(error.starts_with("WARN_NOT_APPROVED:"), "{error}");
    assert!(error.contains(&format!("{key}:{}", codes::FRAME_OVERLAP)));
    let approved = approved_plan(&mut s, &plan, "user");
    assert_eq!(approved.audit.len(), 1);
    let audit = &approved.audit[0];
    assert_eq!(audit.at_ms, 123_456);
    assert_eq!(audit.by, "user");
    assert_eq!(audit.action, "approve");
    assert_eq!(audit.sheet_key, key);
    assert_eq!(audit.code, codes::FRAME_OVERLAP);
    assert_eq!(audit.fingerprint, fingerprint(&plan.sheets[0].issues[0], &key));
    assert_eq!(approved.sheets[0].status, SheetStatus::Ready);
    assert!(approved.sheets[0].issues[0].approved_by_user);
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, approved);
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    assert_eq!(result["plan"]["sheets"][0]["status"], "DONE");
}

#[test]
fn block_is_never_approvable_even_by_json() {
    let (mut s, mut plan) = setup();
    let sheet = &mut plan.sheets[0];
    sheet.issues.push(issue(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame is ambiguous.", Some(&sheet.sheet_id)));
    let key = sheet_key(sheet);
    let last = s.ubplot.last.as_ref().unwrap().plan.clone();
    for by in ["user", "agent:claude"] {
        let error = error_message(
            s.execute("ubplot.approve", &json!({"plan": plan, "approvals": [{"sheetKey": key, "code": codes::FRAME_AMBIGUOUS}], "by": by}))
                .unwrap_err(),
        );
        assert!(error.starts_with("BLOCK_NOT_APPROVABLE"), "{error}");
        assert_eq!(s.ubplot.last.as_ref().unwrap().plan, last);
        let mut forged = plan.clone();
        forged.sheets[0].issues[0].approved_by_user = true;
        forged.audit.push(cadcraft_engine::sheetplan::AuditEntry {
            at_ms: 1,
            by: by.into(),
            action: "approve".into(),
            sheet_key: key.clone(),
            code: codes::FRAME_AMBIGUOUS.into(),
            fingerprint: fingerprint(&forged.sheets[0].issues[0], &key),
        });
        let path = std::env::temp_dir().join(format!("ubplot-forged-block-{}.pdf", std::process::id()));
        assert!(!path.exists());
        let error = error_message(s.execute("ubplot.publish", &json!({"plan": forged, "path": path})).unwrap_err());
        assert!(error.starts_with("BLOCK_NOT_APPROVABLE"), "{error}");
        assert!(!path.exists());
        assert_eq!(s.ubplot.last.as_ref().unwrap().plan, last);
    }
}

#[test]
fn warn_flag_without_audit_is_rejected() {
    let (mut s, mut plan) = warning_plan();
    plan.sheets[0].issues[0].approved_by_user = true;
    let error = error_message(s.execute("ubplot.publish", &json!({"plan": plan})).unwrap_err());
    assert!(error.starts_with("APPROVAL_WITHOUT_AUDIT"), "{error}");
    let mut forged = approved_plan(&mut s, &plan, "user");
    forged.sheets[0].issues[0].message.push_str(" Changed after approval.");
    let error = error_message(s.execute("ubplot.publish", &json!({"plan": forged})).unwrap_err());
    assert!(error.starts_with("APPROVAL_WITHOUT_AUDIT"), "{error}");
}

#[test]
fn agent_can_approve_warn_and_is_audited_as_agent() {
    let (mut s, mut plan) = setup();
    plan.issues.push(issue(Severity::Warn, codes::FONT_FALLBACK, "missing.ttf is drawn with Alternate", None));
    let approved = approved_plan(&mut s, &plan, "agent:claude");
    assert_eq!(approved.audit.len(), 1);
    assert_eq!(approved.audit[0].by, "agent:claude");
    assert_eq!(approved.audit[0].sheet_key, "*");
    assert_eq!(approved.audit[0].fingerprint, fingerprint(plan.issues.last().unwrap(), "*"));
    assert!(approved.issues.last().unwrap().approved_by_user);
    assert!(s.execute("ubplot.publish", &json!({})).is_ok());
}

struct FontSettings {
    map: Vec<(String, String)>,
    alt: String,
    fallback: String,
}

impl Drop for FontSettings {
    fn drop(&mut self) {
        cadcraft_fonts::set_font_map(self.map.clone());
        cadcraft_fonts::ttf::set_font_alt(&self.alt);
        cadcraft_fonts::ttf::set_fallback_fonts(&self.fallback);
        cadcraft_fonts::ttf::remove_user_font("zz-approval-alt-one");
        cadcraft_fonts::ttf::remove_user_font("zz-approval-alt-two");
    }
}

#[test]
fn approval_expires_when_issue_message_changes() {
    let _serial = cadcraft_engine::cmd::fonts::test_lock();
    let _settings =
        FontSettings { map: cadcraft_fonts::font_map(), alt: cadcraft_fonts::ttf::font_alt(), fallback: cadcraft_fonts::ttf::fallback_fonts() };
    cadcraft_fonts::set_font_map([]);
    cadcraft_fonts::ttf::set_fallback_fonts("");
    // Font status only needs a parseable directory; no glyph data or installed fonts are required.
    let directory = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for name in ["zz-approval-alt-one.ttf", "zz-approval-alt-two.ttf"] {
        cadcraft_fonts::ttf::add_user_font(name, directory.clone()).unwrap();
    }
    cadcraft_fonts::ttf::set_font_alt("zz-approval-alt-one.ttf");
    let (mut s, _) = setup();
    let d = s.doc_mut().unwrap();
    d.text_styles.push(TextStyle { name: "Missing".into(), font: "zz-approval-missing.ttf".into(), family: String::new(), ..Default::default() });
    frame_block(d, "FRAME", 420.0, 297.0);
    let block = Arc::make_mut(d.blocks.get_mut("FRAME").unwrap());
    let text = block.entities.iter().find(|entity| matches!(&entity.kind, EntityKind::Text(_))).unwrap().handle;
    block.entities.modify(text, |entity| {
        if let EntityKind::Text(text) = &mut entity.kind {
            text.style = "Missing".into();
        }
    });
    let handle = place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let plan: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()]})).unwrap()).unwrap();
    assert!(plan.issues.iter().any(|entry| entry.code == codes::FONT_FALLBACK && entry.message.contains("zz-approval-alt-one.ttf")));
    let approved = approved_plan(&mut s, &plan, "user");
    let unchanged: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap()).unwrap();
    assert!(unchanged.issues.iter().find(|entry| entry.code == codes::FONT_FALLBACK).unwrap().approved_by_user);
    assert_eq!(unchanged.audit, approved.audit);
    s.execute("fonts", &json!({"alt": "zz-approval-alt-two.ttf"})).unwrap();
    let refreshed: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap()).unwrap();
    let warning = refreshed.issues.iter().find(|entry| entry.code == codes::FONT_FALLBACK).unwrap();
    assert!(warning.message.contains("zz-approval-alt-two.ttf"));
    assert!(!warning.approved_by_user);
    assert_eq!(refreshed.audit, approved.audit);
    assert_eq!(refreshed.sheets[0].status, SheetStatus::NeedsPreview);
    let error = error_message(s.execute("ubplot.publish", &json!({})).unwrap_err());
    assert!(error.starts_with("WARN_NOT_APPROVED:"), "{error}");
}

#[test]
fn approval_survives_rescan_when_unchanged() {
    let (mut s, _) = setup();
    frame_block(s.doc_mut().unwrap(), "FRAME", 420.0, 297.0);
    let handles = [place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[]).hex(), place(&mut s, "FRAME", [200.0, 0.0], 1.0, 0.0, false, &[]).hex()];
    let params = json!({"mode": "block", "handles": handles});
    let plan: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &params).unwrap()).unwrap();
    assert!(plan.sheets.iter().all(|sheet| sheet.issues.iter().any(|entry| entry.code == codes::FRAME_OVERLAP)));
    let approved = approved_plan(&mut s, &plan, "user");
    for params in [json!({"mode": "last"}), params] {
        let refreshed: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &params).unwrap()).unwrap();
        assert_eq!(refreshed.audit, approved.audit);
        assert!(refreshed.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready));
        assert!(
            refreshed
                .sheets
                .iter()
                .flat_map(|sheet| &sheet.issues)
                .filter(|entry| entry.severity == Severity::Warn)
                .all(|entry| entry.approved_by_user)
        );
    }
    let refreshed: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "last"})).unwrap()).unwrap();
    let imported: SheetPlan = serde_json::from_value(s.execute("ubplot.scan", &json!({"mode": "plan", "plan": refreshed})).unwrap()).unwrap();
    assert_eq!(imported.audit, approved.audit);
    assert!(imported.sheets.iter().all(|sheet| sheet.status == SheetStatus::Ready));
    let mut changed = refreshed.clone();
    changed.sheets[0].issues.iter_mut().find(|entry| entry.code == codes::FRAME_OVERLAP).unwrap().message.push_str(" Changed.");
    cadcraft_engine::sheetplan::approval::carry_over(&refreshed, &mut changed);
    assert!(!changed.sheets[0].issues.iter().find(|entry| entry.code == codes::FRAME_OVERLAP).unwrap().approved_by_user);
    assert_eq!(changed.audit, approved.audit);
}

#[test]
fn approve_is_atomic() {
    let (mut s, mut plan) = warning_plan();
    let sheet = &mut plan.sheets[0];
    sheet.issues.push(issue(Severity::Warn, codes::FRAME_MIRRORED, "Frame is mirrored.", Some(&sheet.sheet_id)));
    sheet.issues.push(issue(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame is ambiguous.", Some(&sheet.sheet_id)));
    let key = sheet_key(sheet);
    let original = plan.clone();
    let items: Vec<_> = [codes::FRAME_OVERLAP, codes::FRAME_MIRRORED, codes::FRAME_AMBIGUOUS].map(|code| (key.clone(), code.into())).into();
    assert!(approve(&mut plan, &items, "user", 1).unwrap_err().starts_with("BLOCK_NOT_APPROVABLE"));
    assert_eq!(plan, original);
    let last = s.ubplot.last.as_ref().unwrap().plan.clone();
    let approvals: Vec<_> = items.iter().map(|(key, code)| json!({"sheetKey": key, "code": code})).collect();
    assert!(s.execute("ubplot.approve", &json!({"plan": plan, "approvals": approvals, "by": "user"})).is_err());
    assert_eq!(s.ubplot.last.as_ref().unwrap().plan, last);
}

#[test]
fn hostile_approvals() {
    let (mut s, plan) = warning_plan();
    let key = sheet_key(&plan.sheets[0]);
    let item = json!({"sheetKey": key, "code": codes::FRAME_OVERLAP});
    let last = s.ubplot.last.as_ref().unwrap().plan.clone();
    for params in [
        json!({"plan": plan, "approvals": [item], "by": ""}),
        json!({"plan": plan, "approvals": [item], "by": "a".repeat(65)}),
        json!({"plan": plan, "approvals": [item], "by": format!("agent:{}", "x".repeat(59))}),
        json!({"plan": plan, "approvals": [item], "by": "agent:"}),
        json!({"plan": plan, "approvals": [item], "by": "someone"}),
        json!({"plan": plan, "approvals": vec![item.clone(); 10_001], "by": "user"}),
        json!({"plan": plan, "approvals": [{"sheetKey": "unknown", "code": codes::FRAME_OVERLAP}], "by": "user"}),
        json!({"plan": plan, "approvals": [{"sheetKey": key, "code": "unknown"}], "by": "user"}),
        json!({"plan": plan, "approvals": [item], "split": {}, "by": "user"}),
        json!({"plan": plan, "approvals": "wrong type", "by": "user"}),
    ] {
        assert!(s.execute("ubplot.approve", &params).is_err(), "{params}");
        assert_eq!(s.ubplot.last.as_ref().unwrap().plan, last);
    }
    for severity in [Severity::Info, Severity::Pass] {
        let mut invalid = plan.clone();
        invalid.sheets[0].issues[0].severity = severity;
        let original = invalid.clone();
        assert!(approve(&mut invalid, &[(key.clone(), codes::FRAME_OVERLAP.into())], "user", 0).is_err());
        assert_eq!(invalid, original);
    }
    let mut full = approved_plan(&mut s, &plan, "user");
    full.audit = vec![full.audit[0].clone(); 100_000];
    let original = full.clone();
    assert!(approve(&mut full, &[(key, codes::FRAME_OVERLAP.into())], "user", 0).is_err());
    assert_eq!(full, original);
    let oversized = json!({"plan": {"audit": vec![Value::Null; 100_001], "sheets": []}, "approvals": [], "by": "user"});
    assert!(s.execute("ubplot.approve", &oversized).is_err());
}
