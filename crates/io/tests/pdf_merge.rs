use cadcraft_doc::{Common, Drawing, EntityKind, Layer, Line, Space, Text};
use cadcraft_geom::Vec3;
use cadcraft_io::PdfOptions;
use cadcraft_io::pdf::{MAX_PAGES, pdf_items};
use cadcraft_io::pdf_merge::{MAX_MERGE_OBJECTS, merge_own};

struct Pdf {
    bodies: Vec<Vec<u8>>,
    root: usize,
    info: usize,
}

fn reference(dict: &[u8], key: &str) -> usize {
    let text = std::str::from_utf8(dict).unwrap();
    text.split(&format!("/{key} ")).nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap()
}

fn probe(bytes: &[u8]) -> Pdf {
    let marker = b"startxref\n";
    let at = bytes.windows(marker.len()).rposition(|w| w == marker).unwrap() + marker.len();
    let xref: usize = std::str::from_utf8(&bytes[at..]).unwrap().lines().next().unwrap().parse().unwrap();
    let mut lines = bytes[xref..].split(|b| *b == b'\n');
    assert_eq!(lines.next().unwrap(), b"xref");
    let count: usize = std::str::from_utf8(lines.next().unwrap()).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(lines.next().unwrap(), b"0000000000 65535 f ");
    let offsets: Vec<usize> = (1..count)
        .map(|_| {
            let line = lines.next().unwrap();
            assert_eq!(line.len(), 19);
            assert_eq!(&line[10..], b" 00000 n ");
            std::str::from_utf8(&line[..10]).unwrap().parse().unwrap()
        })
        .collect();
    let bodies = offsets
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let end = offsets.get(i + 1).copied().unwrap_or(xref);
            let header = format!("{} 0 obj\n", i + 1);
            bytes[*start..end].strip_prefix(header.as_bytes()).unwrap().strip_suffix(b"\nendobj\n").unwrap().to_vec()
        })
        .collect();
    assert_eq!(lines.next().unwrap(), b"trailer");
    let trailer = lines.next().unwrap();
    assert_eq!(reference(trailer, "Size"), count);
    Pdf { bodies, root: reference(trailer, "Root"), info: reference(trailer, "Info") }
}

impl Pdf {
    fn body(&self, id: usize) -> &[u8] {
        &self.bodies[id - 1]
    }

    fn pages(&self) -> Vec<&[u8]> {
        let tree = self.body(reference(self.body(self.root), "Pages"));
        let text = std::str::from_utf8(tree).unwrap();
        text.split("/Kids [")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .chunks(3)
            .map(|fields| self.body(fields[0].parse().unwrap()))
            .collect()
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in self.bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root {} 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.bodies.len() + 1, self.root, self.info)
                .as_bytes(),
        );
        out
    }
}

fn stream(body: &[u8]) -> &[u8] {
    let marker = b"\nstream\n";
    let start = body.windows(marker.len()).position(|w| w == marker).unwrap() + marker.len();
    let length = reference(&body[..start - marker.len()], "Length");
    let data = &body[start..start + length];
    assert_eq!(&body[start + length..], b"\nendstream");
    data
}

fn contents(pdf: &Pdf) -> Vec<Vec<u8>> {
    pdf.pages()
        .iter()
        .map(|page| {
            let body = pdf.body(reference(page, "Contents"));
            let data = stream(body);
            if body.windows(b"/FlateDecode".len()).any(|w| w == b"/FlateDecode") {
                miniz_oxide::inflate::decompress_to_vec_zlib(data).unwrap()
            } else {
                data.to_vec()
            }
        })
        .collect()
}

fn drawing(off: bool) -> Drawing {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "WALL".into(), on: !off, ..Layer::default() });
    d.add(
        &Space::Model,
        Common { layer: "WALL".into(), ..Common::default() },
        EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(80.0, 50.0, 0.0) }),
    )
    .unwrap();
    d
}

fn part(d: &Drawing, sizes: &[(f64, f64)], compress: bool) -> Vec<u8> {
    let items: Vec<_> = sizes
        .iter()
        .enumerate()
        .map(|(i, size)| {
            (
                Space::Model,
                PdfOptions {
                    paper_mm: Some(*size),
                    landscape: Some(size.0 > size.1),
                    label: Some(format!("Original {}", i + 1)),
                    ..PdfOptions::default()
                },
            )
        })
        .collect();
    pdf_items(d, &items, &PdfOptions { compress, include_off_layers: true, ..PdfOptions::default() }).unwrap()
}

