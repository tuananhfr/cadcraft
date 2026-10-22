#[allow(dead_code)]
mod support;

use std::sync::Once;

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{EntityCodec, IoHooks, base64_decode, set_entity_codec, set_io};
use cadcraft_engine::lisp::value::{Value as LispValue, to_text};
use cadcraft_engine::sheetplan::{FrameRegistry, Severity, SheetPlan, SheetStatus, TemplateStatus, ids};
use serde_json::{Value, json};
use support::frames::{anon_dynamic, frame_block, place};
use support::lpc_fixture::lpc_like_drawing;

const INSERTS: &str = r#"(ssget "X" '((0 . "INSERT")))"#;

fn setup() -> Session {
    static HOOKS: Once = Once::new();
    HOOKS.call_once(|| {
        set_io(IoHooks {
            read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
            write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
            plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
        });
        set_entity_codec(EntityCodec { groups: cadcraft_io::entity_groups, entity: cadcraft_io::entity_from_groups });
        cadcraft_engine::cmd::ubplot::set_clock(|| 42);
    });
    let mut session = Session::new();
    session.open_drawing(lpc_like_drawing(), "Synthetic", None);
    let templates: Vec<_> = [("a1", "KHUNG_A1", 841.0, 594.0), ("a3", "KHUNG_A3", 420.0, 297.0)]
        .into_iter()
        .map(|(id, name, width, height)| {
            json!({
                "id": id, "revision": "1", "block_names": [name],
                "geometry": [{"kind": "local_rect", "min": [0, 0], "max": [width, height]}],
                "attributes": {"sheet_id": ["SO_BV"], "title": ["TEN_BV"], "revision": ["REV"]},
                "media_mm": [[width, height]], "version": 1, "status": "approved"
            })
        })
        .collect();
    session.ubplot.registry = FrameRegistry::from_json(&json!({"schema_version": "2", "templates": templates}).to_string()).unwrap();
    session
}

fn eval(session: &mut Session, code: &str) -> LispValue {
    let result = session.execute("lisp", &json!({"code": format!("(setq *result* {code})")})).unwrap();
    assert!(result.get("error").is_none(), "{result}");
    session.lisp_var("*result*")
}

fn eval_json(session: &mut Session, code: &str) -> Value {
    let LispValue::Str(text) = eval(session, code) else { panic!("expected JSON string") };
    serde_json::from_str(&text).unwrap()
}

fn error(session: &mut Session, code: &str) -> String {
    let result = session.execute("lisp", &json!({"code": code})).unwrap();
    result["error"].as_str().unwrap_or_else(|| panic!("expected LISP error: {result}")).to_string()
}

fn lisp_string(text: &str) -> String {
    to_text(&LispValue::str(text), true)
}

fn inserts(session: &Session) -> Vec<Handle> {
    session.doc().unwrap().model.iter().filter(|entity| matches!(entity.kind, EntityKind::Insert(_))).map(|entity| entity.handle).collect()
}

fn approve_warnings(session: &mut Session) -> Value {
    let plan = &session.ubplot.last.as_ref().unwrap().plan;
    let approvals: Vec<_> = plan
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|issue| issue.severity == Severity::Warn)
                .map(|issue| json!({"sheetKey": ids::sheet_key(sheet), "code": issue.code}))
        })
        .chain(plan.issues.iter().filter(|issue| issue.severity == Severity::Warn).map(|issue| json!({"sheetKey": "*", "code": issue.code})))
        .collect();
    session.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap()
}

