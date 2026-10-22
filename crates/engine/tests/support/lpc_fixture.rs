use std::sync::Arc;

use cadcraft_doc::{Attrib, Block, Drawing, Entity, EntityKind};
use cadcraft_engine::Session;
use cadcraft_geom::{PolyVertex, Vec2};

use super::frames::{anon_dynamic, place, polyline, text};

pub fn lpc_like_drawing() -> Drawing {
    let mut session = Session::new();
    let drawing = session.doc_mut().unwrap();
    drawing.header.set_i64("INSUNITS", 4);
    for (name, width, height) in [("KHUNG_A1", 841.0, 594.0), ("KHUNG_A3", 420.0, 297.0)] {
        let mut block = Block::new(name);
        block.entities.push(Entity::new(drawing.new_handle(), polyline(width, height)));
        let mut inner = polyline(width - 20.0, height - 20.0);
        if let EntityKind::LwPolyline(polyline) = &mut inner {
            polyline.vertices = polyline.vertices.iter().map(|vertex| PolyVertex::new(vertex.p + Vec2::new(10.0, 10.0))).collect();
        }
        block.entities.push(Entity::new(drawing.new_handle(), inner));
        for (tag, prompt) in
            [("SO_BV", "Sheet number"), ("TEN_BV", "Drawing title"), ("TY_LE", "Drawing scale"), ("REV", "Revision"), ("NGAY", "Date")]
        {
            block.entities.push(Entity::new(
                drawing.new_handle(),
                EntityKind::AttDef(Attrib {
                    tag: tag.into(),
                    text: text(""),
                    invisible: false,
                    constant: false,
                    prompt: prompt.into(),
                    props: Default::default(),
                    multiline: None,
                    modes: Default::default(),
                }),
            ));
        }
        drawing.blocks.insert(name.into(), Arc::new(block));
    }
    anon_dynamic(drawing, "KHUNG_A1", "*U7");
    // Anonymous definitions need distinct entity handles when the fixture is serialized as DXF.
    let anonymous = drawing.blocks.remove("*U7").unwrap();
    let mut block = Block { entities: Default::default(), ..(*anonymous).clone() };
    for source in anonymous.entities.iter() {
        let mut entity = (**source).clone();
        entity.handle = drawing.new_handle();
        block.entities.push(entity);
    }
    drawing.blocks.insert("*U7".into(), Arc::new(block));
    for (index, (name, scale, rotation)) in [("KHUNG_A1", 1.0, 0.0), ("*U7", 1.0, 90.0), ("KHUNG_A3", 2.0, 0.0)].into_iter().enumerate() {
        let sheet_id = format!("KT-{:02}", index + 1);
        place(
            &mut session,
            name,
            [index as f64 * 2000.0, 0.0],
            scale,
            rotation,
            false,
            &[("SO_BV", &sheet_id), ("TEN_BV", "Synthetic drawing"), ("TY_LE", "1:1"), ("REV", "R1"), ("NGAY", "2026-01-01")],
        );
    }
    session.doc().unwrap().clone()
}
