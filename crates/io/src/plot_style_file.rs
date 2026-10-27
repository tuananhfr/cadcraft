//! AutoCAD plot style table files (`.ctb` colour-dependent, `.stb` named) read into a
//! [`PlotStyleTable`].
//!
//! A file is a 48-byte text header (`PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec`, or
//! `STBVER1`), three little-endian `u32`s (Adler-32, decompressed size, compressed size) and a
//! zlib stream. Inside is text: `key=value` lines and `key{ … }` blocks, strings opened by a `"`
//! that is often never closed. The layout comes from public descriptions of the format; no
//! vendor file was copied.

use cadcraft_color::Rgb;
use cadcraft_doc::{EndStyle, FillStyle, JoinStyle, MAX_TABLE_FILE, PlotStyle, PlotStyleKind, PlotStyleTable};

/// A table read from a file, with what of it CADCraft can't use.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotStyleImport {
    pub table: PlotStyleTable,
    /// Styles whose own linetype was dropped: plot style linetypes aren't supported yet, so
    /// they print with the object's linetype.
    pub unsupported_linetypes: usize,
}

const MAGIC: &[u8] = b"PIAFILEVERSION_2.0,";
const CODEC: &[u8] = b",compress\r\npmzlibcodec";
const HEADER: usize = 48;
const DATA: usize = HEADER + 12;
/// Deeper nesting than a table needs (root, `plot_style`, one style) is a damaged file.
const MAX_DEPTH: usize = 8;
/// More entries than any table holds; bounds the work on a hostile file.
const MAX_STYLES: usize = 4096;

const OBJECT_LINETYPE: i64 = 31;
const OBJECT_LINEWEIGHT: i64 = 255;
/// AcCmEntityColor colour methods (ObjectARX): true colour and index colour.
const METHOD_RGB: u32 = 0xC2;
const METHOD_ACI: u32 = 0xC3;
const POLICY_GRAYSCALE: i64 = 2;

/// Whether `bytes` look like an AutoCAD plot style table file.
pub fn is_plot_style_file(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PIAFILEVERSION")
}

/// Read the `.ctb`/`.stb` file `bytes`; `name` (a file name or path) names the table, with the
/// extension its kind needs. Damaged, oversized or hostile files are an `Err`.
pub fn read_plot_style_file(name: &str, bytes: &[u8]) -> Result<PlotStyleImport, String> {
    let kind = header_kind(bytes)?;
    let text = decompress(bytes)?;
    let root = parse(&text)?;
    let lineweights = lineweight_table(&root);
    let mut unsupported = 0usize;
    let mut styles: Vec<PlotStyle> = Vec::new();
    let mut by_index: Vec<PlotStyle> = vec![PlotStyle::object(""); cadcraft_doc::CTB_STYLES];
    for (key, node) in block(&root, "plot_style").unwrap_or_default().iter().take(MAX_STYLES) {
        let Node::Block(fields) = node else { continue };
        match kind {
            PlotStyleKind::ColorDependent => {
                // Style `i` is index colour `i + 1`; keys past 254 have no colour.
                let Some(slot) = key.trim().parse::<usize>().ok().and_then(|i| by_index.get_mut(i)) else { continue };
                *slot = style(fields, &lineweights, &mut unsupported);
            }
            PlotStyleKind::Named => styles.push(style(fields, &lineweights, &mut unsupported)),
        }
    }
    if kind == PlotStyleKind::ColorDependent {
        styles = by_index;
    }
    let description = value(&root, "description").map(str::to_string).unwrap_or_default();
    let table = PlotStyleTable { name: table_name(name, kind), description, kind, styles }.sanitized();
    if table.name.is_empty() {
        return Err("the plot style table has no name".into());
    }
    Ok(PlotStyleImport { table, unsupported_linetypes: unsupported })
}

fn header_kind(bytes: &[u8]) -> Result<PlotStyleKind, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("not an AutoCAD plot style table (.ctb/.stb)".into());
    }
    let kind = match bytes.get(MAGIC.len()..MAGIC.len() + 7) {
        Some(b"CTBVER1") => PlotStyleKind::ColorDependent,
        Some(b"STBVER1") => PlotStyleKind::Named,
        _ => return Err("not a .ctb or .stb plot style table (another PIA file)".into()),
    };
    if bytes.get(MAGIC.len() + 7..HEADER) != Some(CODEC) {
        return Err("the plot style table is not zlib-compressed as expected".into());
    }
    Ok(kind)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
}