#[test]
fn getpropertyvalue_effective_name_of_dynamic_reference() {
    let mut session = setup();
    let fixture_dynamic = inserts(&session)[1];
    let code = format!(r#"(getpropertyvalue (handent "{}") "EffectiveName")"#, fixture_dynamic.hex());
    assert_eq!(to_text(&eval(&mut session, &code), true), r#""KHUNG_A1""#);
    frame_block(session.doc_mut().unwrap(), "KHUNG_DYN", 841.0, 594.0);
    anon_dynamic(session.doc_mut().unwrap(), "KHUNG_DYN", "*U12");
    let handle = place(&mut session, "*U12", [20.0, 30.0], 1.0, 90.0, false, &[]);
    let ename = format!(r#"(handent "{}")"#, handle.hex());
    let value = eval(&mut session, &format!(r#"(getpropertyvalue {ename} "EffectiveName")"#));
    assert_eq!(to_text(&value, true), r#""KHUNG_DYN""#);
    assert_eq!(to_text(&eval(&mut session, &format!(r#"(getpropertyvalue {ename} "BlockName")"#)), true), r#""*U12""#);
    let angle = eval(&mut session, &format!(r#"(getpropertyvalue {ename} "Rotation")"#)).as_number().unwrap();
    assert!((angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!(eval(&mut session, &format!(r#"(getpropertyvalue {ename} "Position")"#)).as_point().unwrap(), [20.0, 30.0, 0.0]);
    assert_eq!(to_text(&eval(&mut session, &format!(r#"(getpropertyvalue {ename} "Layer")"#)), true), r#""0""#);
    let value = eval(&mut session, &format!(r#"(getpropertyvalue {ename} "Handle")"#));
    assert_eq!(to_text(&value, true), lisp_string(&handle.hex()));
}

#[test]
fn getpropertyvalue_unknown_property_is_an_error() {
    let mut session = setup();
    let handle = inserts(&session)[0];
    let code = format!(r#"(getpropertyvalue (handent "{}") "Foo")"#, handle.hex());
    assert_eq!(error(&mut session, &code), "bad argument value: property FOO");
    eval(&mut session, "(defun *error* (message) (setq *caught* message))");
    assert_eq!(error(&mut session, &code), "bad argument value: property FOO");
    assert_eq!(to_text(&session.lisp_var("*caught*"), true), r#""bad argument value: property FOO""#);
    session.execute("line", &json!({"points": [[0, 0], [10, 0]]})).unwrap();
    assert_eq!(error(&mut session, r#"(getpropertyvalue (entlast) "BlockName")"#), "bad argument value: property BLOCKNAME");
}

#[test]
fn ubplot_attribs_lists_tags_in_order() {
    let mut session = setup();
    let handle = inserts(&session)[0];
    let value = eval(&mut session, &format!(r#"(ubplot-attribs (handent "{}"))"#, handle.hex()));
    assert_eq!(
        to_text(&value, true),
        r#"(("SO_BV" . "KT-01") ("TEN_BV" . "Synthetic drawing") ("TY_LE" . "1:1") ("REV" . "R1") ("NGAY" . "2026-01-01"))"#
    );
    let empty = place(&mut session, "KHUNG_A1", [9000.0, 0.0], 1.0, 0.0, false, &[]);
    assert!(eval(&mut session, &format!(r#"(ubplot-attribs (handent "{}"))"#, empty.hex())).is_nil());
    session.execute("line", &json!({"points": [[0, 0], [10, 0]]})).unwrap();
    assert!(eval(&mut session, "(ubplot-attribs (entlast))").is_nil());
}

#[test]
fn lisp_scan_equals_native_scan() {
    let mut session = setup();
    let handles: Vec<_> = inserts(&session).iter().map(|handle| handle.hex()).collect();
    let revision = session.state().unwrap().revision;
    let lisp: SheetPlan = serde_json::from_value(eval_json(&mut session, &format!("(ubplot-scan {INSERTS})"))).unwrap();
    let native: SheetPlan = serde_json::from_value(session.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap()).unwrap();
    assert_eq!(lisp, native);
    assert_eq!(lisp.sheets.len(), 3);
    assert_eq!(lisp.sheets.iter().map(|sheet| sheet.sheet_id.as_str()).collect::<Vec<_>>(), ["KT-01", "KT-02", "KT-03"]);
    assert_eq!(session.state().unwrap().revision, revision);
    let settings = json!({"paper": "A1", "scale": "1:2"});
    let code = format!("(ubplot-scan {INSERTS} {})", lisp_string(&settings.to_string()));
    let lisp: SheetPlan = serde_json::from_value(eval_json(&mut session, &code)).unwrap();
    let native: SheetPlan =
        serde_json::from_value(session.execute("ubplot.scan", &json!({"mode": "block", "handles": handles, "paper": "A1", "scale": "1:2"})).unwrap())
            .unwrap();
    assert_eq!(lisp, native);
    let message = error(&mut session, &format!("(ubplot-scan {INSERTS} {})", lisp_string(r#"{"scale":"invalid"}"#)));
    let native = session.execute("ubplot.scan", &json!({"mode": "block", "handles": handles, "scale": "invalid"})).unwrap_err().to_string();
    assert_eq!(message, native);
}

#[test]
fn ubplot_publish_from_lisp_returns_manifest() {
    let mut session = setup();
    eval_json(&mut session, &format!("(ubplot-scan {INSERTS})"));
    let approved = approve_warnings(&mut session);
    let plan: SheetPlan = serde_json::from_value(approved).unwrap();
    let code = format!("(ubplot-publish {} nil)", lisp_string(&serde_json::to_string(&plan).unwrap()));
    let manifest = eval_json(&mut session, &code);
    assert!(manifest["failed"].as_array().unwrap().is_empty());
    let published: SheetPlan = serde_json::from_value(manifest["plan"].clone()).unwrap();
    assert_eq!(published.sheets.len(), 3);
    assert!(published.sheets.iter().all(|sheet| sheet.status == SheetStatus::Done));
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let bytes = base64_decode(files[0]["data"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(support::pdf_probe::pages(&bytes).len(), 3);
    assert_eq!(session.ubplot.last.as_ref().unwrap().plan, published);
    assert!(session.running.is_none());
}

#[test]
fn ubplot_scan_refused_inside_running_command() {
    let mut session = setup();
    let before = session.doc().unwrap().clone();
    session.start("line").unwrap();
    assert!(session.running.is_some());
    assert_eq!(error(&mut session, &format!("(ubplot-scan {INSERTS})")), "ubplot-scan can't run while a command is active");
    assert!(session.ubplot.last.is_none());
    session.start("line").unwrap();
    assert_eq!(error(&mut session, r#"(ubplot-publish "{}")"#), "ubplot-scan can't run while a command is active");
    assert_eq!(cadcraft_io::write_dxf(session.doc().unwrap()), cadcraft_io::write_dxf(&before));
}

#[test]
fn hostile_ubplot_args_never_panic() {
    let mut session = setup();
    let first = inserts(&session)[0];
    let mut hostile = vec![
        "(ubplot-scan 5)".to_string(),
        r#"(ubplot-publish "{")"#.to_string(),
        "(getpropertyvalue nil nil)".to_string(),
        "(getpropertyvalue)".to_string(),
        "(getpropertyvalue 5 6 7)".to_string(),
        "(ubplot-attribs nil)".to_string(),
        "(ubplot-attribs)".to_string(),
        "(ubplot-attribs 1 2)".to_string(),
        "(ubplot-scan)".to_string(),
        "(ubplot-scan nil)".to_string(),
        "(ubplot-publish)".to_string(),
        "(ubplot-publish 5)".to_string(),
        r#"(ubplot-publish "null")"#.to_string(),
        r#"(ubplot-publish "{}" 5)"#.to_string(),
        "(ubplot-templates 1)".to_string(),
        format!(r#"(getpropertyvalue (handent "{}") nil)"#, first.hex()),
        format!("(ubplot-scan {INSERTS} 5)"),
        format!(r#"(ubplot-scan {INSERTS} "{{")"#),
        format!(r#"(ubplot-scan {INSERTS} "[]")"#),
    ];
    let nested = format!("{}0{}", "[".repeat(256), "]".repeat(256));
    hostile.push(format!("(ubplot-publish {})", lisp_string(&nested)));
    session.execute("line", &json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let deleted = session.doc().unwrap().model.handles().last().copied().unwrap();
    eval(&mut session, &format!(r#"(setq *deleted* (handent "{}"))"#, deleted.hex()));
    eval(&mut session, "(entdel *deleted*)");
    hostile.push(r#"(getpropertyvalue *deleted* "Handle")"#.into());
    hostile.push("(ubplot-attribs *deleted*)".into());
    for code in hostile {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| error(&mut session, &code)));
        let message = result.unwrap_or_else(|_| panic!("panicked for {code}"));
        assert!(!message.contains("internal error"), "{code}: {message}");
        assert!(!session.lisp_running());
    }
    let value = eval(&mut session, "(+ 1 2)");
    assert_eq!(to_text(&value, true), "3");
}

#[test]
fn ubplot_templates_lists_only_approved_versioned_templates() {
    let mut session = setup();
    session.ubplot.registry.templates[0].version = 3;
    let mut draft = session.ubplot.registry.templates[0].clone();
    draft.id = "draft".into();
    draft.status = TemplateStatus::Draft;
    let mut old = draft.clone();
    old.id = "old".into();
    old.status = TemplateStatus::Superseded;
    old.superseded_by = Some("a1@3".into());
    session.ubplot.registry.templates.extend([draft, old]);
    let result = eval(&mut session, "(ubplot-templates)").to_vec();
    assert_eq!(result.len(), 2);
    assert_eq!(to_text(&result[0].car(), true), r#""a1@3""#);
    assert_eq!(
        to_text(&result[0], true),
        r#"("a1@3" ("names" "KHUNG_A1") ("frame" (0.0 0.0) (841.0 0.0) (841.0 594.0) (0.0 594.0)) ("paper" 841.0 594.0 T) ("sheet_id_tags" "SO_BV"))"#
    );
    session.ubplot.registry = FrameRegistry::builtin();
    let result = eval(&mut session, "(ubplot-templates)");
    assert_eq!(result.to_vec().len(), 1);
    assert!(to_text(&result, true).contains(r#"("generic@1" ("names" "*") ("frame") ("paper")"#));
    session.ubplot.registry.templates.clear();
    assert!(eval(&mut session, "(ubplot-templates)").is_nil());
}
