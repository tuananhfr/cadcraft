//! Saving never loses another program's data silently (UbotCAD Cloud V4.2 §07: "explicit warning,
//! block destructive save"). Saving over the DWG a drawing was opened from is refused when the new
//! file would leave out what the old one holds; any other save goes ahead and lists what it leaves out.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, WriterLosses, set_io, set_write_reported};
use cadcraft_engine::doc::{Common, Drawing, EntityKind, RawTag, Space, Unknown};
use serde_json::json;

/// Stand-in writer. As a DWG, a drawing whose `$PROJECTNAME` is ACME holds two custom objects it
/// can't keep (MANY: one each of nine types), and it can't keep model-space Unknown entities.
fn install_writer() {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write: |_, _| Ok(b"PLAIN".to_vec()), plot: None });
    set_write_reported(|d, name, _| {
        let mut losses = WriterLosses::default();
        if !(name.ends_with(".dwg") || name.ends_with(".dwt")) {
            return Ok((b"NEW".to_vec(), losses));
        }
        match d.header.get("PROJECTNAME").and_then(cadcraft_engine::doc::HVal::as_str) {
            Some("ACME") => losses.objects = BTreeMap::from([("ACME_WIDGET".to_string(), 2)]),
            Some("MANY") => losses.objects = ('A'..='I').map(|c| (format!("TYPE_{c}"), 1)).collect(),
            _ => {}
        }
        for e in d.model.iter() {
            if let EntityKind::Unknown(u) = &e.kind {
                *losses.entities.entry(u.dxf_type.clone()).or_insert(0) += 1;
            }
        }
        Ok((b"NEW".to_vec(), losses))
    });
}

fn with_custom_objects() -> Drawing {
    let mut d = Drawing::new_metric();
    d.header.set_str("PROJECTNAME", "ACME");
    d
}

fn with_unknown_entity() -> Drawing {
    let mut d = Drawing::new_metric();
    let kind = EntityKind::Unknown(Unknown { dxf_type: "MLINE".into(), tags: vec![RawTag { code: 8, value: "0".into() }] });
    d.add(&Space::Model, Common::default(), kind).unwrap();
    d
}

/// A fresh directory with `name` holding the bytes ORIGINAL.
fn source(test: &str, name: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cadcraft-lossy-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, b"ORIGINAL").unwrap();
    let text = path.to_string_lossy().to_string();
    (dir, text)
}

