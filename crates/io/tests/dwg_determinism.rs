//! A DWG save is a function of the drawing alone: the same drawing gives the same bytes on every
//! save, whenever it runs, and a saved DWG reopened and saved again gives those bytes back. Revisions
//! are compared by content hash, so a clock or random id in the file would make every save look new.
#![cfg(not(target_arch = "wasm32"))]

use cadcraft_color::{Color, Rgb};
use cadcraft_doc::*;
use cadcraft_geom::Vec3;
use cadcraft_io::{SaveOptions, Version, read, write, write_with};

fn text(value: &str) -> EntityKind {
    EntityKind::Text(Text {
        insert: Vec3::new(1.0, 1.0, 0.0),
        align_pt: None,
        height: 2.5,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Left,
        valign: VAlign::Baseline,
    })
}

fn drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "Tường".into(), color: Color::True(Rgb(12, 200, 99)), ..Layer::default() });
    let wall = Common { layer: "Tường".into(), ..Common::default() };
    d.add(&Space::Model, wall.clone(), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(6000.0, 0.0, 0.0) })).unwrap();
    d.add(&Space::Model, wall, EntityKind::Circle(Circle { center: Vec3::new(3000.0, 2000.0, 0.0), radius: 450.0 })).unwrap();
    d.add(&Space::Model, Common::default(), EntityKind::Arc(Arc { center: Vec3::ZERO, radius: 900.0, start: 0.0, end: 1.5 })).unwrap();
    d.add(&Space::Model, Common::default(), text("Mặt bằng tầng 1 — Ø20")).unwrap();
    let sheet = Space::Paper(d.layouts[0].name.clone());
    d.add(&sheet, Common::default(), text("Khung tên")).unwrap();
    d
}

#[test]
fn the_same_drawing_saves_to_the_same_bytes_whenever_it_is_saved() {
    let d = drawing();
    let first = write(&d, "a.dwg").unwrap();
    assert_eq!(write(&d, "a.dwg").unwrap(), first, "two saves in a row");
    // DWG time stamps have millisecond resolution; past a second, a stamped clock would show.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(write(&d, "a.dwg").unwrap(), first, "a save a second later");
    assert_eq!(write(&d, "other-name.dwg").unwrap(), first, "the file name is not stored");

    let mut changed = d.clone();
    changed.add(&Space::Model, Common::default(), text("thêm")).unwrap();
    assert_ne!(write(&changed, "a.dwg").unwrap(), first, "a different drawing gives different bytes");
}

#[test]
fn every_dwg_version_saves_deterministically() {
    let d = drawing();
    for v in Version::ALL {
        let o = SaveOptions { version: Some(v), binary: false };
        assert_eq!(write_with(&d, "a.dwg", &o).unwrap(), write_with(&d, "a.dwg", &o).unwrap(), "{v:?}");
    }
}

#[test]
fn a_reopened_dwg_saves_back_to_the_same_bytes() {
    let saved = write(&drawing(), "a.dwg").unwrap();
    let reopened = read(&saved, "a.dwg").unwrap();
    assert_eq!(write(&reopened, "a.dwg").unwrap(), saved);
}
