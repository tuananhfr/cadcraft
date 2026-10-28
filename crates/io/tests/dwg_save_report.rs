//! Saving as DWG says which of another program's custom objects the file leaves out (the DWG
//! writer has no DWG data for them); DXF writes them back as read and reports nothing.
#![cfg(not(target_arch = "wasm32"))]

use cadcraft_io::{SaveOptions, read, write_reported, write_with};

/// A 2018 DXF whose OBJECTS section holds two custom objects of a type no DWG writer knows.
fn foreign() -> cadcraft_doc::Drawing {
    let text = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
                0\nSECTION\n2\nENTITIES\n\
                0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
                0\nENDSEC\n\
                0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n3\nACME_DATA\n350\nD\n\
                0\nDICTIONARY\n5\nD\n330\nC\n100\nAcDbDictionary\n3\nW1\n350\n30\n3\nW2\n350\n31\n\
                0\nACME_WIDGET\n5\n30\n330\nD\n100\nAcDbAcmeWidget\n1\nfirst\n\
                0\nACME_WIDGET\n5\n31\n330\nD\n100\nAcDbAcmeWidget\n1\nsecond\n\
                0\nENDSEC\n0\nEOF\n";
    read(text.as_bytes(), "foreign.dxf").unwrap()
}

#[test]
fn dwg_save_lists_custom_objects_it_leaves_out() {
    let d = foreign();
    let (bytes, dropped) = write_reported(&d, "foreign.dwg", &SaveOptions::default()).unwrap();
    assert!(cadcraft_dwg::is_dwg(&bytes));
    assert_eq!(dropped.objects.get("ACME_WIDGET"), Some(&2), "{dropped:?}");
}

#[test]
fn dxf_save_keeps_custom_objects_and_reports_nothing() {
    let d = foreign();
    let (bytes, dropped) = write_reported(&d, "foreign.dxf", &SaveOptions::default()).unwrap();
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(bytes, write_with(&d, "foreign.dxf", &SaveOptions::default()).unwrap());
    assert!(String::from_utf8_lossy(&bytes).contains("ACME_WIDGET"));
}

#[test]
fn reported_dwg_save_writes_the_same_bytes_as_a_plain_save() {
    let d = foreign();
    let (reported, _) = write_reported(&d, "foreign.dwg", &SaveOptions::default()).unwrap();
    assert_eq!(reported, write_with(&d, "foreign.dwg", &SaveOptions::default()).unwrap());
}
