use std::sync::Arc;

use cadcraft_doc::{Attrib, Block, Common, Drawing, Entity, EntityKind, HAlign, Handle, Insert, Line, LwPolyline, RawTag, Space, Text, VAlign};
use cadcraft_engine::Session;
use cadcraft_geom::{PolyVertex, Vec2, Vec3};

pub fn polyline(w: f64, h: f64) -> EntityKind {
    EntityKind::LwPolyline(LwPolyline {
        vertices: [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(PolyVertex::new).into(),
        closed: true,
        const_width: 0.0,
        elevation: 0.0,
        plinegen: false,
    })
}

pub fn text(value: &str) -> Text {
    Text {
        insert: Vec3::new(10.0, 10.0, 0.0),
        align_pt: None,
        height: 2.0,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Left,
        valign: VAlign::Baseline,
    }
}

pub fn frame_block(d: &mut Drawing, name: &str, w: f64, h: f64) {
    let mut block = Block::new(name);
    let h1 = d.new_handle();
    let h2 = d.new_handle();
    block.entities.push(Entity::new(h1, polyline(w, h)));
    block.entities.push(Entity::new(h2, EntityKind::Text(text("FRAME"))));
    d.blocks.insert(name.into(), Arc::new(block));
}

pub fn title_only_block(d: &mut Drawing, name: &str) {
    let mut block = Block::new(name);
    let corners = [Vec2::ZERO, Vec2::new(180.0, 0.0), Vec2::new(180.0, 60.0), Vec2::new(0.0, 60.0)];
    for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)).take(4) {
        block.entities.push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })));
    }
    d.blocks.insert(name.into(), Arc::new(block));
}

pub fn place(s: &mut Session, name: &str, at: [f64; 2], scale: f64, rot_deg: f64, mirror_x: bool, attribs: &[(&str, &str)]) -> Handle {
    let attributes = attribs
        .iter()
        .map(|(tag, value)| Attrib {
            tag: (*tag).into(),
            text: text(value),
            invisible: false,
            constant: false,
            prompt: String::new(),
            props: Default::default(),
            multiline: None,
            modes: Default::default(),
        })
        .collect();
    s.doc_mut()
        .unwrap()
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: name.into(),
                insert: Vec3::new(at[0], at[1], 0.0),
                scale: Vec3::new(if mirror_x { -scale } else { scale }, scale, 1.0),
                rotation: rot_deg.to_radians(),
                attribs: attributes,
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap()
}

pub fn nest(d: &mut Drawing, outer: &str, inner: &str, at: [f64; 2], rot: f64) {
    let h = d.new_handle();
    let block = d.blocks.entry(outer.into()).or_insert_with(|| Arc::new(Block::new(outer)));
    Arc::make_mut(block).entities.push(Entity::new(
        h,
        EntityKind::Insert(Insert {
            block: inner.into(),
            insert: Vec3::new(at[0], at[1], 0.0),
            scale: Vec3::new(1.0, 1.0, 1.0),
            rotation: rot.to_radians(),
            attribs: Vec::new(),
            cols: 1,
            rows: 1,
            col_spacing: 0.0,
            row_spacing: 0.0,
        }),
    ));
}

pub fn anon_dynamic(d: &mut Drawing, effective: &str, anon: &str) {
    let mut block = (**d.block(effective).unwrap()).clone();
    block.name = anon.into();
    block.anonymous = true;
    let mut hidden = Entity::new(d.new_handle(), polyline(2000.0, 1500.0));
    hidden.common.visible = false;
    block.entities.push(hidden);
    d.blocks.insert(anon.into(), Arc::new(block));
    let target = d.new_handle();
    let foreign = Arc::make_mut(&mut d.foreign);
    foreign.targets.insert(target, ("BLOCK_RECORD".into(), effective.into()));
    foreign.records.insert(
        ("BLOCK_RECORD".into(), anon.to_ascii_uppercase()),
        cadcraft_doc::foreign::ForeignExtra {
            xdata: vec![RawTag { code: 1001, value: "AcDbBlockRepBTag".into() }, RawTag { code: 1005, value: target.hex() }],
            ..Default::default()
        },
    );
}