/// The table's text, decompressed with its size capped at [`MAX_TABLE_FILE`].
fn decompress(bytes: &[u8]) -> Result<String, String> {
    let truncated = || "the plot style table file is cut short".to_string();
    let checksum = u32_at(bytes, HEADER).ok_or_else(truncated)?;
    let declared = u32_at(bytes, HEADER + 4).ok_or_else(truncated)?;
    let packed = u32_at(bytes, HEADER + 8).ok_or_else(truncated)?;
    if usize::try_from(declared).map_or(true, |n| n > MAX_TABLE_FILE) {
        return Err("the plot style table is too large".into());
    }
    let end = usize::try_from(packed).ok().and_then(|n| DATA.checked_add(n)).ok_or_else(truncated)?;
    let body = bytes.get(DATA..end).ok_or_else(truncated)?;
    let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, MAX_TABLE_FILE).map_err(|e| match e.status {
        miniz_oxide::inflate::TINFLStatus::HasMoreOutput => "the plot style table is too large".to_string(),
        _ => "the plot style table file is damaged (bad compressed data)".to_string(),
    })?;
    // Public descriptions disagree on whether the checksum covers the packed or the unpacked
    // bytes, so either is accepted.
    if checksum != adler32(body) && checksum != adler32(&out) {
        return Err("the plot style table file is damaged (checksum mismatch)".into());
    }
    let end = out.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    let raw = out.get(..end).unwrap_or_default();
    // Older files are in the Windows code page, not UTF-8; Latin-1 keeps their ASCII intact.
    Ok(match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|b| char::from(*b)).collect(),
    })
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 bytes is the most that can be summed before `b` could overflow a u32.
    for chunk in data.chunks(5552) {
        for x in chunk {
            a += u32::from(*x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

#[derive(Clone, Debug)]
enum Node {
    Value(String),
    Block(Vec<(String, Node)>),
}

fn parse(text: &str) -> Result<Vec<(String, Node)>, String> {
    let unbalanced = || "the plot style table is damaged (unbalanced braces)".to_string();
    let mut stack: Vec<(String, Vec<(String, Node)>)> = vec![(String::new(), Vec::new())];
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line == "}" {
            if stack.len() < 2 {
                return Err(unbalanced());
            }
            let (key, items) = stack.pop().ok_or_else(unbalanced)?;
            stack.last_mut().ok_or_else(unbalanced)?.1.push((key, Node::Block(items)));
        } else if let Some((key, value)) = line.split_once('=') {
            let v = value.trim();
            let v = v.strip_prefix('"').unwrap_or(v);
            let v = v.strip_suffix('"').unwrap_or(v);
            stack.last_mut().ok_or_else(unbalanced)?.1.push((key.trim().to_string(), Node::Value(v.to_string())));
        } else if let Some(key) = line.strip_suffix('{') {
            if stack.len() > MAX_DEPTH {
                return Err("the plot style table is damaged (nested too deeply)".into());
            }
            stack.push((key.trim().to_string(), Vec::new()));
        }
    }
    match stack.pop() {
        Some((_, root)) if stack.is_empty() => Ok(root),
        _ => Err(unbalanced()),
    }
}

fn value<'a>(items: &'a [(String, Node)], key: &str) -> Option<&'a str> {
    items.iter().find_map(|(k, n)| match n {
        Node::Value(v) if k.eq_ignore_ascii_case(key) => Some(v.as_str()),
        _ => None,
    })
}

fn block<'a>(items: &'a [(String, Node)], key: &str) -> Option<&'a [(String, Node)]> {
    items.iter().find_map(|(k, n)| match n {
        Node::Block(b) if k.eq_ignore_ascii_case(key) => Some(b.as_slice()),
        _ => None,
    })
}

fn int(items: &[(String, Node)], key: &str) -> Option<i64> {
    value(items, key).and_then(|v| v.trim().parse::<i64>().ok())
}

/// `custom_lineweight_table`: index → millimetres.
fn lineweight_table(root: &[(String, Node)]) -> Vec<Option<f64>> {
    let mut table = vec![None; 255];
    for (k, n) in block(root, "custom_lineweight_table").unwrap_or_default() {
        if let (Ok(i), Node::Value(v)) = (k.trim().parse::<usize>(), n)
            && let Some(slot) = table.get_mut(i)
        {
            *slot = v.trim().parse::<f64>().ok().filter(|mm| mm.is_finite() && *mm >= 0.0);
        }
    }
    table
}

