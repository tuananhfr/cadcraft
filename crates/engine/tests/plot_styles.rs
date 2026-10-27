//! PAGESETUP: the plot style table and "Display plot styles" of a layout (issue #410).

use cadcraft_engine::Session;
use serde_json::json;

#[test]
fn pagesetup_sets_table_and_display_flag() {
    let mut s = Session::new();
    let r = s.execute("pagesetup", &json!({"layout": "Layout1", "plotStyleTable": "monochrome.ctb", "displayPlotStyles": true})).unwrap();
    assert_eq!(r["page"]["plotStyleTable"], "monochrome.ctb");
    assert_eq!(r["page"]["showPlotStyles"], true);
    let page = &s.doc().unwrap().layout("Layout1").unwrap().page;
    assert!(page.show_plot_styles && page.plot_style_table == "monochrome.ctb");
    // The report lists the built-in tables.
    let report = s.execute("pagesetup", &json!({})).unwrap();
    let tables: Vec<&str> = report["plotStyleTables"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert!(tables.contains(&"monochrome.ctb") && tables.contains(&"grayscale.ctb"), "{tables:?}");
    s.execute("pagesetup", &json!({"layout": "Layout1", "displayPlotStyles": false})).unwrap();
    assert!(!s.doc().unwrap().layout("Layout1").unwrap().page.show_plot_styles);
}

#[test]
fn plotstyle_sets_objects_and_the_current_style() {
    let mut s = Session::new();
    s.execute("line", &json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let handle = s.doc().unwrap().model.last().unwrap().handle;
    let h = handle.hex();
    // No name: a report.
    let r = s.execute("plotstyle", &json!({})).unwrap();
    assert_eq!(r["current"], "ByLayer");
    assert!(r["styles"].as_array().unwrap().iter().any(|v| v == "Normal"));
    // An object's style.
    let r = s.execute("plotstyle", &json!({"name": "Black", "handles": [h]})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!(s.doc().unwrap().entity(handle).unwrap().common.plot_style, "Black");
    // Nothing selected: the current style for new objects.
    s.execute("select", &json!({"handles": []})).ok();
    s.execute("plotstyle", &json!({"name": "byblock"})).unwrap();
    s.execute("line", &json!({"points": [[0, 1], [10, 1]]})).unwrap();
    let h2 = s.doc().unwrap().model.last().unwrap().handle;
    assert_eq!(s.doc().unwrap().entity(h2).unwrap().common.plot_style, "ByBlock");
    assert!(s.execute("plotstyle", &json!({"name": "  "})).is_err());
    assert!(s.execute("plotstyle", &json!({"name": 5})).is_err());
    // A layer's plot style.
    s.execute("layer.set", &json!({"name": "0", "plotStyle": "Thick"})).unwrap();
    assert_eq!(s.doc().unwrap().layer("0").unwrap().plot_style, "Thick");
    assert!(s.execute("layer.set", &json!({"name": "0", "plotStyle": "ByLayer"})).is_err());
}

#[test]
fn stylesmanager_creates_edits_saves_and_loads_tables() {
    let mut s = Session::new();
    let list = s.execute("stylesmanager", &json!({})).unwrap();
    let names: Vec<&str> = list["tables"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"monochrome.ctb") && names.contains(&"default.stb"), "{names:?}");
    // A new named table gets its extension and Normal.
    let r = s.execute("stylesmanager", &json!({"action": "new", "name": "office", "kind": "named"})).unwrap();
    assert_eq!(r["name"], "office.stb");
    let r = s
        .execute(
            "stylesmanager",
            &json!({"action": "set", "name": "office.stb", "style": {"name": "Pen 5", "color": "#00ff00", "screening": 40, "lineweight": 0.35, "linetype": "HIDDEN", "end": "round"}}),
        )
        .unwrap();
    let pen = r["styles"].as_array().unwrap().iter().find(|v| v["name"] == "Pen 5").unwrap().clone();
    assert_eq!(
        (pen["color"].as_str(), pen["screening"].as_u64(), pen["lineweight"].as_u64(), pen["end"].as_str()),
        (Some("#00ff00"), Some(40), Some(35), Some("round"))
    );
    // Bad edits are refused.
    for bad in [
        json!({"name": "Pen 5", "screening": 101}),
        json!({"name": "Pen 5", "color": "green"}),
        json!({"name": "Pen 5", "lineweight": 9}),
        json!({"name": "Pen 5", "join": "zigzag"}),
    ] {
        assert!(s.execute("stylesmanager", &json!({"action": "set", "name": "office.stb", "style": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("stylesmanager", &json!({"action": "set", "name": "office.stb", "removeStyle": "Normal"})).is_err());
    // Editing a built-in keeps a copy in the drawing; colour-dependent styles are Color_n.
    s.execute("stylesmanager", &json!({"action": "set", "name": "monochrome.ctb", "style": {"name": "Color_1", "color": null}})).unwrap();
    assert_eq!(s.doc().unwrap().plot_style_tables.len(), 2);
    assert!(s.execute("stylesmanager", &json!({"action": "set", "name": "monochrome.ctb", "style": {"name": "Pen"}})).is_err());
    // Save as text, delete, load back.
    let text = s.execute("stylesmanager", &json!({"action": "save", "name": "office.stb"})).unwrap()["text"].as_str().unwrap().to_string();
    s.execute("stylesmanager", &json!({"action": "delete", "name": "office.stb"})).unwrap();
    assert!(s.execute("stylesmanager", &json!({"action": "get", "name": "office.stb"})).is_err());
    s.execute("stylesmanager", &json!({"action": "load", "text": text})).unwrap();
    let got = s.execute("stylesmanager", &json!({"action": "get", "name": "office.stb"})).unwrap();
    assert_eq!(got["inDrawing"], true);
    assert!(got["styles"].as_array().unwrap().iter().any(|v| v["name"] == "Pen 5"));
    assert!(s.execute("stylesmanager", &json!({"action": "load", "text": "{\"x\": 1}"})).is_err());
    assert!(s.execute("stylesmanager", &json!({"action": "delete", "name": "grayscale.ctb"})).is_err(), "built-ins stay");
    assert!(s.execute("stylesmanager", &json!({"action": "explode"})).is_err());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::temp_dir().join(format!("cadcraft-pstyle-{}.stb.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        s.execute("stylesmanager", &json!({"action": "save", "name": "office.stb", "path": p})).unwrap();
        s.execute("stylesmanager", &json!({"action": "load", "path": p, "name": "copy.stb"})).unwrap();
        assert!(s.execute("stylesmanager", &json!({"action": "get", "name": "copy.stb"})).is_ok());
        let _ = std::fs::remove_file(&path);
    }
    // Undo takes a table edit back.
    s.execute("undo", &json!({})).unwrap();
}

/// An AutoCAD-style `.ctb` file (as the format is publicly described) whose styles all print
/// black; style 1 also asks for a plot style linetype, which CADCraft drops.
fn black_ctb() -> Vec<u8> {
    let black = 0xC200_0000u32 as i32;
    let styles: String = (0..255)
        .map(|i| {
            let linetype = if i == 0 { 2 } else { 31 };
            format!(" {i}{{\n  name=\"Color_{}\n  color={black}\n  mode_color={black}\n  color_policy=1\n  screen=100\n  linetype={linetype}\n  lineweight=255\n  fill_style=73\n  end_style=4\n  join_style=5\n }}\n", i + 1)
        })
        .collect();
    let text =
        format!("description=\"Black pens\naci_table_available=TRUE\nplot_style{{\n{styles}}}\ncustom_lineweight_table{{\n 0=0.00\n 1=0.25\n}}\n\0");
    let body = miniz_oxide::deflate::compress_to_vec_zlib(text.as_bytes(), 6);
    let mut out = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec".to_vec();
    let mut adler = (1u32, 0u32);
    for b in &body {
        adler.0 = (adler.0 + u32::from(*b)) % 65521;
        adler.1 = (adler.1 + adler.0) % 65521;
    }
    for n in [(adler.1 << 16) | adler.0, text.len() as u32, body.len() as u32] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&body);
    out
}

#[test]
fn load_ctb_then_plot_black() {
    use cadcraft_engine::cmd::file::{IoHooks, base64_decode, base64_encode, set_io, set_plot_style_reader};
    set_io(IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    set_plot_style_reader(|name, bytes| cadcraft_io::plot_style_file::read_plot_style_file(name, bytes).map(|r| (r.table, r.unsupported_linetypes)));
    let mut s = Session::new();
    s.execute("line", &json!({"points": [[0, 0], [100, 0]]})).unwrap();
    let h = s.doc().unwrap().model.last().unwrap().handle;
    s.doc_mut().unwrap().modify_entity(h, |e| e.common.color = cadcraft_color::Color::Index(1)).unwrap();

    let ctb = black_ctb();
    let r = s.execute("stylesmanager", &json!({"action": "load", "name": "Office pens.ctb", "data": base64_encode(&ctb)})).unwrap();
    assert_eq!((r["name"].as_str(), r["kind"].as_str(), r["inDrawing"].as_bool()), (Some("Office pens.ctb"), Some("colorDependent"), Some(true)));
    assert_eq!(r["unsupportedLinetypes"], 1);
    assert_eq!(r["description"], "Black pens");

    let pdf = |s: &mut Session, table: &str| {
        let r = s.execute("plot", &json!({"layout": "Model", "plotStyleTable": table, "compress": false})).unwrap();
        String::from_utf8_lossy(&base64_decode(r["data"].as_str().unwrap()).unwrap()).into_owned()
    };
    let plain = pdf(&mut s, "None");
    assert!(plain.contains("1 0 0 RG"), "no table: red");
    let black = pdf(&mut s, "Office pens.ctb");
    assert!(black.contains("0 0 0 RG") && !black.contains("1 0 0 RG"), "the imported table prints black");

    // Damaged and oversized files are refused; our own JSON still loads through `data`.
    let mut damaged = ctb.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0xFF;
    for data in [base64_encode(&damaged), base64_encode(b"PIAFILEVERSION_2.0,CTBVER1"), "!!".into(), "A".repeat(8 << 20)] {
        assert!(s.execute("stylesmanager", &json!({"action": "load", "name": "x.ctb", "data": data})).is_err());
    }
    assert!(s.execute("stylesmanager", &json!({"action": "load", "data": base64_encode(&ctb)})).is_err(), "a name is needed");
    assert!(s.execute("stylesmanager", &json!({"action": "load"})).is_err());
    let own = s.execute("stylesmanager", &json!({"action": "save", "name": "monochrome.ctb"})).unwrap()["text"].as_str().unwrap().to_string();
    let r = s.execute("stylesmanager", &json!({"action": "load", "name": "mono-copy.ctb", "data": base64_encode(own.as_bytes())})).unwrap();
    assert_eq!((r["name"].as_str(), r["unsupportedLinetypes"].as_u64()), (Some("mono-copy.ctb"), Some(0)));

    // From a path the table is named after the file.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::temp_dir().join(format!("cadcraft-import-{}.CTB", std::process::id()));
        std::fs::write(&path, &ctb).unwrap();
        let r = s.execute("stylesmanager", &json!({"action": "load", "path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["name"].as_str(), Some(format!("cadcraft-import-{}.ctb", std::process::id()).as_str()));
        let _ = std::fs::remove_file(&path);
    }
}
