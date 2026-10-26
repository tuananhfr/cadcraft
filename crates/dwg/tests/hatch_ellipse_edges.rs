//! HATCH ellipse edges keep their extent through the bridge. DWG stores an ellipse edge's start
//! and end parameters in radians and DXF (groups 50/51) in degrees, as for arc edges.
#![cfg(not(target_arch = "wasm32"))]

use std::f64::consts::PI;
use std::io::Cursor;

/// A 2018 DXF with one solid HATCH: the half ellipse (major axis (5,0), ratio 0.5) from parameter
/// `start` to `end` as group 50/51 text, closed by its diameter.
fn dxf(start: &str, end: &str) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n\
         0\nHATCH\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbHatch\n10\n0\n20\n0\n30\n0\n210\n0\n220\n0\n230\n1\n\
         2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n2\n\
         72\n3\n10\n0\n20\n0\n11\n5\n21\n0\n40\n0.5\n50\n{start}\n51\n{end}\n73\n1\n\
         72\n1\n10\n-5\n20\n0\n11\n5\n21\n0\n97\n0\n75\n0\n76\n1\n98\n0\n\
         0\nENDSEC\n0\nEOF\n"
    )
}

/// The (start, end) of every ellipse edge acadrust holds for the drawing's hatches.
fn ellipse_edges(doc: &acadrust::CadDocument) -> Vec<(f64, f64)> {
    doc.entities()
        .filter_map(|e| if let acadrust::entities::EntityType::Hatch(h) = e { Some(h) } else { None })
        .flat_map(|h| h.paths.iter().flat_map(|p| &p.edges))
        .filter_map(|e| if let acadrust::entities::BoundaryEdge::EllipticArc(a) = e { Some((a.start_angle, a.end_angle)) } else { None })
        .collect()
}

/// Group 50 and 51 values of the DXF text's ellipse edges (those after a `72 3`).
fn dxf_ellipse_angles(text: &str) -> Vec<(f64, f64)> {
    let pairs: Vec<(&str, &str)> =
        text.lines().map(str::trim).collect::<Vec<_>>().chunks(2).filter_map(|p| Some((*p.first()?, *p.get(1)?))).collect();
    let mut out = Vec::new();
    for (i, w) in pairs.iter().enumerate() {
        if *w != ("72", "3") {
            continue;
        }
        let value = |code: &str| pairs.iter().skip(i + 1).take(8).find(|p| p.0 == code).and_then(|p| p.1.parse::<f64>().ok());
        out.extend(value("50").zip(value("51")));
    }
    out
}

fn near(got: &[(f64, f64)], want: (f64, f64)) -> bool {
    got.len() == 1 && got.iter().all(|g| (g.0 - want.0).abs() < 1e-9 && (g.1 - want.1).abs() < 1e-9)
}

/// A DWG whose ellipse edge runs from 0 to π radians, as AutoCAD writes it.
fn dwg_in_radians() -> Vec<u8> {
    let text = dxf("0", &PI.to_string());
    let doc = acadrust::DxfReader::from_reader(Cursor::new(text.into_bytes())).and_then(|r| r.read()).unwrap();
    assert!(near(&ellipse_edges(&doc), (0.0, PI)), "acadrust holds radians: {:?}", ellipse_edges(&doc));
    acadrust::DwgWriter::write_to_vec(&doc).unwrap()
}

#[test]
fn dwg_ellipse_edges_read_as_degrees() {
    let text = String::from_utf8(cadcraft_dwg::dwg_to_dxf(&dwg_in_radians()).unwrap()).unwrap();
    let got = dxf_ellipse_angles(&text);
    assert!(near(&got, (0.0, 180.0)), "groups 50/51: {got:?}");
}

#[test]
fn dxf_ellipse_edges_write_as_radians() {
    let dwg = cadcraft_dwg::dxf_to_dwg(dxf("0", "180").as_bytes()).unwrap();
    let doc = acadrust::DwgReader::from_stream(Cursor::new(dwg.clone())).read().unwrap();
    let got = ellipse_edges(&doc);
    assert!(near(&got, (0.0, PI)), "DWG parameters: {got:?}");
    // And back: the same degrees.
    let text = String::from_utf8(cadcraft_dwg::dwg_to_dxf(&dwg).unwrap()).unwrap();
    assert!(near(&dxf_ellipse_angles(&text), (0.0, 180.0)), "{:?}", dxf_ellipse_angles(&text));
}
