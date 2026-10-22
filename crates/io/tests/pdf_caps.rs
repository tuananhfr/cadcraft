#[allow(dead_code)]
#[path = "../../engine/tests/support/pdf_probe.rs"]
mod pdf_probe;

use cadcraft_doc::{Common, Drawing, EntityKind, Layer, Line, Space, Text};
use cadcraft_geom::Vec3;
use cadcraft_io::PdfOptions;
use cadcraft_io::pdf::{MAX_PAGES, pdf, pdf_items, pdf_sheets};

#[test]
fn thousand_pages_fit_and_1001_fail() {
    let d = Drawing::new_metric();
    let options = PdfOptions { compress: true, ..PdfOptions::default() };
    assert_eq!(MAX_PAGES, 1000);
    let mut spaces = vec![Space::Model; 1000];
    let bytes = pdf_sheets(&d, &spaces, &options).unwrap();
    assert_eq!(pdf_probe::pages(&bytes).len(), 1000);
    assert!(bytes.len() <= cadcraft_doc::MAX_PDF_BYTES);
    spaces.push(Space::Model);
    let error = pdf_sheets(&d, &spaces, &options).unwrap_err().to_string();
    assert!(error.contains("1000") && error.contains("split"), "{error}");
}

#[test]
fn sheet_15m_uses_user_unit() {
    let mut d = Drawing::new_metric();
    let layer = d.layers.len();
    d.layers.push(Layer::new("Measured frame"));
    for (a, b) in [([0.0, 0.0], [914.0, 0.0]), ([914.0, 0.0], [914.0, 15000.0]), ([914.0, 15000.0], [0.0, 15000.0]), ([0.0, 15000.0], [0.0, 0.0])] {
        let line = EntityKind::Line(Line { a: Vec3::new(a[0], a[1], 0.0), b: Vec3::new(b[0], b[1], 0.0) });
        d.add(&Space::Model, Common { layer: "Measured frame".into(), ..Common::default() }, line).unwrap();
    }
    let options = PdfOptions {
        paper_mm: Some((914.0, 15000.0)),
        landscape: Some(false),
        fit: Some(false),
        scale: Some(1.0),
        margins: Some([0.0; 4]),
        lineweights: Some(false),
        compress: false,
        ..PdfOptions::default()
    };
    let bytes = pdf(&d, &Space::Model, &options).unwrap();
    let pages = pdf_probe::pages(&bytes);
    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    // 42519.7 pt / 14400 rounds up to 3: a unit of 2 would still leave the MediaBox above the 14400 limit.
    assert_eq!(page.user_unit, 3.0);
    assert!((page.media_box[3] * page.user_unit - 42519.7).abs() < 0.1);
    let bounds = pdf_probe::path_bbox(&bytes, layer);
    for (coordinate, expected) in bounds.into_iter().zip([0.0, 0.0, 914.0 * 72.0 / 25.4, 15000.0 * 72.0 / 25.4]) {
        assert!((coordinate * page.user_unit - expected).abs() < 0.1);
    }
    let text = String::from_utf8_lossy(&bytes);
    let catalog = text.lines().find(|line| line.starts_with("<< /Type /Catalog ")).unwrap();
    assert!(catalog.contains("/Version /1.6"));
    let options = PdfOptions { layers: false, ..options };
    let bytes = pdf(&d, &Space::Model, &options).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/Version /1.6"));
}

#[test]
fn sheet_over_15m_is_an_error() {
    let mut d = Drawing::new_metric();
    for size in [(914.0, 15001.0), (15001.0, 914.0)] {
        let options = PdfOptions { paper_mm: Some(size), ..PdfOptions::default() };
        let error = pdf(&d, &Space::Model, &options).unwrap_err().to_string();
        assert!(error.contains("15000"), "{error}");
        let items = vec![(Space::Model, options); 2];
        assert!(pdf_items(&d, &items, &PdfOptions::default()).is_err());
    }
    d.layouts.first_mut().unwrap().page.height_mm = 15001.0;
    assert!(pdf(&d, &Space::Paper("Layout1".into()), &PdfOptions::default()).is_err());
}

