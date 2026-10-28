//! The DWG loss report and the refusal to overwrite, with the real DXF/DWG writer wired as the apps
//! wire it: another program's custom objects are listed, the opened DWG stays as it was, and the
//! DWG saved under a new name opens and saves over itself without a refusal.
#![cfg(not(target_arch = "wasm32"))]

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, WriterLosses, set_io, set_write_as, set_write_reported};
use serde_json::json;

fn install_real_io() {
    set_io(IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: None,
    });
    set_write_as(|d, name, o| cadcraft_io::write_with(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary }).map_err(|e| e.to_string()));
    set_write_reported(|d, name, o| {
        cadcraft_io::write_reported(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary })
            .map(|(bytes, dropped)| (bytes, WriterLosses { entities: dropped.entities, objects: dropped.objects }))
            .map_err(|e| e.to_string())
    });
}

/// A 2018 drawing whose OBJECTS section holds two custom objects of a type no DWG writer knows.
const FOREIGN_DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
    0\nSECTION\n2\nENTITIES\n\
    0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
    0\nENDSEC\n\
    0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACME_DATA\n350\nD\n\
    0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nW1\n350\n30\n3\nW2\n350\n31\n\
    0\nACME_WIDGET\n5\n30\n330\nD\n100\nAcDbAcmeWidget\n1\nfirst\n\
    0\nACME_WIDGET\n5\n31\n330\nD\n100\nAcDbAcmeWidget\n1\nsecond\n\
    0\nENDSEC\n0\nEOF\n";

#[test]
fn custom_objects_block_overwriting_the_dwg_and_the_copy_saves_over_itself() {
    install_real_io();
    let dir = std::env::temp_dir().join(format!("cadcraft-real-writer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Stands in for the DWG the drawing came from: its bytes must survive the refused save.
    let source = dir.join("plan.dwg");
    std::fs::write(&source, b"ORIGINAL").unwrap();
    let source = source.to_string_lossy().to_string();

    let mut s = Session::empty();
    let drawing = cadcraft_io::read(FOREIGN_DXF.as_bytes(), "plan.dxf").unwrap();
    s.open_drawing(drawing, "plan.dwg", Some(source.clone()));

    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("2 ACME_WIDGET") && e.contains("SAVEAS"), "{e}");
    assert_eq!(std::fs::read(&source).unwrap(), b"ORIGINAL");

    let copy = dir.join("plan-copy.dwg").to_string_lossy().to_string();
    let r = s.execute("saveas", &json!({ "path": copy })).unwrap();
    assert_eq!(r["objectsNotSaved"], json!({ "ACME_WIDGET": 2 }));
    assert!(cadcraft_dwg_magic(&std::fs::read(&copy).unwrap()));

    // Opened from the copy, the drawing has nothing the copy would lose.
    s.execute("open", &json!({ "path": copy })).unwrap();
    let r = s.execute("qsave", &json!({})).unwrap();
    assert!(r.get("objectsNotSaved").is_none() && r.get("notSaved").is_none(), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

fn cadcraft_dwg_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"AC10")
}

/// A drawing with one entity of a type neither CADCraft nor the DWG writer models.
const SHAPE_DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
    0\nSECTION\n2\nENTITIES\n\
    0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
    0\nACME_SHAPE\n5\n2B\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbAcmeShape\n1\nkept in DXF only\n\
    0\nENDSEC\n0\nEOF\n";

/// A LINE after the last ENDSEC and a VERTEX with no POLYLINE before it: two records the reader can't place.
const DAMAGED_DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n5\n21\n5\n0\nVERTEX\n8\n0\n10\n1\n20\n1\n0\nENDSEC\n0\nLINE\n8\n0\n10\n9\n20\n9\n11\n8\n21\n8\n0\nEOF\n";

fn scratch(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cadcraft-real-writer-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_entity_the_dwg_writer_drops_is_counted_once() {
    install_real_io();
    let dir = scratch("once");
    let mut s = Session::empty();
    s.open_drawing(cadcraft_io::read(SHAPE_DXF.as_bytes(), "shape.dxf").unwrap(), "shape.dxf", None);
    let r = s.execute("saveas", &json!({ "path": dir.join("shape.dwg").to_string_lossy() })).unwrap();
    assert_eq!(r["notSaved"], json!({ "ACME_SHAPE": 1 }), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_xref_holding_unmodelled_entities_does_not_block_saving_the_host() {
    install_real_io();
    let dir = scratch("xref");
    let source = dir.join("host.dwg");
    std::fs::write(&source, b"ORIGINAL").unwrap();
    let source = source.to_string_lossy().to_string();
    // The xref's objects are loaded from its own file on open and never written with the host.
    let arch = cadcraft_io::read(SHAPE_DXF.as_bytes(), "arch.dxf").unwrap();
    let mut block = cadcraft_engine::doc::Block::new("ARCH");
    for e in arch.model.iter() {
        block.entities.push(e.as_ref().clone());
    }
    block.xref_path = Some("arch.dxf".into());
    let mut host = cadcraft_engine::doc::Drawing::new_metric();
    host.blocks.insert("ARCH".into(), std::sync::Arc::new(block));
    let mut s = Session::empty();
    s.open_drawing(host, "host.dwg", Some(source.clone()));
    let r = s.execute("qsave", &json!({})).unwrap();
    assert!(r.get("notSaved").is_none(), "{r}");
    assert_ne!(std::fs::read(&source).unwrap(), b"ORIGINAL");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn records_that_could_not_be_read_block_overwriting_the_file_they_came_from() {
    install_real_io();
    let dir = scratch("skipped");
    let source = dir.join("damaged.dxf");
    std::fs::write(&source, DAMAGED_DXF).unwrap();
    let source = source.to_string_lossy().to_string();
    let mut s = Session::empty();
    s.execute("open", &json!({ "path": source })).unwrap();
    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Not saved over damaged.dxf") && e.contains("SAVEAS"), "{e}");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), DAMAGED_DXF);

    let copy = dir.join("copy.dxf").to_string_lossy().to_string();
    let r = s.execute("saveas", &json!({ "path": copy })).unwrap();
    assert_eq!(r["skippedRecords"], json!({ "(no section)": 1, "ENTITIES": 1 }), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_recovered_file_is_not_overwritten_with_its_unreadable_records_left_out() {
    install_real_io();
    let dir = scratch("recover");
    let source = dir.join("damaged.dxf");
    std::fs::write(&source, DAMAGED_DXF).unwrap();
    let source = source.to_string_lossy().to_string();
    let mut s = Session::empty();
    s.execute("recover", &json!({ "path": source })).unwrap();
    let e = s.execute("qsave", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Not saved over damaged.dxf"), "{e}");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), DAMAGED_DXF);
    let _ = std::fs::remove_dir_all(dir);
}