fn dropped_linetype(fields: &[(String, Node)]) -> bool {
    int(fields, "linetype").is_some_and(|lt| lt != OBJECT_LINETYPE)
}

/// A style colour: `None` keeps the object's colour (also for methods CADCraft doesn't plot by,
/// such as by layer or by pen).
fn color(v: i64) -> Option<Rgb> {
    let v = u32::try_from(v).ok().or_else(|| i32::try_from(v).ok().map(|x| x as u32))?;
    let low = v & 0x00FF_FFFF;
    match v >> 24 {
        METHOD_RGB => Some(Rgb::from_u32(low)),
        METHOD_ACI => u8::try_from(low).ok().filter(|i| *i >= 1).map(cadcraft_color::aci_rgb),
        _ => None,
    }
}

fn style(fields: &[(String, Node)], lineweights: &[Option<f64>], unsupported: &mut usize) -> PlotStyle {
    let mut s = PlotStyle::object(value(fields, "name").unwrap_or_default());
    s.description = value(fields, "description").unwrap_or_default().to_string();
    s.color = int(fields, "color").or_else(|| int(fields, "mode_color")).and_then(color);
    s.grayscale = int(fields, "color_policy").is_some_and(|p| p & POLICY_GRAYSCALE != 0);
    s.screening = int(fields, "screen").map_or(100, |v| v.clamp(0, 100) as u8);
    if dropped_linetype(fields) {
        *unsupported += 1;
    }
    s.lineweight = int(fields, "lineweight")
        .filter(|i| *i != OBJECT_LINEWEIGHT)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| lineweights.get(i).copied().flatten())
        .map(|mm| (mm.min(10.0) * 100.0).round() as u16);
    s.end = match int(fields, "end_style") {
        Some(0) => EndStyle::Butt,
        Some(1) => EndStyle::Square,
        Some(2) => EndStyle::Round,
        Some(3) => EndStyle::Diamond,
        _ => EndStyle::Object,
    };
    s.join = match int(fields, "join_style") {
        Some(0) => JoinStyle::Miter,
        Some(1) => JoinStyle::Bevel,
        Some(2) => JoinStyle::Round,
        Some(3) => JoinStyle::Diamond,
        _ => JoinStyle::Object,
    };
    s.fill = match int(fields, "fill_style") {
        Some(64) => FillStyle::Solid,
        Some(65) => FillStyle::Checkerboard,
        Some(66) => FillStyle::Crosshatch,
        Some(67) => FillStyle::Diamonds,
        Some(68) => FillStyle::HorizontalBars,
        Some(69) => FillStyle::SlantLeft,
        Some(70) => FillStyle::SlantRight,
        Some(71) => FillStyle::SquareDots,
        Some(72) => FillStyle::VerticalBars,
        _ => FillStyle::Object,
    };
    s
}

