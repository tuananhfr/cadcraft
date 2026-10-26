//! Loading an xref from a file the user picked (the web's way, where saved paths can't be
//! followed): `Session::reload_xref_from_bytes` against desktop Reload from disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::color::Color;
use cadcraft_engine::doc::geom::Vec3;
use cadcraft_engine::doc::{Block, Circle, Drawing, Entity, EntityKind, Handle, Insert, Layer, Line, Space, XrefStatus, entity_bounds};
use serde_json::json;

fn hooks() {
    set_io(IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: None,
    });
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cadcraft-xref-picked-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn dxf(d: &Drawing) -> Vec<u8> {
    cadcraft_io::write(d, "x.dxf").unwrap()
}

fn insert(block: &str) -> EntityKind {
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::ZERO,
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: Vec::new(),
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
    })
}

/// A red line on layer Walls and a reference to its block Tree (a circle).
fn site() -> Drawing {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "Walls".into(), color: Color::Index(1), ..Layer::default() });
    let h = d.new_handle();
    let mut e = Entity::new(h, EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(10.0, 0.0, 0.0) }));
    e.common.layer = "Walls".into();
    d.model.push(e);
    let mut tree = Block::new("Tree");
    tree.entities.push(Entity::new(Handle(0x50), EntityKind::Circle(Circle { center: Vec3::ZERO, radius: 1.0 })));
    d.blocks.insert("Tree".into(), Arc::new(tree));
    let h = d.new_handle();
    d.model.push(Entity::new(h, insert("Tree")));
    d
}

/// A drawing referencing `path` as an attached xref named after its stem, placed at 0,0.
fn host_with(path: &str) -> Drawing {
    let mut d = Drawing::new_metric();
    let name = Path::new(path).file_stem().unwrap().to_string_lossy().into_owned();
    let mut b = Block::new(&name);
    b.xref_path = Some(path.to_string());
    d.blocks.insert(name.clone(), Arc::new(b));
    let h = d.new_handle();
    d.model.push(Entity::new(h, insert(&name)));
    d
}

/// A session with `host` (saved in a fresh folder, its xref file not beside it) open.
fn opened(tag: &str, host: &Drawing) -> (Session, PathBuf) {
    hooks();
    let dir = folder(tag);
    let path = dir.join("host.dxf");
    std::fs::write(&path, dxf(host)).unwrap();
    let mut s = Session::new();
    s.execute("open", &json!({ "path": path.to_string_lossy() })).unwrap();
    (s, dir)
}

fn status(s: &Session, name: &str) -> Option<XrefStatus> {
    s.doc().unwrap().block(name).map(|b| b.xref_status)
}

fn objects(s: &Session, name: &str) -> usize {
    s.doc().unwrap().block(name).map_or(0, |b| b.entities.len())
}

#[test]
fn a_picked_file_loads_the_missing_xref_whatever_its_name() {
    let (mut s, _) = opened("load", &host_with("frames/title.dxf"));
    assert_eq!((status(&s, "title"), objects(&s, "title")), (Some(XrefStatus::NotFound), 0));

    let r = s.reload_xref_from_bytes("TITLE", "Title block v2.dxf", &dxf(&site())).unwrap();
    assert_eq!((r["name"].as_str(), r["status"].as_str(), r["file"].as_str()), (Some("title"), Some("Loaded"), Some("Title block v2.dxf")));
    assert_eq!(status(&s, "title"), Some(XrefStatus::Loaded));
    assert_eq!(objects(&s, "title"), 2, "the picked file's model space");
    let d = s.doc().unwrap();
    assert_eq!(d.layer("title|Walls").map(|l| l.color), Some(Color::Index(1)));
    assert!(d.block("title|Tree").is_some());
    assert!(d.extents(&Space::Model).max.x >= 10.0 - 1e-9, "the frame is drawn");
    // The saved path stays the drawing's; the echo names the file that was attached.
    assert_eq!(d.block("title").unwrap().xref_path.as_deref(), Some("frames/title.dxf"));
    assert!(s.log.iter().any(|m| m.contains("loaded from Title block v2.dxf")), "{:?}", s.log);
}

#[test]
fn a_file_that_cannot_be_loaded_changes_nothing() {
    let (mut s, _) = opened("bad", &host_with("title.dxf"));
    let before = Arc::clone(&s.state().unwrap().doc);
    let undo = s.state().unwrap().undo.len();
    let cases: [(&str, &[u8], &str); 4] = [
        ("title.dxf", b"this is not a drawing", "title.dxf"),
        ("title.dwg", b"AC1032\0\0\0garbage", "title.dwg"),
        ("title.pdf", b"%PDF-1.4", "only DWG and DXF"),
        ("title", b"0\nEOF\n", "only DWG and DXF"),
    ];
    for (file, bytes, says) in cases {
        let e = s.reload_xref_from_bytes("title", file, bytes).unwrap_err().to_string();
        assert!(e.contains("not loaded") && e.contains(says), "{file}: {e}");
        assert!(Arc::ptr_eq(&before, &s.state().unwrap().doc), "{file}: the drawing is unchanged");
        assert_eq!(s.state().unwrap().undo.len(), undo, "{file}: no undo step");
        assert_eq!(status(&s, "title"), Some(XrefStatus::NotFound));
    }
    assert!(s.reload_xref_from_bytes("nothing", "title.dxf", &dxf(&site())).unwrap_err().to_string().contains("no xref"));
    // A block that isn't an xref, and a nested xref's dependent block, aren't xrefs to load.
    s.doc_mut().unwrap().blocks.insert("Plain".into(), Arc::new(Block::new("Plain")));
    assert!(s.reload_xref_from_bytes("Plain", "title.dxf", &dxf(&site())).is_err());
}