#[test]
fn merged_pages_keep_media_boxes() {
    let d = drawing(false);
    let parts: Vec<_> = [(420.0, 297.0), (594.0, 841.0), (841.0, 2400.0)].iter().map(|size| part(&d, &[*size], true)).collect();
    let refs: Vec<_> = parts.iter().map(Vec::as_slice).collect();
    let merged = probe(&merge_own(&refs, &[], "Mixed paper").unwrap());
    let expected = [[0.0, 0.0, 1190.55, 841.89], [0.0, 0.0, 1683.78, 2383.94], [0.0, 0.0, 2383.94, 6803.15]];
    assert_eq!(merged.pages().len(), expected.len());
    for (page, want) in merged.pages().iter().zip(expected) {
        let text = std::str::from_utf8(page).unwrap();
        let coords: Vec<f64> =
            text.split("/MediaBox [").nth(1).unwrap().split(']').next().unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
        for (got, value) in coords.iter().zip(want) {
            assert!((got - value).abs() < 0.01, "expected {value}, got {got}");
        }
    }
}

#[test]
fn merge_keeps_order_and_labels() {
    let d = drawing(false);
    let parts: Vec<_> = [(420.0, 297.0), (594.0, 841.0), (841.0, 2400.0)].iter().map(|size| part(&d, &[*size], true)).collect();
    let refs: Vec<_> = parts.iter().map(Vec::as_slice).collect();
    let labels = ["KT-01", "KT-02", "KT-03"].map(String::from);
    let merged = probe(&merge_own(&refs, &labels, "Batch (one)").unwrap());
    let catalog = std::str::from_utf8(merged.body(merged.root)).unwrap();
    assert!(catalog.contains("/PageLabels << /Nums [0 << /P (KT-01) >> 1 << /P (KT-02) >> 2 << /P (KT-03) >>] >>"));
    assert!(std::str::from_utf8(merged.body(merged.info)).unwrap().contains("/Title (Batch \\(one\\))"));
    let expected: Vec<_> = parts.iter().flat_map(|bytes| contents(&probe(bytes))).collect();
    assert_eq!(contents(&merged), expected);
    for (page, source) in merged.pages().iter().zip(&parts) {
        let source = probe(source);
        assert_eq!(stream(merged.body(reference(page, "Contents"))), stream(source.body(reference(source.pages()[0], "Contents"))));
    }
}

#[test]
fn merge_dedups_layers_by_name() {
    let on = part(&drawing(false), &[(420.0, 297.0)], false);
    let off = part(&drawing(true), &[(420.0, 297.0)], false);
    let merged = probe(&merge_own(&[&on, &off], &[], "Layers").unwrap());
    let walls: Vec<_> = merged.bodies.iter().enumerate().filter(|(_, body)| body.as_slice() == b"<< /Type /OCG /Name (WALL) >>").collect();
    assert_eq!(walls.len(), 1);
    let id = walls[0].0 + 1;
    let cat = std::str::from_utf8(merged.body(merged.root)).unwrap();
    assert!(cat.contains(&format!("/OFF [{id} 0 R]")));
    for page in merged.pages() {
        assert!(std::str::from_utf8(page).unwrap().contains(&format!(" {id} 0 R")));
    }
}

#[test]
fn stream_bytes_are_not_renumbered() {
    let mut d = drawing(false);
    d.add(
        &Space::Model,
        Common::default(),
        EntityKind::Text(Text {
            insert: Vec3::ZERO,
            align_pt: None,
            height: 5.0,
            value: "1 0 R".into(),
            rotation: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            style: "Standard".into(),
            halign: Default::default(),
            valign: Default::default(),
        }),
    )
    .unwrap();
    let mut source = probe(&part(&d, &[(420.0, 297.0)], false));
    let id = reference(source.pages()[0], "Contents");
    let mut data = stream(source.body(id)).to_vec();
    // Glyphs are outlines or hex codes, so a PDF comment supplies literal reference-like bytes without a font fixture.
    data.extend_from_slice(b"% text (1 0 R), endobj, stream, /Length -1\n");
    let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(&data);
    body.extend_from_slice(b"\nendstream");
    source.bodies[id - 1] = body;
    let first = part(&drawing(false), &[(210.0, 297.0)], false);
    let source = source.bytes();
    let merged = probe(&merge_own(&[&first, &source], &[], "Streams").unwrap());
    assert_eq!(contents(&merged)[1], data);
}

#[test]
fn foreign_pdf_is_refused() {
    let error = merge_own(&[b"%PDF-1.7\nforeign PDF\n%%EOF\n"], &[], "Foreign").unwrap_err();
    assert_eq!(error.to_string(), "not a UbotCAD PDF");
    let mut foreign = probe(&part(&drawing(false), &[(210.0, 297.0)], false));
    foreign.bodies[foreign.info - 1] = b"<< /Producer (Other) >>".to_vec();
    assert_eq!(merge_own(&[&foreign.bytes()], &[], "Foreign").unwrap_err().to_string(), "not a UbotCAD PDF");
}

