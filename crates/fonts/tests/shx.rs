//! SHX reader: every font here is built byte by byte in this file; the corpus test only reads
//! files a developer points `SHX_TEST_DIR` at and never copies them.

use std::path::{Path, PathBuf};

use cadcraft_fonts::shx::{MAX_GLYPHS, MAX_SHX_BYTES, ShxError, ShxFont, ShxKind};

const SHAPES_10: &[u8] = b"AutoCAD-86 shapes 1.0\r\n\x1a";
const SHAPES_11: &[u8] = b"AutoCAD-86 shapes 1.1\r\n\x1a";
const UNIFONT: &[u8] = b"AutoCAD-86 unifont 1.0\r\n\x1a";
const BIGFONT: &[u8] = b"AutoCAD-86 bigfont 1.0\r\n\x1a";

fn font_header(name: &str, above: u8, below: u8, modes: u8) -> Vec<u8> {
    let mut d = name.as_bytes().to_vec();
    d.extend_from_slice(&[0, above, below, modes, 0, 0]);
    d
}

/// A glyph body as the table stores it: a name, a NUL, then shape bytes ending in 0.
fn glyph(name: &str, ops: &[u8]) -> Vec<u8> {
    let mut d = name.as_bytes().to_vec();
    d.push(0);
    d.extend_from_slice(ops);
    d.push(0);
    d
}

fn shapes_image(magic: &[u8], header: Option<Vec<u8>>, glyphs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut entries: Vec<(u16, Vec<u8>)> = Vec::new();
    if let Some(h) = header {
        entries.push((0, h));
    }
    entries.extend(glyphs.iter().cloned());
    let first = entries.iter().map(|e| e.0).min().unwrap_or(0);
    let last = entries.iter().map(|e| e.0).max().unwrap_or(0);
    let mut out = magic.to_vec();
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&last.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (code, data) in &entries {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    }
    for (_, data) in &entries {
        out.extend_from_slice(data);
    }
    out.extend_from_slice(b"EOF");
    out
}