/// Adds xref `name` saved as `path` (not loaded) with one reference to it.
fn attach_missing(d: &mut Drawing, name: &str, path: &str) {
    let mut b = Block::new(name);
    b.xref_path = Some(path.to_string());
    d.blocks.insert(name.into(), Arc::new(b));
    let h = d.new_handle();
    d.model.push(Entity::new(h, insert(name)));
}

#[test]
fn one_pick_loads_every_missing_xref_that_names_the_same_file() {
    let mut host = host_with("frames/Title.dxf");
    // The same file under another name and folder, with the other separator and case.
    attach_missing(&mut host, "TitleA2", r"C:\Users\someone\Downloads\TITLE.DXF");
    attach_missing(&mut host, "TitleOld", "old/title.dxf");
    attach_missing(&mut host, "logo", "frames/logo.dxf");
    attach_missing(&mut host, "TitleBak", "frames/Title.dxf.bak");
    let (mut s, _) = opened("same-file", &host);
    let undo = s.state().unwrap().undo.len();
    let log = s.log.len();

    let r = s.reload_xref_from_bytes("title", "Title block v2.dxf", &dxf(&site())).unwrap();
    assert_eq!(r["name"], "Title");
    assert_eq!(r["also"], json!(["TitleA2", "TitleOld"]), "{r}");
    assert_eq!(r["line"], "\"Title\" loaded from Title block v2.dxf.");
    let same = ["Title", "TitleA2", "TitleOld"];
    for name in same {
        assert_eq!((status(&s, name), objects(&s, name)), (Some(XrefStatus::Loaded), 2), "{name}");
        assert!(s.doc().unwrap().layer(&format!("{name}|Walls")).is_some(), "{name}");
    }
    for name in ["logo", "TitleBak"] {
        assert_eq!((status(&s, name), objects(&s, name)), (Some(XrefStatus::NotFound), 0), "{name}: another file");
    }
    let echoed: Vec<&String> = s.log[log..].iter().filter(|m| m.contains(" loaded from ")).collect();
    let expected = same.map(|name| format!("\"{name}\" loaded from Title block v2.dxf."));
    assert_eq!(echoed, expected.iter().collect::<Vec<_>>(), "one line per xref loaded: {:?}", s.log);

    assert_eq!(s.state().unwrap().undo.len(), undo + 1, "one undo step for the whole pick");
    s.undo().unwrap();
    for name in same {
        assert_ne!(status(&s, name), Some(XrefStatus::Loaded), "{name}: undone together");
        assert_eq!(objects(&s, name), 0, "{name}");
    }
    s.redo().unwrap();

    // Only missing xrefs get the file: a loaded or unloaded one is left as it is.
    s.execute("-xref", &json!({ "option": "unload", "names": "TitleOld" })).unwrap();
    s.execute("-xref", &json!({ "option": "reload", "names": "Title" })).unwrap();
    assert_eq!(status(&s, "Title"), Some(XrefStatus::NotFound), "no such file on disk");
    let kept = Arc::clone(s.doc().unwrap().block("TitleA2").unwrap());
    let r = s.reload_xref_from_bytes("Title", "again.dxf", &dxf(&site())).unwrap();
    assert_eq!(r["also"], json!([]), "{r}");
    assert!(Arc::ptr_eq(&kept, s.doc().unwrap().block("TitleA2").unwrap()), "a loaded xref is not reloaded");
    assert_eq!(status(&s, "TitleOld"), Some(XrefStatus::Unloaded));
}

#[test]
fn undo_and_redo_match_a_reload_from_disk() {
    let (mut picked, _) = opened("undo-picked", &host_with("title.dxf"));
    let (mut disk, dir) = opened("undo-disk", &host_with("title.dxf"));
    std::fs::write(dir.join("title.dxf"), dxf(&site())).unwrap();

    picked.reload_xref_from_bytes("title", "title.dxf", &dxf(&site())).unwrap();
    disk.execute("-xref", &json!({ "option": "reload", "names": "title" })).unwrap();
    for s in [&picked, &disk] {
        assert_eq!((status(s, "title"), objects(s, "title")), (Some(XrefStatus::Loaded), 2));
        assert!(s.state().unwrap().is_dirty(), "a reload counts as a change");
    }
    let label = |s: &Session| s.state().unwrap().undo.last().map(|u| u.label.clone());
    assert_eq!(label(&picked), label(&disk), "one step with the same label");
    assert_eq!(picked.state().unwrap().undo.len(), disk.state().unwrap().undo.len());

    for s in [&mut picked, &mut disk] {
        s.undo().unwrap();
        assert_eq!((status(s, "title"), objects(s, "title")), (Some(XrefStatus::NotFound), 0));
        assert!(s.doc().unwrap().layer("title|Walls").is_none() && s.doc().unwrap().block("title|Tree").is_none());
        s.redo().unwrap();
        assert_eq!((status(s, "title"), objects(s, "title")), (Some(XrefStatus::Loaded), 2));
    }
}

