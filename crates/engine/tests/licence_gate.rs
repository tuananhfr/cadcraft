//! The edit policy gates content-changing commands only; files, view, plot and settings stay open.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use cadcraft_engine::cmd::command_specs;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::{EntityKind, Space};
use cadcraft_engine::{EditPolicy, EngineError, Session, find_command, is_view_only};
use serde_json::{Value, json};

struct Deny;

impl EditPolicy for Deny {
    fn check(&self) -> std::result::Result<(), String> {
        Err("no licence".into())
    }
}

fn denied() -> Session {
    let mut s = Session::new();
    s.set_edit_policy(Box::new(Deny));
    s
}

fn unlicensed(r: &Result<Value, EngineError>) -> bool {
    matches!(r, Err(EngineError::Unlicensed(_)))
}

fn temp_path(name: &str) -> String {
    std::env::temp_dir().join(format!("cadcraft-licence-{}-{name}", std::process::id())).to_string_lossy().to_string()
}

#[test]
fn deny_blocks_json_edit() {
    let mut s = denied();
    let r = s.execute("circle", &json!({"center": [0, 0], "radius": 1}));
    assert!(unlicensed(&r), "{r:?}");
    assert_eq!(s.doc().unwrap().model.len(), 0);
    assert_eq!(s.state().unwrap().undo.len(), 0);
}

#[test]
fn deny_blocks_interactive_start() {
    let mut s = denied();
    let r = s.cmdline("LINE");
    assert!(matches!(r, Err(EngineError::Unlicensed(_))), "{r:?}");
    assert!(s.running.is_none());
}

#[test]
fn deny_allows_view_file_and_plot() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Ok(b"drawing".to_vec()),
        plot: Some(|_, _, _| Ok(b"plot".to_vec())),
    });
    let mut s = denied();
    s.execute("line", &json!({"points": [[0, 0], [1, 0]]})).unwrap_err();
    for (id, params) in [
        ("zoom.extents", json!({})),
        ("regen", json!({})),
        ("qsave", json!({"path": temp_path("q.dxf")})),
        ("export", json!({"path": temp_path("e.dxf")})),
        ("plot", json!({})),
        ("setvar", json!({"name": "ORTHOMODE", "value": 1})),
    ] {
        let r = s.execute(id, &params);
        assert!(!unlicensed(&r), "{id}: {r:?}");
        assert!(r.is_ok(), "{id}: {r:?}");
    }
}