fn unifont_image(header: Vec<u8>, glyphs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = UNIFONT.to_vec();
    // The count includes the font header, which is stored as entry zero.
    out.extend_from_slice(&(glyphs.len() as u32 + 1).to_le_bytes());
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(&header);
    for (code, data) in glyphs {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn two_glyph_shapes() -> Vec<u8> {
    shapes_image(
        SHAPES_10,
        Some(font_header("TESTFONT", 8, 2, 0)),
        &[(0x41, glyph("A", &[0x02, 0x08, 0x04, 0x06])), (0x42, glyph("B", &[0x02, 0x08, 0x08, 0x00]))],
    )
}

fn two_glyph_unifont() -> Vec<u8> {
    unifont_image(
        font_header("UNITEST", 21, 7, 2),
        &[(0x41, glyph("A", &[0x02, 0x08, 0x04, 0x06])), (0x20ac, glyph("EURO", &[0x02, 0x08, 0x08, 0x00]))],
    )
}

#[test]
fn parses_synthetic_shapes_font() {
    let font = ShxFont::parse(&two_glyph_shapes()).unwrap();
    assert_eq!(font.kind, ShxKind::Shapes);
    assert_eq!(font.name, "TESTFONT");
    assert_eq!((font.above, font.below, font.modes), (8, 2, 0));
    assert_eq!(font.len(), 2);
    assert!(!font.is_empty());
    assert_eq!(font.glyph_bytes(0x41), Some(&glyph("A", &[0x02, 0x08, 0x04, 0x06])[..]));
    assert_eq!(font.glyph_bytes(0x42), Some(&glyph("B", &[0x02, 0x08, 0x08, 0x00])[..]));
    assert_eq!(font.glyph_bytes(0x43), None);
    // Shape 0 is the font header, not a drawable glyph.
    assert_eq!(font.glyph_bytes(0), None);
}

#[test]
fn shapes_1_1_has_the_same_table() {
    let img = shapes_image(SHAPES_11, Some(font_header("ROMANISH", 21, 7, 2)), &[(0x30, glyph("ZERO", &[0x02]))]);
    let font = ShxFont::parse(&img).unwrap();
    assert_eq!((font.kind, font.name.as_str(), font.above, font.below, font.modes), (ShxKind::Shapes, "ROMANISH", 21, 7, 2));
    assert_eq!(font.glyph_bytes(0x30), Some(&glyph("ZERO", &[0x02])[..]));
}

#[test]
fn shape_file_without_header_shape_parses() {
    let img = shapes_image(SHAPES_10, None, &[(130, glyph("BOX", &[0x01]))]);
    let font = ShxFont::parse(&img).unwrap();
    assert_eq!((font.name.as_str(), font.above, font.below, font.modes), ("", 0, 0, 0));
    assert_eq!(font.len(), 1);
    assert!(font.glyph_bytes(130).is_some());
}

#[test]
fn first_of_duplicate_codes_wins() {
    let img = shapes_image(SHAPES_10, Some(font_header("DUP", 1, 1, 0)), &[(7, glyph("ONE", &[1])), (7, glyph("TWO", &[2]))]);
    let font = ShxFont::parse(&img).unwrap();
    assert_eq!(font.len(), 1);
    assert_eq!(font.glyph_bytes(7), Some(&glyph("ONE", &[1])[..]));
}

#[test]
fn second_shape_zero_does_not_replace_the_header() {
    let img = shapes_image(SHAPES_10, Some(font_header("FIRST", 8, 2, 0)), &[(0, font_header("SECOND", 1, 1, 2)), (0x41, glyph("A", &[1]))]);
    let font = ShxFont::parse(&img).unwrap();
    assert_eq!((font.name.as_str(), font.above, font.below, font.modes), ("FIRST", 8, 2, 0));
    assert_eq!(font.len(), 1);
}

#[test]
fn unifont_code_zero_is_not_a_glyph() {
    let img = unifont_image(font_header("UNI0", 5, 1, 0), &[(0, glyph("NUL", &[1])), (0x41, glyph("A", &[1]))]);
    let font = ShxFont::parse(&img).unwrap();
    assert_eq!(font.glyph_bytes(0), None);
    assert_eq!(font.len(), 1);
    assert!(font.glyph_bytes(0x41).is_some());
}

#[test]
fn parses_synthetic_unifont() {
    let font = ShxFont::parse(&two_glyph_unifont()).unwrap();
    assert_eq!(font.kind, ShxKind::Unifont);
    assert_eq!(font.name, "UNITEST");
    assert_eq!((font.above, font.below, font.modes), (21, 7, 2));
    assert_eq!(font.len(), 2);
    assert_eq!(font.glyph_bytes(0x20ac), Some(&glyph("EURO", &[0x02, 0x08, 0x08, 0x00])[..]));
    assert_eq!(font.glyph_bytes(0x42), None);
}

#[test]
fn rejects_non_shx_and_bigfont() {
    for junk in [
        &b""[..],
        b"\0\0\0\0\0\0\0\0",
        b"AutoCAD Lock File 0.1\r\n",
        b"AutoCAD-586 bigfont 1.0\r\n\x1a\0\0\0\0",
        b"AutoCAD-86 shapes 2.0\r\n\x1a\0\0\0\0\0\0",
        b"MZ\x90\0\x03\0\0\0",
    ] {
        assert_eq!(ShxFont::parse(junk).unwrap_err(), ShxError::NotShx, "{junk:?}");
    }
    let mut big = BIGFONT.to_vec();
    big.extend_from_slice(&[0; 8]);
    assert_eq!(ShxFont::parse(&big).unwrap_err(), ShxError::Unsupported(ShxKind::Bigfont));
    // The header alone is enough to refuse it, whatever follows.
    assert_eq!(ShxFont::parse(BIGFONT).unwrap_err(), ShxError::Unsupported(ShxKind::Bigfont));
}

#[test]
fn truncated_table_is_error() {
    let img = two_glyph_shapes();
    // Header only, a cut count, and a table cut in its last entry.
    for cut in [SHAPES_10.len(), SHAPES_10.len() + 3, SHAPES_10.len() + 6 + 4 * 3 - 1] {
        assert_eq!(ShxFont::parse(img.get(..cut).unwrap_or_default()).unwrap_err(), ShxError::Truncated, "cut {cut}");
    }
    let uni = two_glyph_unifont();
    for cut in [UNIFONT.len(), UNIFONT.len() + 5, UNIFONT.len() + 6 + 3] {
        assert_eq!(ShxFont::parse(uni.get(..cut).unwrap_or_default()).unwrap_err(), ShxError::Truncated, "cut {cut}");
    }
}

#[test]
fn fake_offsets_are_error() {
    // A table entry whose length runs past the end of the file.
    let mut img = two_glyph_shapes();
    let len_at = SHAPES_10.len() + 6 + 4 + 2;
    img[len_at] = 0xff;
    img[len_at + 1] = 0xff;
    assert_eq!(ShxFont::parse(&img).unwrap_err(), ShxError::Truncated);

    // Lengths that sum past the end although each one fits alone.
    let mut sums = shapes_image(SHAPES_10, Some(font_header("S", 1, 1, 0)), &[(1, vec![0; 20]), (2, vec![0; 20])]);
    sums.truncate(sums.len() - 3 - 25);
    assert_eq!(ShxFont::parse(&sums).unwrap_err(), ShxError::Truncated);

    // A count that promises more entries than the file holds.
    let mut many = two_glyph_shapes();
    many[SHAPES_10.len() + 4] = 0xff;
    many[SHAPES_10.len() + 5] = 0xff;
    assert_eq!(ShxFont::parse(&many).unwrap_err(), ShxError::Truncated);

    // Unifont: a glyph length past the end, and a header length past the end.
    let mut uni = two_glyph_unifont();
    let first_glyph_len = UNIFONT.len() + 4 + 2 + 6 + "UNITEST".len() + 2;
    uni[first_glyph_len] = 0xff;
    assert_eq!(ShxFont::parse(&uni).unwrap_err(), ShxError::Truncated);
    let mut uni = two_glyph_unifont();
    uni[UNIFONT.len() + 4] = 0xff;
    uni[UNIFONT.len() + 5] = 0xff;
    assert_eq!(ShxFont::parse(&uni).unwrap_err(), ShxError::Truncated);
}

#[test]
fn malformed_font_header_is_error() {
    // Shape 0 without a name terminator, and with a name but no above/below/modes.
    for bad in [b"NONAMETERMINATOR".to_vec(), b"NAME\0\x08".to_vec()] {
        let img = shapes_image(SHAPES_10, Some(bad.clone()), &[(0x41, glyph("A", &[1]))]);
        assert_eq!(ShxFont::parse(&img).unwrap_err(), ShxError::Truncated);
        let uni = unifont_image(bad, &[(0x41, glyph("A", &[1]))]);
        assert_eq!(ShxFont::parse(&uni).unwrap_err(), ShxError::Truncated);
    }
}

#[test]
fn size_and_count_caps_apply_before_allocation() {
    let oversize = vec![0u8; MAX_SHX_BYTES + 1];
    assert_eq!(ShxFont::parse(&oversize).unwrap_err(), ShxError::TooLarge);

    let mut huge = UNIFONT.to_vec();
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    huge.extend_from_slice(&[0, 0]);
    assert_eq!(ShxFont::parse(&huge).unwrap_err(), ShxError::TooLarge);
    let mut over_cap = UNIFONT.to_vec();
    over_cap.extend_from_slice(&(MAX_GLYPHS as u32 + 2).to_le_bytes());
    over_cap.extend_from_slice(&[0, 0]);
    assert_eq!(ShxFont::parse(&over_cap).unwrap_err(), ShxError::TooLarge);
}

/// xorshift64*: a fixed seed keeps hostile runs reproducible without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn hostile_shx_never_panics() {
    let seeds = [two_glyph_shapes(), two_glyph_unifont(), shapes_image(SHAPES_11, None, &[(1, glyph("X", &[1, 2, 3]))])];

    // Every prefix of every valid font; the shapes' trailing EOF marker is the only part that may
    // be cut off without losing data.
    for (img, ignorable_tail) in [(&seeds[0], 3), (&seeds[1], 0), (&seeds[2], 3)] {
        for cut in 0..img.len() - ignorable_tail {
            assert!(ShxFont::parse(img.get(..cut).unwrap_or_default()).is_err(), "cut {cut} of {} parsed", img.len());
        }
        for cut in img.len() - ignorable_tail..=img.len() {
            assert!(ShxFont::parse(img.get(..cut).unwrap_or_default()).is_ok(), "cut {cut} of {} refused", img.len());
        }
    }

    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..1000 {
        let mut img = match rng.below(4) {
            0 => {
                // A supported magic in front, so the junk reaches the table parsers (large counts included).
                let mut v = [SHAPES_10, SHAPES_11, UNIFONT][rng.below(3)].to_vec();
                v.extend((0..rng.below(300)).map(|_| rng.next() as u8));
                v
            }
            n => seeds.get(n - 1).cloned().unwrap_or_default(),
        };
        for _ in 0..rng.below(6) {
            if img.is_empty() {
                break;
            }
            let at = rng.below(img.len());
            if let Some(b) = img.get_mut(at) {
                *b = rng.next() as u8;
            }
        }
        if rng.below(3) == 0 {
            img.truncate(rng.below(img.len() + 1));
        }
        let _ = ShxFont::parse(&img);
    }
}

fn shx_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            shx_files(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("shx")) {
            out.push(path);
        }
    }
}

