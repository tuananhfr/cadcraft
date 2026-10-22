use std::sync::Arc;

use cadcraft_engine::doc::{EntityKind, Handle, Insert};
use cadcraft_engine::{EngineError, Input, Session};
use serde_json::{Value, json};

fn definition(session: &mut Session, name: &str, tags: &[(&str, &str)]) {
    let mut handles = vec![session.execute("line", &json!({"points": [[0, 0], [1, 0]]})).unwrap()["handles"][0].clone()];
    for (tag, default) in tags {
        handles.push(session.execute("attdef", &json!({"tag": tag, "default": default, "at": [1, 2], "height": 1})).unwrap()["handle"].clone());
    }
    session.execute("block", &json!({"name": name, "base": [1, 1], "handles": handles, "keep": "delete"})).unwrap();
}

fn insert(session: &mut Session, name: &str, at: [f64; 2], scale: Value, attributes: Value) -> Handle {
    let result = session.execute("insert", &json!({"name": name, "at": at, "scale": scale, "rotation": 30, "attribs": attributes})).unwrap();
    Handle::parse_hex(result["handle"].as_str().unwrap()).unwrap()
}

fn reference(session: &Session, handle: Handle) -> Insert {
    match &session.doc().unwrap().entity(handle).unwrap().kind {
        EntityKind::Insert(insert) => insert.clone(),
        _ => panic!("expected block reference"),
    }
}

fn bad_params(result: cadcraft_engine::Result<Value>, message: &str) {
    assert!(matches!(result, Err(EngineError::BadParams { cmd, msg }) if cmd == "blockreplace" && msg.contains(message)));
}

#[test]
fn blockreplace_swaps_definition_and_keeps_transform() {
    let mut session = Session::new();
    definition(&mut session, "A", &[]);
    definition(&mut session, "B", &[]);
    let first = insert(&mut session, "A", [10.0, 20.0], json!(2), json!({}));
    let second = insert(&mut session, "A", [-5.0, 8.0], json!(2), json!({}));
    let before = session.doc().unwrap().clone();
    let undo_count = session.state().unwrap().undo.len();
    session.set_selection(vec![first, second]);
    let result = session.execute("blockreplace", &json!({"name": "b"})).unwrap();
    assert_eq!(result, json!({"block": "B", "replaced": 2}));
    assert_eq!(session.state().unwrap().undo.len(), undo_count + 1);
    for handle in [first, second] {
        let mut expected = match &before.entity(handle).unwrap().kind {
            EntityKind::Insert(insert) => insert.clone(),
            _ => panic!("expected block reference"),
        };
        expected.block = "B".into();
        assert_eq!(reference(&session, handle), expected);
    }
    session.execute("u", &json!({})).unwrap();
    assert_eq!(session.doc().unwrap(), &before);
}

#[test]
fn blockreplace_matches_attributes_by_tag() {
    let mut session = Session::new();
    definition(&mut session, "A", &[("ROOM", "old"), ("REMOVED", "obsolete")]);
    definition(&mut session, "B", &[("ROOM", "new"), ("LEVEL", "ground")]);
    let handle = insert(&mut session, "A", [10.0, 20.0], json!([2, 3]), json!({"ROOM": "101"}));
    session
        .doc_mut()
        .unwrap()
        .modify_entity(handle, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                insert.attribs[0].tag = "room".into();
            }
        })
        .unwrap();
    let before = session.doc().unwrap().clone();
    let mut expected = reference(&session, handle);
    expected.block = "B".into();
    let baseline = insert(&mut session, "B", [10.0, 20.0], json!([2, 3]), json!({"ROOM": "101"}));
    let expected_attributes = reference(&session, baseline).attribs;
    session.execute("u", &json!({})).unwrap();
    let undo_count = session.state().unwrap().undo.len();
    session.execute("blockreplace", &json!({"name": "B", "handles": [handle.hex()]})).unwrap();
    let replaced = reference(&session, handle);
    let values: Vec<_> = replaced.attribs.iter().map(|attribute| (attribute.tag.as_str(), attribute.text.value.as_str())).collect();
    assert_eq!(values, [("ROOM", "101"), ("LEVEL", "ground")]);
    assert_eq!(replaced.attribs, expected_attributes);
    expected.attribs = expected_attributes;
    assert_eq!(replaced, expected);
    assert_eq!(session.state().unwrap().undo.len(), undo_count + 1);
    session.execute("u", &json!({})).unwrap();
    assert_eq!(session.doc().unwrap(), &before);
}

