//! HATCH boundaries with ellipse edges read from DWG keep their curve. DWG holds an ellipse
//! edge's parameters in radians and DXF in degrees; read unconverted, the edges of AutoCAD files
//! collapsed onto their major axis ends and the fills ran off as spikes.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use cadcraft_doc::{Drawing, EntityKind, Space, entity_bounds};
use cadcraft_geom::Bounds2;

/// A one-hatch DXF: the half ellipse (major axis (5,0), ratio 0.5) from 0° to 180°, `ccw` or
/// not, closed by its diameter.
fn half_ellipse_dxf(ccw: u8) -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1032\n9\n$HANDSEED\n5\n100\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n\
         0\nHATCH\n5\n2A\n330\n1F\n100\nAcDbEntity\n8\n0\n100\nAcDbHatch\n10\n0\n20\n0\n30\n0\n210\n0\n220\n0\n230\n1\n\
         2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n2\n\
         72\n3\n10\n0\n20\n0\n11\n5\n21\n0\n40\n0.5\n50\n0\n51\n180\n73\n{ccw}\n\
         72\n1\n10\n-5\n20\n0\n11\n5\n21\n0\n97\n0\n75\n0\n76\n1\n98\n0\n\
         0\nENDSEC\n0\nEOF\n"
    )
}

fn assert_extents(d: &Drawing, want: [f64; 4]) {
    let b = d.extents(&Space::Model);
    let got = [b.min.x, b.min.y, b.max.x, b.max.y];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3), "extents {got:?}, want {want:?}");
}

#[test]
fn ellipse_edges_keep_their_half_through_dwg() {
    for (ccw, want) in [(1, [-5.0, 0.0, 5.0, 2.5]), (0, [-5.0, -2.5, 5.0, 0.0])] {
        let text = half_ellipse_dxf(ccw);
        assert_extents(&cadcraft_io::read_dxf(text.as_bytes()).unwrap(), want);
        let dwg = cadcraft_dwg::dxf_to_dwg(text.as_bytes()).unwrap();
        assert_extents(&cadcraft_io::read(&dwg, "x.dwg").unwrap(), want);
    }
}

/// The LPC title block (`XREF_LPC_DIR` holds `Khung ten A3.dwg`; no customer data is committed):
/// its logo is solid hatches bounded by lines and ellipse edges, drawn over ellipse outlines. Read
/// wrongly, three of them ran thousands of units off (more than twice the logo's size); read
/// right, only the letters' straight ends pass the curved outlines, by about a tenth of the logo.
#[test]
fn lpc_logo_hatches_stay_inside_their_outlines() {
    let Some(dir) = std::env::var_os("XREF_LPC_DIR").map(PathBuf::from) else {
        eprintln!("XREF_LPC_DIR is not set; skipped");
        return;
    };
    let bytes = std::fs::read(dir.join("Khung ten A3.dwg")).unwrap();
    let d = cadcraft_io::read(&bytes, "Khung ten A3.dwg").unwrap();
    let union = |it: &mut dyn Iterator<Item = Bounds2>| it.fold(Bounds2::EMPTY, |a, b| a.union(&b));
    let outlines = union(&mut d.model.iter().filter(|e| matches!(e.kind, EntityKind::Ellipse(_))).map(|e| entity_bounds(&d, e, 0)));
    let sheet = union(&mut d.model.iter().filter(|e| matches!(e.kind, EntityKind::Line(_))).map(|e| entity_bounds(&d, e, 0)));
    assert!(!outlines.is_empty() && !sheet.is_empty(), "the title block has ellipse outlines and frame lines");
    let logo: Vec<Bounds2> = d
        .model
        .iter()
        .filter(|e| matches!(&e.kind, EntityKind::Hatch(h) if h.solid))
        .map(|e| entity_bounds(&d, e, 0))
        .filter(|b| outlines.intersects(b))
        .collect();
    assert!(logo.len() >= 4, "the logo's hatches: {}", logo.len());
    let slack = outlines.width().max(outlines.height()) * 0.25;
    let outside = logo.iter().filter(|b| !outlines.expand(slack).contains_box(b)).count();
    assert_eq!(outside, 0, "{outside} of {} logo hatches reach outside their outlines", logo.len());
    assert!(logo.iter().all(|b| sheet.contains_box(b)), "the logo stays on the sheet");
    eprintln!("{} logo hatches inside their outlines", logo.len());
}