/// The file name of `name` (a path or a name) with the extension of `kind`.
fn table_name(name: &str, kind: PlotStyleKind) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let lower = base.to_ascii_lowercase();
    let stem = if lower.ends_with(".ctb") || lower.ends_with(".stb") { base.get(..base.len() - 4).unwrap_or(base) } else { base };
    let stem = stem.trim();
    if stem.is_empty() {
        return String::new();
    }
    format!("{stem}{}", if kind == PlotStyleKind::ColorDependent { ".ctb" } else { ".stb" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file holding `text`, laid out as the format is publicly described.
    fn file(kind: &str, text: &str) -> Vec<u8> {
        let mut raw = text.as_bytes().to_vec();
        raw.push(0);
        let body = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let mut out = format!("PIAFILEVERSION_2.0,{kind},compress\r\npmzlibcodec").into_bytes();
        assert_eq!(out.len(), HEADER);
        for n in [adler32(&body), raw.len() as u32, body.len() as u32] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend_from_slice(&body);
        out
    }

    fn lineweight_block() -> String {
        let mm = [0.0, 0.05, 0.09, 0.1, 0.13, 0.15, 0.18, 0.2, 0.25, 0.3, 0.35];
        let rows: String = mm.iter().enumerate().map(|(i, w)| format!(" {i}={w:.2}\n")).collect();
        format!("custom_lineweight_table{{\n{rows}}}\n")
    }

    fn ctb_style(i: usize, color: i64, lineweight: i64, linetype: i64) -> String {
        format!(
            " {i}{{\n  name=\"Color_{n}\n  localized_name=\"Color_{n}\n  description=\"\n  color={color}\n  mode_color={color}\n  color_policy=1\n  physical_pen_number=0\n  virtual_pen_number=0\n  screen=100\n  linepattern_size=0.5\n  linetype={linetype}\n  adaptive_linetype=TRUE\n  lineweight={lineweight}\n  fill_style=73\n  end_style=4\n  join_style=5\n }}\n",
            n = i + 1
        )
    }

    fn ctb(description: &str, style: impl Fn(usize) -> String) -> Vec<u8> {
        let styles: String = (0..255).map(style).collect();
        let text =
            format!("description=\"{description}\naci_table_available=TRUE\nscale_factor=1.0\nplot_style{{\n{styles}}}\n{}", lineweight_block());
        file("CTBVER1", &text)
    }

    const RGB_BLACK: i64 = 0xC200_0000u32 as i32 as i64;
    const OBJECT: i64 = 0xC400_0000u32 as i32 as i64;

    #[test]
    fn reads_monochrome_like_ctb() {
        let bytes = ctb("Every colour black", |i| ctb_style(i, RGB_BLACK, 7, 31));
        assert!(is_plot_style_file(&bytes));
        let r = read_plot_style_file("C:\\plot styles\\Office Mono.CTB", &bytes).unwrap();
        let t = &r.table;
        assert_eq!(t.name, "Office Mono.ctb");
        assert_eq!(t.description, "Every colour black");
        assert!(t.is_color_dependent());
        assert_eq!(t.styles.len(), 255);
        let s = t.by_color(1).unwrap();
        assert_eq!(s.color, Some(Rgb(0, 0, 0)));
        assert_eq!(s.lineweight, Some(20), "index 7 of the table is 0.20 mm");
        assert_eq!(t.by_color(255).unwrap().color, Some(Rgb(0, 0, 0)));
        assert_eq!(r.unsupported_linetypes, 0);
    }

    #[test]
    fn object_color_and_lineweight() {
        // The object colour has been described both as -1006632960 and as 0xC3000000 (which are
        // different numbers); both, and -1, keep the object's colour.
        let objects = [OBJECT, 0xC300_0000u32 as i32 as i64, 0xC3FF_FFFFu32 as i32 as i64, -1];
        let bytes = ctb("", |i| match i {
            0..4 => ctb_style(i, objects[i], 255, 31),
            4 => ctb_style(i, 0xC2FF_8000u32 as i32 as i64, 255, 2),
            5 => ctb_style(i, 0xC300_0005u32 as i32 as i64, 3, 31),
            _ => ctb_style(i, OBJECT, 255, 31),
        });
        let r = read_plot_style_file("x", &bytes).unwrap();
        let t = &r.table;
        assert_eq!(t.name, "x.ctb");
        for aci in 1..=4 {
            let s = t.by_color(aci).unwrap();
            assert_eq!((s.color, s.lineweight), (None, None), "Color_{aci}");
            assert!(s.keeps_color());
        }
        assert_eq!(t.by_color(5).unwrap().color, Some(Rgb(255, 128, 0)));
        assert_eq!(t.by_color(5).unwrap().linetype, None, "plot style linetypes aren't supported");
        assert_eq!(r.unsupported_linetypes, 1);
        assert_eq!(t.by_color(6).unwrap().color, Some(cadcraft_color::aci_rgb(5)), "an index colour");
        assert_eq!(t.by_color(6).unwrap().lineweight, Some(10));
        assert_eq!(t.by_color(7).unwrap().end, EndStyle::Object);
    }

    #[test]
    fn reads_named_stb() {
        let text = format!(
            "description=\"Office\naci_table_available=FALSE\nplot_style{{\n 0{{\n  name=\"Normal\n  color=-1\n  screen=100\n  linetype=31\n  lineweight=255\n }}\n 1{{\n  name=\"Thick\n  color={RGB_BLACK}\n  color_policy=3\n  screen=50\n  linetype=31\n  lineweight=10\n  end_style=2\n  join_style=0\n  fill_style=64\n }}\n}}\n{}",
            lineweight_block()
        );
        let r = read_plot_style_file("office.stb", &file("STBVER1", &text)).unwrap();
        let t = &r.table;
        assert_eq!(t.kind, PlotStyleKind::Named);
        assert_eq!(t.name, "office.stb");
        assert_eq!(t.styles.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Normal", "Thick"]);
        let thick = t.by_name("Thick").unwrap();
        assert_eq!((thick.color, thick.grayscale, thick.screening, thick.lineweight), (Some(Rgb(0, 0, 0)), true, 50, Some(35)));
        assert_eq!((thick.end, thick.join, thick.fill), (EndStyle::Round, JoinStyle::Miter, FillStyle::Solid));
        assert!(t.by_name("Normal").unwrap().keeps_color());
        // A named file keeps .stb even when its name says otherwise.
        assert_eq!(read_plot_style_file("office.ctb", &file("STBVER1", &text)).unwrap().table.name, "office.stb");
    }

    #[test]
    fn rejects_bad_files() {
        let good = ctb("", |i| ctb_style(i, RGB_BLACK, 255, 31));
        assert!(read_plot_style_file("ok.ctb", &good).is_ok());
        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", Vec::new()),
            ("header only", good.get(..HEADER).unwrap().to_vec()),
            ("JSON", b"{\"cadcraftPlotStyleTable\": 1}".to_vec()),
            ("printer config", good.iter().enumerate().map(|(i, b)| if (19..26).contains(&i) { b"PC3VER1"[i - 19] } else { *b }).collect()),
        ];
        let mut bad_sum = good.clone();
        bad_sum[HEADER] ^= 0xFF;
        cases.push(("checksum", bad_sum));
        let mut huge = good.clone();
        huge[HEADER + 4..HEADER + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(("declared size", huge));
        let mut long = good.clone();
        long[HEADER + 8..HEADER + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(("compressed size past the end", long));
        let cut = good.len() - 20;
        let mut truncated = good.get(..cut).unwrap().to_vec();
        let packed = (cut - DATA) as u32;
        truncated[HEADER + 8..HEADER + 12].copy_from_slice(&packed.to_le_bytes());
        cases.push(("truncated zlib", truncated));
        let mut garbage = good.clone();
        for b in garbage.iter_mut().skip(DATA + 2) {
            *b = 0x5A;
        }
        cases.push(("garbage stream", garbage));
        cases.push(("unbalanced", file("CTBVER1", "plot_style{\n 0{\n  name=\"x\n")));
        cases.push(("too deep", file("STBVER1", &"a{\n".repeat(64))));
        cases.push(("no name", good.clone()));
        for (what, bytes) in cases {
            let name = if what == "no name" { " .ctb" } else { "t.ctb" };
            assert!(read_plot_style_file(name, &bytes).is_err(), "{what}");
        }
    }

    #[test]
    fn a_zlib_bomb_is_refused_without_inflating_it_all() {
        // 64 MB of zeros packs into a few dozen kilobytes: more than the table limit.
        let zeros = vec![b' '; 64 << 20];
        let body = miniz_oxide::deflate::compress_to_vec_zlib(&zeros, 9);
        drop(zeros);
        let mut bytes = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec".to_vec();
        for n in [adler32(&body), 1000, body.len() as u32] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&body);
        let e = read_plot_style_file("bomb.ctb", &bytes).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn hostile_text_never_panics() {
        for text in [
            "plot_style{\n 99999999999999999999{\n  color=99999999999999\n  lineweight=-5\n  screen=-300\n }\n}\n",
            "plot_style{\n 0{\n  color=abc\n  lineweight=999\n }\n}\ncustom_lineweight_table{\n 999=1\n 3=NaN\n 4=1e300\n}\n",
            "}\n",
            "=\n{\n}\n",
        ] {
            let _ = read_plot_style_file("h.ctb", &file("CTBVER1", text));
            let _ = read_plot_style_file("h.stb", &file("STBVER1", text));
        }
        let r = read_plot_style_file("w.ctb", &file("CTBVER1", "plot_style{\n 0{\n  lineweight=4\n }\n}\ncustom_lineweight_table{\n 4=1e300\n}\n"))
            .unwrap();
        assert_eq!(r.table.by_color(1).unwrap().lineweight, Some(211), "clamped to the widest lineweight");
    }

    #[test]
    fn adler32_matches_the_reference_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }
}
