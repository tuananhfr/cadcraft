use cadcraft_doc::{Common, Drawing, EntityKind, Line, Space};
use cadcraft_geom::Vec3;
use cadcraft_io::plotter;
use serde_json::{Value, json};

fn drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    let line = EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(100.0, 0.0, 0.0) });
    d.add(&Space::Model, Common::default(), line).unwrap();
    d
}

fn pdf_line_mm(options: Value) -> f64 {
    let mut options = options;
    options["compress"] = json!(false);
    options["lineweights"] = json!(false);
    let bytes = plotter::plot(&drawing(), &Space::Model, &options).unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(!pdf.contains("/FlateDecode"));
    let xs: Vec<f64> = pdf
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            match fields.as_slice() {
                [x, _, "m" | "l"] => Some(x.parse().unwrap()),
                _ => None,
            }
        })
        .collect();
    assert_eq!(xs.len(), 2, "{pdf}");
    (xs[1] - xs[0]).abs() * 25.4 / 72.0
}

fn assert_mm(got: f64, want: f64) {
    assert!((got - want).abs() * 72.0 / 25.4 <= 0.05, "{got} mm != {want} mm (tolerance 0.05 pt)");
}

#[test]
fn pages_with_scale_are_not_fitted() {
    for area in [json!({"window": [[0, 0], [100, 50]]}), json!({"outline": [[0, 0], [100, 0], [100, 50], [0, 50]]})] {
        let mut page = area;
        page["scale"] = json!(1.0);
        page["paper"] = json!("A3");
        assert_mm(pdf_line_mm(json!({"pages": [page]})), 100.0);
    }
}

#[test]
fn model_plot_with_scale_is_not_fitted_without_fit_flag() {
    for extra in [json!({}), json!({"window": [[0, 0], [100, 50]]})] {
        let mut options = extra;
        options["scale"] = json!(0.5);
        assert_mm(pdf_line_mm(options), 50.0);
    }
}

#[test]
fn png_plot_with_scale_is_not_fitted() {
    for (extra, expected_mm) in [(json!({"scale": 0.5}), 50.0), (json!({"scale": 1.0}), 100.0), (json!({"fit": false}), 100.0)] {
        let mut options = json!({"output": "png", "paper": "A3", "landscape": true, "resolution": 100, "lineweights": false});
        options.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let bytes = plotter::plot(&drawing(), &Space::Model, &options).unwrap();
        let image = tiny_skia::Pixmap::decode_png(&bytes).unwrap();
        let width = image.width() as usize;
        let inked: Vec<_> = image.pixels().iter().enumerate().filter(|(_, pixel)| pixel.red() < 128).map(|(index, _)| index % width).collect();
        let left = *inked.iter().min().unwrap();
        let right = *inked.iter().max().unwrap();
        let got = (right - left) as f64 * 25.4 / 100.0;
        assert!((got - expected_mm).abs() < 0.6, "{options}: {got} mm != {expected_mm} mm");
        assert!(((left + right) as f64 / 2.0 - f64::from(image.width()) / 2.0).abs() < 2.0, "explicit scales stay centered");
    }
}

#[test]
fn fit_true_still_fits() {
    for scale in [json!(null), json!(0.5)] {
        assert_mm(pdf_line_mm(json!({"paper": "A3", "landscape": true, "margins": [5, 5, 5, 5], "fit": true, "scale": scale})), 410.0);
        assert_mm(
            pdf_line_mm(json!({"pages": [{"window": [[0, 0], [100, 50]], "paper": "A3", "landscape": true,
                "margins": [5, 5, 5, 5], "fit": true, "scale": scale}]})),
            410.0,
        );
    }
}

#[test]
fn pages_use_individual_margins_and_explicit_fit_false() {
    assert_mm(
        pdf_line_mm(json!({"margins": [5, 5, 5, 5], "pages": [{"window": [[0, 0], [100, 50]],
            "paper": "A3", "landscape": true, "margins": [20, 20, 20, 20]}]})),
        380.0,
    );
    assert_mm(pdf_line_mm(json!({"pages": [{"window": [[0, 0], [100, 50]], "paper": "A3", "fit": false}]})), 100.0);
}