#[test]
#[ignore]
fn bench_thousand_dense_pages() {
    let mut d = Drawing::new_metric();
    for index in 0..10_000 {
        let x = f64::from(index % 100) * 2.0;
        let y = f64::from(index / 100) * 2.0;
        let line = EntityKind::Line(Line { a: Vec3::new(x, y, 0.0), b: Vec3::new(x + 1.5, y + 1.0, 0.0) });
        d.add(&Space::Model, Common::default(), line).unwrap();
    }
    for index in 0..500 {
        let text = Text {
            insert: Vec3::new(f64::from(index % 20) * 10.0, f64::from(index / 20) * 8.0, 0.0),
            align_pt: None,
            height: 1.0,
            value: format!("CAD {index:03}"),
            rotation: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            style: "Standard".into(),
            halign: Default::default(),
            valign: Default::default(),
        };
        d.add(&Space::Model, Common::default(), EntityKind::Text(text)).unwrap();
    }
    let options = PdfOptions { compress: true, paper: Some("A3".into()), ..PdfOptions::default() };
    let started = std::time::Instant::now();
    let bytes = pdf_sheets(&d, &vec![Space::Model; 1000], &options).unwrap();
    eprintln!("bytes/page={}, total bytes={}, time={:?}", bytes.len() / 1000, bytes.len(), started.elapsed());
    assert_eq!(pdf_probe::pages(&bytes).len(), 1000);
    assert!(bytes.len() <= cadcraft_doc::MAX_PDF_BYTES);
}

#[test]
fn seams_draw_dashed_overlap_edges() {
    let d = Drawing::new_metric();
    let mut page = serde_json::json!({
        "window": [[0, 0], [297, 210]], "width": 297, "height": 210,
        "landscape": true, "scale": 1, "fit": false, "margins": [0, 0, 0, 0], "label": "Seam test"
    });
    let render = |page: &serde_json::Value| {
        cadcraft_io::plotter::plot(&d, &Space::Model, &serde_json::json!({"output": "pdf", "compress": false, "pages": [page]})).unwrap()
    };
    let baseline = render(&page);
    page["seams"] = serde_json::json!([false, false, false, false]);
    assert_eq!(render(&page), baseline);
    page["seams"] = serde_json::json!([false, false, true, false]);
    let bytes = render(&page);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("[8.504 5.669] 0 d"), "{text}");
    assert!(text.contains("0.709 w"));
    let paths: Vec<Vec<f64>> = text
        .lines()
        .filter_map(|line| {
            let values: Vec<_> = line.split_whitespace().collect();
            if values.len() == 7 && values[2] == "m" && values[5] == "l" && values[6] == "S" {
                Some([values[0], values[1], values[3], values[4]].into_iter().map(|value| value.parse().unwrap()).collect())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(paths.len(), 3, "one seam and two ticks");
    let right = (297.0 - 20.0) * 72.0 / 25.4;
    assert!((paths[0][0] - right).abs() < 0.001);
    assert!((paths[0][2] - right).abs() < 0.001);
    assert!(paths[0][1].abs() < 0.001);
    assert!((paths[0][3] - 210.0 * 72.0 / 25.4).abs() < 0.001);
    for tick in &paths[1..] {
        assert!((tick[2] - tick[0] - 10.0 * 72.0 / 25.4).abs() < 0.001);
        assert_eq!(tick[1], tick[3]);
    }
    page["seamOverlapMm"] = serde_json::json!(40);
    let custom = render(&page);
    assert_ne!(custom, bytes);
    assert_eq!(pdf_probe::pages(&custom)[0].media_box, pdf_probe::pages(&bytes)[0].media_box);
}