#[test]
fn qsave_over_the_opened_dwg_is_refused_when_the_file_would_lose_data() {
    install_writer();
    let (dir, src) = source("qsave", "plan.dwg");
    let mut s = Session::empty();
    s.open_drawing(with_custom_objects(), "plan.dwg", Some(src.clone()));

    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("2 ACME_WIDGET"), "{e}");
    assert!(e.contains("SAVEAS") && e.contains("DXF"), "the message says how to save: {e}");
    assert_eq!(std::fs::read(&src).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unknown_entities_also_block_overwriting_the_opened_dwg() {
    install_writer();
    let (dir, src) = source("entities", "plan.dwg");
    let mut s = Session::empty();
    s.open_drawing(with_unknown_entity(), "plan.dwg", Some(src.clone()));

    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("1 MLINE"), "{e}");
    assert_eq!(std::fs::read(&src).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saveas_another_name_saves_with_the_list_and_later_qsave_goes_there() {
    install_writer();
    let (dir, src) = source("saveas", "plan.dwg");
    let copy = dir.join("plan-copy.dwg").to_string_lossy().to_string();
    let mut s = Session::empty();
    s.open_drawing(with_custom_objects(), "plan.dwg", Some(src.clone()));

    let r = s.execute("saveas", &json!({ "path": copy })).unwrap();
    assert_eq!(r["objectsNotSaved"], json!({ "ACME_WIDGET": 2 }));
    assert!(s.log.iter().any(|l| l.contains("2 ACME_WIDGET")), "{:?}", s.log);
    assert_eq!(std::fs::read(&copy).unwrap(), b"NEW");

    // The new file never had those objects: saving over it loses nothing it holds.
    s.execute("qsave", &json!({})).unwrap();
    assert_eq!(s.state().unwrap().path.as_deref(), Some(copy.as_str()));
    assert_eq!(std::fs::read(&src).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saveas_back_over_the_opened_dwg_is_refused_too() {
    install_writer();
    let (dir, src) = source("back", "plan.dwg");
    let mut s = Session::empty();
    s.open_drawing(with_custom_objects(), "plan.dwg", Some(src.clone()));

    assert!(s.execute("saveas", &json!({ "path": src })).is_err());
    assert_eq!(std::fs::read(&src).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_drawing_that_loses_nothing_saves_over_its_dwg() {
    install_writer();
    let (dir, src) = source("plain", "plan.dwg");
    let mut s = Session::empty();
    s.open_drawing(Drawing::new_metric(), "plan.dwg", Some(src.clone()));

    let r = s.execute("qsave", &json!({})).unwrap();
    assert!(r.get("objectsNotSaved").is_none() && r.get("notSaved").is_none(), "{r}");
    assert_eq!(std::fs::read(&src).unwrap(), b"NEW");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_new_drawing_saves_as_dwg_with_the_warning() {
    install_writer();
    let (dir, _) = source("new", "unused.dwg");
    let target = dir.join("fresh.dwg").to_string_lossy().to_string();
    let mut s = Session::empty();
    s.open_drawing(with_custom_objects(), "Drawing1", None);

    let r = s.execute("saveas", &json!({ "path": target })).unwrap();
    assert_eq!(r["objectsNotSaved"], json!({ "ACME_WIDGET": 2 }));
    s.execute("qsave", &json!({})).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn records_skipped_when_reading_block_overwriting_the_source() {
    install_writer();
    let (dir, src) = source("notes", "plan.dwg");
    let note = "DWG: 3 of 10 object records couldn't be read and were left out; saving over the original file would lose them";
    let mut d = Drawing::new_metric();
    d.read_notes = vec![note.to_string()];
    let mut s = Session::empty();
    s.open_drawing(d, "plan.dwg", Some(src.clone()));

    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("SAVEAS"), "{e}");
    assert_eq!(std::fs::read(&src).unwrap(), b"ORIGINAL");

    let copy = dir.join("plan-copy.dwg").to_string_lossy().to_string();
    let r = s.execute("saveas", &json!({ "path": copy })).unwrap();
    assert_eq!(r["readNotes"], json!([note]));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_a_dxf_over_itself_is_not_refused() {
    install_writer();
    let (dir, src) = source("dxf", "plan.dxf");
    let mut s = Session::empty();
    s.open_drawing(with_unknown_entity(), "plan.dxf", Some(src.clone()));

    s.execute("qsave", &json!({})).unwrap();
    assert_ne!(std::fs::read(&src).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn document_bytes_lists_objects_the_dwg_leaves_out() {
    install_writer();
    let mut s = Session::empty();
    s.open_drawing(with_custom_objects(), "Drawing1", None);
    let r = s.execute("document.bytes", &json!({ "format": "dwg" })).unwrap();
    assert_eq!(r["objectsNotSaved"], json!({ "ACME_WIDGET": 2 }));
}

#[test]
fn a_long_loss_list_names_the_first_types_and_counts_the_rest() {
    install_writer();
    let (dir, src) = source("many", "plan.dwg");
    let mut s = Session::empty();
    let mut d = Drawing::new_metric();
    d.header.set_str("PROJECTNAME", "MANY");
    s.open_drawing(d, "plan.dwg", Some(src));

    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("1 TYPE_A, 1 TYPE_B, 1 TYPE_C, 1 TYPE_D, 1 TYPE_E, 1 TYPE_F, 3 more types"), "{e}");
    assert!(!e.contains("TYPE_G"), "{e}");
    let lost: BTreeMap<String, usize> = ('A'..='I').map(|c| (format!("TYPE_{c}"), 2)).collect();
    let msg = cadcraft_engine::cmd::file::not_saved_message(&lost).unwrap();
    assert!(msg.ends_with("2 TYPE_F, 3 more types"), "{msg}");
    let _ = std::fs::remove_dir_all(dir);
}