#[test]
fn hostile_bytes_never_panic() {
    let bytes = part(&drawing(false), &[(420.0, 297.0)], false);
    for cut in (0..bytes.len()).step_by(97) {
        assert!(merge_own(&[&bytes[..cut]], &[], "Truncated").is_err(), "cut {cut}");
    }
    let marker = b"startxref\n";
    let at = bytes.windows(marker.len()).rposition(|w| w == marker).unwrap() + marker.len();
    let end = at + bytes[at..].iter().position(|b| *b == b'\n').unwrap();
    for offset in [(bytes.len() + 99).to_string(), "999999999999999999999999999".into()] {
        let mut bad_xref = bytes.clone();
        bad_xref.splice(at..end, offset.bytes());
        assert!(merge_own(&[&bad_xref], &[], "Bad xref").is_err());
    }
    let mut negative = probe(&bytes);
    let id = reference(negative.pages()[0], "Contents");
    let body = String::from_utf8(negative.bodies[id - 1].clone()).unwrap();
    let length = reference(body.as_bytes().split(|b| *b == b'\n').next().unwrap(), "Length");
    negative.bodies[id - 1] = body.replacen(&format!("/Length {length}"), "/Length -1", 1).into_bytes();
    assert!(merge_own(&[&negative.bytes()], &[], "Negative length").is_err());
    let mut deep = probe(&bytes);
    deep.bodies[id - 1] = format!("{}0{}", "[".repeat(100), "]".repeat(100)).into_bytes();
    assert!(merge_own(&[&deep.bytes()], &[], "Deep nesting").is_err());
}

#[test]
fn multipage_parts_keep_fallback_labels_and_unicode() {
    let d = drawing(false);
    let a = part(&d, &[(210.0, 297.0), (420.0, 297.0)], true);
    let b = part(&d, &[(594.0, 841.0)], false);
    let merged = merge_own(&[&a, &b], &["Tờ (1)".into()], "Bản vẽ").unwrap();
    let parsed = probe(&merged);
    assert_eq!(parsed.pages().len(), 3);
    let cat = std::str::from_utf8(parsed.body(parsed.root)).unwrap();
    assert!(cat.contains("0 << /P <FEFF00541EDD0020002800310029> >> 1 << /P (Original 2) >> 2 << /P (Original 1) >>"));
    assert_eq!(contents(&parsed), [contents(&probe(&a)), contents(&probe(&b))].concat());
    let again = probe(&merge_own(&[&merged], &[], "Again").unwrap());
    assert_eq!(contents(&again), contents(&parsed));
}

#[test]
fn merge_refuses_empty_and_page_object_limits() {
    assert_eq!(MAX_MERGE_OBJECTS, 2_000_000);
    assert!(merge_own(&[], &[], "Empty").is_err());
    let one = part(&drawing(false), &[(210.0, 297.0)], false);
    assert!(merge_own(&[one.as_slice(); MAX_PAGES + 1], &[], "Too many").is_err());
    let two = part(&drawing(false), &[(210.0, 297.0); 2], false);
    let boundary = merge_own(&[two.as_slice(); MAX_PAGES / 2], &[], "At the page limit").unwrap();
    assert_eq!(probe(&boundary).pages().len(), MAX_PAGES);
    assert!(merge_own(&[two.as_slice(); MAX_PAGES / 2 + 1], &[], "Total page overflow").is_err());
    let mut forged = one.clone();
    let xref = forged.windows(7).rposition(|w| w == b"xref\n0 ").unwrap() + 7;
    let end = xref + forged[xref..].iter().position(|b| *b == b'\n').unwrap();
    forged.splice(xref..end, (MAX_MERGE_OBJECTS + 2).to_string().bytes());
    assert!(merge_own(&[&forged], &[], "Too many objects").is_err());
}

#[test]
fn merge_keeps_annotations_and_user_unit() {
    let mut d = drawing(false);
    let handle = d.add(&Space::Model, Common::default(), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(10.0, 5.0, 0.0) })).unwrap();
    let options = PdfOptions { compress: false, links: vec![(handle, "https://example.com/(1 0 R)".into())], ..PdfOptions::default() };
    let large = PdfOptions { paper_mm: Some((841.0, 15000.0)), landscape: Some(false), ..PdfOptions::default() };
    let linked = pdf_items(&d, &[(Space::Model, large)], &options).unwrap();
    let small = part(&d, &[(210.0, 297.0)], false);
    let merged = probe(&merge_own(&[&small, &linked], &[], "Links").unwrap());
    let original = probe(&linked);
    let page = merged.pages()[1];
    let original_page = original.pages()[0];
    let unit = |bytes: &[u8]| {
        std::str::from_utf8(bytes).unwrap().split("/UserUnit ").nth(1).unwrap().split_whitespace().next().unwrap().parse::<f64>().unwrap()
    };
    assert!(unit(original_page) > 1.0);
    assert_eq!(unit(page), unit(original_page));
    assert!(std::str::from_utf8(merged.body(merged.root)).unwrap().contains("/Version /1.6"));
    let text = std::str::from_utf8(page).unwrap();
    let annot_id: usize = text.split("/Annots [").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    let annot = std::str::from_utf8(merged.body(annot_id)).unwrap();
    assert!(annot.contains("/Subtype /Link"));
    assert!(annot.contains(r"/URI (https://example.com/\(1%200%20R\))"));
}
