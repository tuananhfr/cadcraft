//! `convert` says what the output file leaves out, as the save commands do, and never replaces its
//! input with a file that leaves something out.

use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cadcraft-cli")).args(args).output().unwrap()
}

/// Two custom objects of a type no DWG writer knows.
const FOREIGN: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
    0\nSECTION\n2\nENTITIES\n\
    0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
    0\nENDSEC\n\
    0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACME_DATA\n350\nD\n\
    0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nW1\n350\n30\n3\nW2\n350\n31\n\
    0\nACME_WIDGET\n5\n30\n330\nD\n100\nAcDbAcmeWidget\n1\nfirst\n\
    0\nACME_WIDGET\n5\n31\n330\nD\n100\nAcDbAcmeWidget\n1\nsecond\n\
    0\nENDSEC\n0\nEOF\n";

/// A LINE after the last ENDSEC and a VERTEX with no POLYLINE before it.
const DAMAGED: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n5\n21\n5\n0\nVERTEX\n8\n0\n10\n1\n20\n1\n0\nENDSEC\n0\nLINE\n8\n0\n10\n9\n20\n9\n11\n8\n21\n8\n0\nEOF\n";

#[test]
fn convert_lists_custom_objects_a_dwg_leaves_out_and_keeps_its_input() {
    let dir = std::env::temp_dir().join(format!("cc-cli-dwg-losses-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let foreign = dir.join("foreign.dxf");
    std::fs::write(&foreign, FOREIGN).unwrap();
    let out = cli(&["convert", foreign.to_str().unwrap(), dir.join("out.dwg").to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}: {err}", out.status);
    assert!(err.contains("2 ACME_WIDGET"), "{err}");

    let damaged = dir.join("damaged.dxf");
    std::fs::write(&damaged, DAMAGED).unwrap();
    let damaged = damaged.to_str().unwrap();
    let out = cli(&["convert", damaged, damaged]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("Not saved over damaged.dxf"), "{err}");
    assert_eq!(std::fs::read_to_string(damaged).unwrap(), DAMAGED);
    let _ = std::fs::remove_dir_all(&dir);
}
