//! SHX compiled shape fonts: recognition and the glyph table (shapes 1.0/1.1, unifont 1.0).
//!
//! Layouts follow public descriptions plus black-box structure checks (counts only) on local fonts;
//! anything that does not fit exactly is a typed error.
mod interp;
mod shapes;
mod standard;
pub(crate) mod store;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use shapes::{FontHeader, Reader};

pub use interp::{ARC_SEGMENTS_PER_OCTANT, MAX_POINTS, MAX_SUBSHAPE_DEPTH, ShxGlyph};
pub use standard::{STANDARD_SHX, is_standard_shx};

pub const MAX_GLYPHS: usize = 65_536;
pub const MAX_SHX_BYTES: usize = 8 << 20;
/// The glyph cache of one font is dropped when it would hold more points than this.
pub const MAX_CACHED_POINTS: usize = 2_000_000;

const SHAPES_10: &[u8] = b"AutoCAD-86 shapes 1.0\r\n\x1a";
const SHAPES_11: &[u8] = b"AutoCAD-86 shapes 1.1\r\n\x1a";
const UNIFONT_10: &[u8] = b"AutoCAD-86 unifont 1.0\r\n\x1a";
const BIGFONT_10: &[u8] = b"AutoCAD-86 bigfont 1.0\r\n\x1a";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShxKind {
    Shapes,
    Unifont,
    Bigfont,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShxError {
    NotShx,
    Unsupported(ShxKind),
    Truncated,
    TooLarge,
}

impl std::fmt::Display for ShxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShxError::NotShx => f.write_str("not an SHX font"),
            ShxError::Unsupported(kind) => write!(f, "unsupported SHX kind {kind:?}"),
            ShxError::Truncated => f.write_str("SHX font is truncated or its table is inconsistent"),
            ShxError::TooLarge => f.write_str("SHX font exceeds the size limits"),
        }
    }
}

impl std::error::Error for ShxError {}

/// Whether `bytes` start with any SHX signature, readable or not.
pub fn has_shx_signature(bytes: &[u8]) -> bool {
    [SHAPES_10, SHAPES_11, UNIFONT_10, BIGFONT_10].iter().any(|magic| bytes.starts_with(magic))
}

/// Glyphs built so far, by code. The interpreter's work budget is per glyph, so text drawing
/// builds each glyph once and shares it.
#[derive(Default)]
struct GlyphCache(Mutex<CachedGlyphs>);

#[derive(Default)]
struct CachedGlyphs {
    glyphs: HashMap<u32, Arc<ShxGlyph>>,
    points: usize,
}

/// Bounds memory for fonts with tens of thousands of glyphs; dropping the cache only costs a rebuild.
const MAX_CACHED_GLYPHS: usize = 4096;

impl Clone for GlyphCache {
    fn clone(&self) -> Self {
        GlyphCache::default()
    }
}

// The cache never changes what a font is, so it takes no part in comparing fonts.
impl PartialEq for GlyphCache {
    fn eq(&self, _: &GlyphCache) -> bool {
        true
    }
}

impl Eq for GlyphCache {}

/// A parsed font. Glyph bodies are kept as stored (name, NUL, shape bytes); decoding them is the
/// job of the layers above.
#[derive(Clone, PartialEq, Eq)]
pub struct ShxFont {
    pub kind: ShxKind,
    pub name: String,
    pub above: u8,
    pub below: u8,
    pub modes: u8,
    glyphs: BTreeMap<u32, Vec<u8>>,
    built: GlyphCache,
}

// The glyph table is large; a font prints as what identifies it.
impl std::fmt::Debug for ShxFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShxFont").field("kind", &self.kind).field("name", &self.name).field("glyphs", &self.glyphs.len()).finish_non_exhaustive()
    }
}

impl ShxFont {
    pub fn parse(bytes: &[u8]) -> Result<ShxFont, ShxError> {
        if bytes.len() > MAX_SHX_BYTES {
            return Err(ShxError::TooLarge);
        }
        let (kind, body) = if let Some(rest) = bytes.strip_prefix(SHAPES_10).or_else(|| bytes.strip_prefix(SHAPES_11)) {
            (ShxKind::Shapes, rest)
        } else if let Some(rest) = bytes.strip_prefix(UNIFONT_10) {
            (ShxKind::Unifont, rest)
        } else if bytes.starts_with(BIGFONT_10) {
            return Err(ShxError::Unsupported(ShxKind::Bigfont));
        } else {
            return Err(ShxError::NotShx);
        };
        let (header, glyphs) = match kind {
            ShxKind::Unifont => parse_unifont(body)?,
            _ => shapes::parse(body)?,
        };
        Ok(ShxFont { kind, name: header.name, above: header.above, below: header.below, modes: header.modes, glyphs, built: GlyphCache::default() })
    }

    pub fn glyph_bytes(&self, code: u32) -> Option<&[u8]> {
        self.glyphs.get(&code).map(Vec::as_slice)
    }

    /// Points in the glyphs built so far.
    pub fn cached_points(&self) -> usize {
        self.built.0.lock().unwrap_or_else(PoisonError::into_inner).points
    }

    /// [`ShxFont::glyph`], built on first use and shared afterwards.
    pub fn glyph_shared(&self, code: u32) -> Option<Arc<ShxGlyph>> {
        if let Some(g) = self.built.0.lock().unwrap_or_else(PoisonError::into_inner).glyphs.get(&code) {
            return Some(g.clone());
        }
        let built = Arc::new(self.glyph(code)?);
        let points: usize = built.strokes.iter().map(Vec::len).sum();
        let mut cache = self.built.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(g) = cache.glyphs.get(&code) {
            return Some(g.clone());
        }
        if cache.glyphs.len() >= MAX_CACHED_GLYPHS || cache.points.saturating_add(points) > MAX_CACHED_POINTS {
            *cache = CachedGlyphs::default();
        }
        cache.points += points;
        cache.glyphs.insert(code, built.clone());
        Some(built)
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

/// Unifont: u32 entry count, u16 header length, the font header, then `(code u16, len u16, body)`
/// entries. The count includes the header itself, so it promises one glyph fewer.
fn parse_unifont(body: &[u8]) -> Result<(FontHeader, BTreeMap<u32, Vec<u8>>), ShxError> {
    let mut r = Reader::new(body);
    let entries = r.u32()?;
    if entries == 0 {
        return Err(ShxError::Truncated);
    }
    let glyph_count = usize::try_from(entries - 1).map_err(|_| ShxError::TooLarge)?;
    if glyph_count > MAX_GLYPHS {
        return Err(ShxError::TooLarge);
    }
    let header_len = usize::from(r.u16()?);
    let header = FontHeader::parse(r.take(header_len)?).ok_or(ShxError::Truncated)?;
    let mut glyphs = BTreeMap::new();
    for _ in 0..glyph_count {
        let code = u32::from(r.u16()?);
        let len = usize::from(r.u16()?);
        let data = r.take(len)?;
        // Code 0 is the header in shapes files; keep the two kinds consistent.
        if code != 0 {
            glyphs.entry(code).or_insert_with(|| data.to_vec());
        }
    }
    Ok((header, glyphs))
}