#[test]
fn deny_allows_recover_and_audit() {
    let mut s = denied();
    let r = s.execute("audit", &json!({}));
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn deny_layer_state_vs_content() {
    let mut s = Session::new();
    s.execute("layer.new", &json!({"name": "A"})).unwrap();
    s.set_edit_policy(Box::new(Deny));
    for (id, params) in [
        ("layoff", json!({"handles": []})),
        ("layfrz", json!({"handles": []})),
        ("laylck", json!({"handles": []})),
        ("layer.set", json!({"name": "A", "on": false})),
        ("layer.set", json!({"name": "A", "frozen": true, "locked": true})),
    ] {
        let r = s.execute(id, &params);
        assert!(!unlicensed(&r), "{id}: {r:?}");
    }
    for (id, params) in
        [("layer.set", json!({"name": "A", "color": 1})), ("layer.set", json!({"name": "A", "newName": "B"})), ("layer.new", json!({"name": "C"}))]
    {
        let r = s.execute(id, &params);
        assert!(unlicensed(&r), "{id}: {r:?}");
    }
}

#[test]
fn deny_allows_xref_reload_only() {
    let mut s = denied();
    for option in ["reload", "unload"] {
        let r = s.execute("-xref", &json!({"option": option, "names": "none"}));
        assert!(!unlicensed(&r), "{option}: {r:?}");
    }
    let r = s.execute("-xref", &json!({"option": "bind", "names": "none"}));
    assert!(unlicensed(&r), "{r:?}");
}

#[test]
fn deny_allows_undo() {
    let mut s = denied();
    for id in ["u", "undo", "redo"] {
        let r = s.execute(id, &json!({}));
        assert!(!unlicensed(&r), "{id}: {r:?}");
    }
}

#[test]
fn policy_flip_mid_line_finishes() {
    let mut s = Session::new();
    s.cmdline("LINE").unwrap();
    s.cmdline("0,0").unwrap();
    s.set_edit_policy(Box::new(Deny));
    s.cmdline("10,0").unwrap();
    s.cmdline("").unwrap();
    assert!(s.running.is_none());
    assert_eq!(s.doc().unwrap().model.len(), 1);
    let r = s.cmdline("LINE");
    assert!(matches!(r, Err(EngineError::Unlicensed(_))), "{r:?}");
}

#[test]
fn view_only_ids_snapshot() {
    let ids: BTreeSet<&str> = command_specs().iter().filter(|s| is_view_only(s, &Value::Null)).map(|s| s.id).collect();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/view_only_ids.txt");
    let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
    // Opt-in so a normal run can never rewrite the guarded file.
    if std::env::var("UPDATE_VIEW_ONLY_IDS").is_ok_and(|v| v == "1") {
        fs::write(&path, &text).expect("write view_only_ids.txt");
        return;
    }
    let recorded = fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert!(!recorded.is_empty(), "{} is missing; run with UPDATE_VIEW_ONLY_IDS=1", path.display());
    assert_eq!(recorded, text, "view-only ids changed; run with UPDATE_VIEW_ONLY_IDS=1 after review");
    assert!(find_command("layer.set").is_some_and(|s| !is_view_only(s, &Value::Null)));
}

#[test]
fn denied_start_keeps_running_command() {
    let mut s = Session::new();
    s.cmdline("LINE").unwrap();
    s.cmdline("0,0").unwrap();
    s.set_edit_policy(Box::new(Deny));
    let r = s.start("circle");
    assert!(matches!(r, Err(EngineError::Unlicensed(_))), "{r:?}");
    assert!(s.running.is_some(), "the refused command must not cancel LINE");
    s.cmdline("10,0").unwrap();
    s.cmdline("").unwrap();
    assert!(s.running.is_none());
    let lines = s.doc().unwrap().model.iter().filter(|e| matches!(e.kind, EntityKind::Line(_))).count();
    assert_eq!(lines, 1);
}

fn viewport_count(s: &Session, layout: &str) -> usize {
    let space = Space::Paper(layout.into());
    s.doc().unwrap().space(&space).map_or(0, |st| st.iter().filter(|e| matches!(e.kind, EntityKind::Viewport(_))).count())
}

#[test]
fn deny_blocks_layout_viewports_from_vports() {
    let mut s = Session::new();
    s.execute("layout.new", &json!({"name": "Sheet A"})).unwrap();
    let before = viewport_count(&s, "Sheet A");
    s.set_edit_policy(Box::new(Deny));
    for (id, params) in [
        ("-vports", json!({"option": "2", "layout": "Sheet A"})),
        ("vports", json!({"count": 2, "layout": "Sheet A"})),
        ("vports.3", json!({"layout": "Sheet A"})),
    ] {
        let r = s.execute(id, &params);
        assert!(unlicensed(&r), "{id}: {r:?}");
    }
    assert_eq!(viewport_count(&s, "Sheet A"), before);
    let r = s.execute("-vports", &json!({"option": "2"}));
    assert!(!unlicensed(&r), "model-space tiling is a view operation: {r:?}");
}

#[test]
fn is_view_only_parameter_rules() {
    let layer_set = find_command("layer.set").unwrap();
    for params in [Value::Null, json!([]), json!({"name": "A", "bogus": 1}), json!({"name": "A", "color": 1})] {
        assert!(!is_view_only(layer_set, &params), "{params}");
    }
    assert!(is_view_only(layer_set, &json!({"name": "A", "on": true, "frozen": false, "locked": false})));
    let xref = find_command("-xref").unwrap();
    assert!(!is_view_only(xref, &json!({})));
    assert!(!is_view_only(xref, &Value::Null));
}