#[test]
fn blockreplace_rejects_missing_name_and_xrefs() {
    let mut session = Session::new();
    definition(&mut session, "A", &[]);
    definition(&mut session, "B", &[]);
    let normal = insert(&mut session, "A", [0.0, 0.0], json!(1), json!({}));
    let external = insert(&mut session, "A", [5.0, 0.0], json!(1), json!({}));
    let before = session.doc().unwrap().clone();
    let undo_count = session.state().unwrap().undo.len();
    for name in [json!(null), json!(""), json!("Z")] {
        let message = if name == "Z" { "no block" } else { "required" };
        bad_params(session.execute("blockreplace", &json!({"name": name, "handles": [normal.hex()]})), message);
    }
    bad_params(session.execute("blockreplace", &json!({"handles": [normal.hex()]})), "required");
    assert_eq!(session.doc().unwrap(), &before);
    assert_eq!(session.state().unwrap().undo.len(), undo_count);
    let mut xref = session.doc().unwrap().block("A").unwrap().as_ref().clone();
    xref.name = "External".into();
    xref.xref_path = Some("synthetic.dxf".into());
    session.doc_mut().unwrap().blocks.insert("External".into(), Arc::new(xref));
    session
        .doc_mut()
        .unwrap()
        .modify_entity(external, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                insert.block = "External".into();
            }
        })
        .unwrap();
    let before = session.doc().unwrap().clone();
    bad_params(
        session.execute("blockreplace", &json!({"name": "B", "handles": [normal.hex(), external.hex()]})),
        "external references cannot be replaced",
    );
    bad_params(session.execute("blockreplace", &json!({"name": "External", "handles": [normal.hex()]})), "external references cannot be replaced");
    assert_eq!(session.doc().unwrap(), &before);
    assert_eq!(session.state().unwrap().undo.len(), undo_count);
}

#[test]
fn blockreplace_interactive_selects_then_prompts_and_undoes_once() {
    let mut session = Session::new();
    definition(&mut session, "A", &[("ROOM", "101")]);
    definition(&mut session, "B", &[("ROOM", "102"), ("LEVEL", "1")]);
    let handle = insert(&mut session, "A", [3.0, 4.0], json!(2), json!({}));
    let before = session.doc().unwrap().clone();
    session.settings.pickfirst = true;
    for pickfirst in [false, true] {
        session.set_selection(if pickfirst { vec![handle] } else { Vec::new() });
        let undo_count = session.state().unwrap().undo.len();
        session.start("blockreplace").unwrap();
        if !pickfirst {
            assert_eq!(session.current_prompt().unwrap().message, "Select block references to replace");
            session.input(Input::Pick(vec![handle])).unwrap();
            session.input(Input::Enter).unwrap();
        }
        assert_eq!(session.current_prompt().unwrap().message, "Enter replacement block name");
        session.input(Input::Text("B".into())).unwrap();
        assert!(session.running.is_none());
        assert_eq!(reference(&session, handle).block, "B");
        assert_eq!(session.state().unwrap().undo.len(), undo_count + 1);
        session.execute("u", &json!({})).unwrap();
        assert_eq!(session.doc().unwrap(), &before);
    }
}

#[test]
fn blockreplace_rejects_mixed_locked_and_invalid_targets_atomically() {
    let mut session = Session::new();
    definition(&mut session, "A", &[]);
    definition(&mut session, "B", &[]);
    let handle = insert(&mut session, "A", [0.0, 0.0], json!(1), json!({}));
    let line = session.execute("line", &json!({"points": [[0, 0], [2, 0]]})).unwrap()["handles"][0].clone();
    let before = session.doc().unwrap().clone();
    bad_params(session.execute("blockreplace", &json!({"name": "B", "handles": [handle.hex(), line]})), "not a block reference");
    bad_params(session.execute("blockreplace", &json!({"name": "B", "handles": [handle.hex(), "FFFFFFFF"]})), "not a block reference");
    bad_params(session.execute("blockreplace", &json!({"name": "B", "handles": []})), "no block references");
    assert_eq!(session.doc().unwrap(), &before);
    session.execute("layer.set", &json!({"name": "0", "locked": true})).unwrap();
    let before = session.doc().unwrap().clone();
    bad_params(session.execute("blockreplace", &json!({"name": "B", "handles": [handle.hex()]})), "locked layer");
    assert_eq!(session.doc().unwrap(), &before);
}