#[test]
fn xrefs_inside_a_picked_file_stay_unloaded_and_are_listed() {
    let (mut s, dir) = opened("nested", &host_with("title.dxf"));
    let mut frame = site();
    let mut logo = Block::new("logo");
    logo.xref_path = Some("logo.dxf".into());
    frame.blocks.insert("logo".into(), Arc::new(logo));
    let h = frame.new_handle();
    frame.model.push(Entity::new(h, insert("logo")));
    // Even a file of that name beside the host isn't looked for.
    std::fs::write(dir.join("logo.dxf"), dxf(&site())).unwrap();

    let r = s.reload_xref_from_bytes("title", "title.dxf", &dxf(&frame)).unwrap();
    assert_eq!(status(&s, "title"), Some(XrefStatus::Loaded));
    assert_eq!(objects(&s, "title"), 3);
    let nested = r["nested"].as_array().unwrap();
    assert_eq!(nested.len(), 1, "{r}");
    assert!(nested[0].as_str().unwrap().contains("\"logo\" not loaded"), "{r}");
    assert!(s.log.iter().any(|m| m.contains("\"logo\" not loaded")), "{:?}", s.log);
    assert!(s.doc().unwrap().block("title|logo").is_some_and(|b| b.entities.is_empty()));
}

/// The LPC drawing whose title-block frames are an xref (`XREF_LPC_DIR` holds both files; no
/// customer data is committed). Opened from bytes, as on the web, the frames are missing until
/// the xref's file is picked.
#[test]
fn lpc_title_block_xref_loads_from_picked_bytes() {
    let Some(dir) = std::env::var_os("XREF_LPC_DIR").map(PathBuf::from) else {
        eprintln!("XREF_LPC_DIR is not set; skipped");
        return;
    };
    hooks();
    let main = "260525_LPC_ST_KC PHONG BOM + BE PCCC.dwg";
    let bytes = std::fs::read(dir.join(main)).unwrap();
    let mut s = Session::new();
    let f = cadcraft_engine::cmd::file::read_data(&bytes, main).unwrap();
    s.finish_open(f).unwrap();
    // Two xrefs name that file (one by an absolute path from another computer); one pick loads both.
    let keys: Vec<String> = s
        .doc()
        .unwrap()
        .blocks
        .values()
        .filter(|b| b.xref_path.as_deref().is_some_and(|p| p.to_ascii_lowercase().ends_with("khung ten a3.dwg")))
        .map(|b| b.name.clone())
        .collect();
    assert!(!keys.is_empty(), "the drawing references Khung ten A3");
    for key in &keys {
        assert_ne!(status(&s, key), Some(XrefStatus::Loaded), "opened from bytes, nothing is looked for on disk");
    }
    // The frames sit in blocks placed in model space: those references grow once they load.
    let bounds = |s: &Session| -> Vec<_> {
        let d = s.doc().unwrap();
        d.model.iter().map(|e| (e.handle, entity_bounds(d, e, 0))).collect()
    };
    let before = bounds(&s);

    let frame = std::fs::read(dir.join("Khung ten A3.dwg")).unwrap();
    let r = s.reload_xref_from_bytes(&keys[0], "Khung ten A3.dwg", &frame).unwrap();
    assert_eq!(r["status"], "Loaded");
    assert_eq!(r["also"].as_array().map(Vec::len), Some(keys.len() - 1), "one pick loads them all: {r}");
    for key in &keys {
        assert_eq!(status(&s, key), Some(XrefStatus::Loaded));
        assert!(objects(&s, key) > 0, "the frame's objects are in the xref");
    }
    let after = bounds(&s);
    assert_eq!(before.len(), after.len());
    let grown = before.iter().zip(&after).filter(|((h, b), (k, a))| h == k && a.union(b) == *a && a != b).count();
    assert!(grown > 0, "the references holding the frames are drawn larger");
    eprintln!("xrefs {keys:?}: {grown} model-space references now draw the frames");
}

/// The web's way to keep a picked xref: Bind it and save, so the next visit needs no pick.
#[test]
fn a_bound_picked_xref_is_saved_with_the_drawing() {
    let (mut s, _) = opened("bind", &host_with("title.dxf"));
    s.reload_xref_from_bytes("title", "picked.dxf", &dxf(&site())).unwrap();
    s.execute("-xref", &json!({ "option": "bind", "names": "title" })).unwrap();
    let saved = cadcraft_io::read(&dxf(s.doc().unwrap()), "host.dxf").unwrap();
    let b = saved.block("title").unwrap();
    assert_eq!(b.xref_path, None, "a block of the drawing now");
    assert_eq!(b.entities.len(), 2, "the frame's objects are saved");
}
