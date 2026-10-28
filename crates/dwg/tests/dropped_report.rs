//! A DXF→DWG conversion says what it left out (other programs' custom entities and objects with no
//! DWG data to write), by DXF type, instead of dropping it silently.
#![cfg(not(target_arch = "wasm32"))]

/// A 2018 DXF with one custom entity and two custom objects of a type no DWG writer knows.
fn dxf_with_custom_records() -> String {
    "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
     0\nSECTION\n2\nENTITIES\n\
     0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
     0\nACME_SHAPE\n5\n2B\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbAcmeShape\n1\nkept in DXF only\n\
     0\nENDSEC\n\
     0\nSECTION\n2\nOBJECTS\n0\nDICTIONARY\n5\nC\n330\n0\n100\nAcDbDictionary\n\
     0\nACME_WIDGET\n5\n30\n330\nC\n100\nAcDbAcmeWidget\n1\nfirst\n\
     0\nACME_WIDGET\n5\n31\n330\nC\n100\nAcDbAcmeWidget\n1\nsecond\n\
     0\nENDSEC\n0\nEOF\n"
        .to_string()
}

#[test]
fn conversion_lists_custom_entities_and_objects_it_left_out() {
    let (dwg, dropped) = cadcraft_dwg::dxf_to_dwg_reported(dxf_with_custom_records().as_bytes(), Some("AC1032")).unwrap();
    assert!(cadcraft_dwg::is_dwg(&dwg));
    assert_eq!(dropped.entities.get("ACME_SHAPE"), Some(&1), "{dropped:?}");
    assert_eq!(dropped.objects.get("ACME_WIDGET"), Some(&2), "{dropped:?}");
    assert!(!dropped.is_empty());
}

#[test]
fn conversion_of_a_plain_drawing_leaves_nothing_out() {
    let plain = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n0\nENDSEC\n\
                 0\nSECTION\n2\nENTITIES\n\
                 0\nLINE\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0\n20\n0\n30\n0\n11\n1\n21\n1\n31\n0\n\
                 0\nENDSEC\n0\nEOF\n";
    let (_, dropped) = cadcraft_dwg::dxf_to_dwg_reported(plain.as_bytes(), None).unwrap();
    assert!(dropped.is_empty(), "{dropped:?}");
}

#[test]
fn reported_conversion_writes_the_same_bytes_as_the_plain_one() {
    let text = dxf_with_custom_records();
    let (reported, _) = cadcraft_dwg::dxf_to_dwg_reported(text.as_bytes(), Some("AC1032")).unwrap();
    assert_eq!(reported, cadcraft_dwg::dxf_to_dwg_as(text.as_bytes(), "AC1032").unwrap());
}