/// Corpus files whose own tables are inconsistent (an entry runs past the end of the file or the
/// table is desynchronised); the strict reader must refuse them rather than guess.
const DAMAGED_IN_CORPUS: &[&str] = &["eur_____.shx", "vnarial.shx", "vnavant.shx", "vna_____.shx", "vnhel.shx", "vnrevu.shx"];

#[test]
#[ignore = "needs SHX_TEST_DIR pointing at a folder of .shx files"]
fn real_corpus_parses() {
    let dir = std::env::var("SHX_TEST_DIR").expect("set SHX_TEST_DIR to a folder of .shx files to run this test");
    let mut files = Vec::new();
    shx_files(Path::new(&dir), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .shx files under {dir}");

    let (mut shapes, mut unifont, mut bigfont, mut foreign, mut damaged) = (0, 0, 0, 0, 0);
    let mut glyphs = 0usize;
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let result = ShxFont::parse(&bytes);
        if bytes.starts_with(b"AutoCAD-86 shapes 1.0\r\n\x1a") || bytes.starts_with(b"AutoCAD-86 shapes 1.1\r\n\x1a") {
            if DAMAGED_IN_CORPUS.contains(&name.as_str()) {
                assert_eq!(result.unwrap_err(), ShxError::Truncated, "{name}");
                damaged += 1;
            } else {
                let font = result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert_eq!(font.kind, ShxKind::Shapes, "{name}");
                glyphs += font.len();
                shapes += 1;
            }
        } else if bytes.starts_with(b"AutoCAD-86 unifont 1.0\r\n\x1a") {
            let font = result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(font.kind, ShxKind::Unifont, "{name}");
            assert!(!font.name.is_empty() && !font.is_empty(), "{name}");
            glyphs += font.len();
            unifont += 1;
        } else if bytes.starts_with(b"AutoCAD-86 bigfont 1.0\r\n\x1a") {
            assert_eq!(result.unwrap_err(), ShxError::Unsupported(ShxKind::Bigfont), "{name}");
            bigfont += 1;
        } else {
            assert!(result.is_err(), "{name} is not a known SHX header but parsed");
            foreign += 1;
        }
    }
    println!(
        "SHX corpus {dir}: files={} shapes_ok={shapes} unifont_ok={unifont} bigfont_unsupported={bigfont} damaged_refused={damaged} not_shx={foreign} glyphs={glyphs}",
        files.len()
    );
}
