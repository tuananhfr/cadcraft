//! Vector PDF plotting (PLOT / EXPORTPDF): a minimal PDF writer of our own (pure Rust, no file
//! access, so it runs on the web too).
//!
//! One page per sheet: model-space extents fitted unless a scale is given, or a layout at 1:1
//! (or the given `scale`). Paper units are millimetres in metric drawings, inches otherwise.
//! Lines are stroked paths (`m`/`l`/`S`), fills are `f` paths, colours are RGB with colour 7 printing black on white
//! paper, and line widths come from lineweights when enabled. The page setup's plot style
//! table (or the `plotStyleTable` option) sets the printed colours, linetypes and lineweights.
//!
//! Several sheets go into one file ([`pdf_sheets`], the `layouts` option), each page labelled
//! with its layout name. Drawing layers become PDF layers (optional content groups) that a PDF
//! viewer can switch on and off, and entity hyperlinks become link annotations.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cadcraft_color::Rgb;
use cadcraft_doc::{Drawing, Handle, MAX_PAPER_MM, MAX_PDF_BYTES, PageSetup, Space};
use cadcraft_geom::{Bounds2, Vec2};
use cadcraft_render::{Kind, Sheet, clip, paper};
use serde_json::Value;

use crate::{IoError, Result};

mod fonts;

/// PDF points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// Most sheets (pages) in one file.
pub const MAX_PAGES: usize = cadcraft_doc::MAX_PLOT_PAGES;
/// Most PDF layers in one file; objects on further layers print without one.
const MAX_LAYERS: usize = 4096;
/// Most link annotations in one file, and the longest link target (bytes).
const MAX_LINKS: usize = 10_000;
const MAX_URL: usize = 2048;

/// A rectangular plot area in world coordinates; rotation is counter-clockwise in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotFrame {
    pub center: Vec2,
    pub size: Vec2,
    pub rotation: f64,
}

impl PlotFrame {
    fn from_outline(v: &Value) -> Result<Self> {
        let invalid = || IoError::Format("outline must contain exactly four finite rectangle vertices".into());
        let raw = v.as_array().filter(|a| a.len() == 4).ok_or_else(invalid)?;
        let mut points = [Vec2::ZERO; 4];
        for (p, raw) in points.iter_mut().zip(raw) {
            let pair = raw.as_array().filter(|a| a.len() == 2).ok_or_else(invalid)?;
            *p = Vec2::new(pair.first().and_then(Value::as_f64).ok_or_else(invalid)?, pair.get(1).and_then(Value::as_f64).ok_or_else(invalid)?);
        }
        let (center, size, rotation) = cadcraft_render::rectangle_frame(&points).ok_or_else(invalid)?;
        Ok(Self { center, size, rotation })
    }

    fn bounds(self) -> Result<Bounds2> {
        let b = Bounds2::new(self.center - self.size * 0.5, self.center + self.size * 0.5);
        if !self.center.is_finite()
            || !self.size.is_finite()
            || self.size.x <= 0.0
            || self.size.y <= 0.0
            || !self.rotation.is_finite()
            || !b.min.is_finite()
            || !b.max.is_finite()
            || !b.width().is_finite()
            || !b.height().is_finite()
            || b.width() <= 0.0
            || b.height() <= 0.0
        {
            return Err(IoError::Format("plot frame must have a finite centre, positive size and finite rotation".into()));
        }
        Ok(b)
    }
}

/// Plot settings. `None` fields fall back to the layout's page setup (or sensible defaults for
/// model space).
#[derive(Clone, Debug)]
pub struct PdfOptions {
    pub cancel: Option<Arc<AtomicBool>>,
    pub max_prims: Option<usize>,
    /// Plot style table name, replacing the page setup's (`"None"` plots without one).
    pub plot_style_table: Option<String>,
    /// Paper name from [`cadcraft_render::PAPER_SIZES`] (`"A4"`, `"Letter"`, `"ANSI B"`…).
    pub paper: Option<String>,
    /// Custom paper size in millimetres (portrait width, height).
    pub paper_mm: Option<(f64, f64)>,
    pub landscape: Option<bool>,
    /// Fit the drawing to the printable area. Default: on for model space/windows without a scale.
    pub fit: Option<bool>,
    /// Plot scale in paper units per drawing unit (when not fitting).
    pub scale: Option<f64>,
    /// Model-space plot placement. `None` keeps the legacy centered behavior.
    pub center: Option<bool>,
    /// Model-space X/Y plot offset in millimetres from the printable area's lower-left corner.
    pub offset: Option<[f64; 2]>,
    /// Plot object lineweights. Default: the layout's setting; on for model space.
    pub lineweights: Option<bool>,
    /// Flate-compress the content stream.
    pub compress: bool,
    pub title: String,
    /// Model space only: plot this rectangle (drawing units) instead of the extents. Ignored for layouts.
    pub window: Option<Bounds2>,
    /// Rotate the display list into this frame before placing its local rectangle.
    pub frame: Option<PlotFrame>,
    /// Page label, replacing the space name.
    pub label: Option<String>,
    pub seams: Option<[bool; 4]>,
    pub seam_overlap_mm: Option<f64>,
    /// Drawing layers become PDF layers (optional content groups). Default on.
    pub layers: bool,
    /// With `layers`: objects on layers that are off are included on PDF layers that start
    /// hidden (frozen layers are never included). Default off: they aren't plotted.
    pub include_off_layers: bool,
    /// Hyperlinks: an entity's handle and its link target (a URL). The area the entity covers
    /// on the sheet becomes a link annotation.
    pub links: Vec<(Handle, String)>,
    /// Write text in installed TrueType fonts as filled outlines instead of as text in an
    /// embedded font subset (like AutoCAD's PDFSHX for its own fonts). Stroke-font text is
    /// always geometry.
    pub text_as_geometry: bool,
    /// Margins (left, bottom, right, top; mm) of the plot device's paper, bounding the printable
    /// area of a model-space plot. `None` keeps the page setup's (none for model space).
    pub margins: Option<[f64; 4]>,
    /// `margins` also replace a layout's page setup margins (a publish page that names its own).
    pub page_margins: bool,
    /// Entity hyperlinks read from a drawing file (`PE_URL` extended data) become link
    /// annotations too, after [`PdfOptions::links`]. Default on.
    pub hyperlinks: bool,
    /// Plot objects with their transparency (PDF alpha). `None` = the page setup's "Plot
    /// transparency" (off for model space).
    pub plot_transparency: Option<bool>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions {
            cancel: None,
            max_prims: None,
            plot_style_table: None,
            paper: None,
            paper_mm: None,
            landscape: None,
            fit: None,
            scale: None,
            center: None,
            offset: None,
            lineweights: None,
            compress: false,
            title: String::new(),
            window: None,
            frame: None,
            label: None,
            seams: None,
            seam_overlap_mm: None,
            layers: true,
            include_off_layers: false,
            links: Vec::new(),
            text_as_geometry: false,
            margins: None,
            page_margins: false,
            hyperlinks: true,
            plot_transparency: None,
        }
    }
}

impl PdfOptions {
    /// Parse `{paper?, width?, height?, landscape?, fit?, scale?, center?, offset?: [x,y] mm,
    /// lineweights?, compress?, title?, window?: [[x1, y1], [x2, y2]], margins?: [l, b, r, t] (mm),
    /// hyperlinks?: bool, plotTransparency?: bool}`.
    pub fn from_json(v: &Value) -> PdfOptions {
        let num = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        let paper_mm = match (num("width"), num("height")) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
            _ => None,
        };
        PdfOptions {
            cancel: None,
            max_prims: None,
            // `layers?` (default on), `offLayers?`, `links?: [{handle, url}]`, `text?: "text" |
            // "geometry"`.
            text_as_geometry: v.get("text").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("geometry")),
            layers: v.get("layers").and_then(Value::as_bool).unwrap_or(true),
            include_off_layers: v.get("offLayers").and_then(Value::as_bool).unwrap_or(false),
            links: links_json(v.get("links")),
            plot_style_table: v.get("plotStyleTable").and_then(Value::as_str).map(|s| s.chars().take(260).collect()),
            paper: v.get("paper").and_then(Value::as_str).map(str::to_string),
            paper_mm,
            landscape: v.get("landscape").and_then(Value::as_bool),
            fit: v.get("fit").and_then(Value::as_bool),
            scale: num("scale").filter(|s| *s > 0.0),
            center: v.get("center").and_then(Value::as_bool),
            offset: offset(v.get("offset")),
            lineweights: v.get("lineweights").and_then(Value::as_bool),
            compress: v.get("compress").and_then(Value::as_bool).unwrap_or(true),
            title: v.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            window: window(v.get("window")),
            frame: v.get("outline").and_then(|v| PlotFrame::from_outline(v).ok()),
            label: v.get("label").and_then(Value::as_str).map(str::to_string),
            seams: v.get("seams").and_then(|value| {
                let [left, bottom, right, top] = value.as_array()?.as_slice() else { return None };
                Some([left.as_bool()?, bottom.as_bool()?, right.as_bool()?, top.as_bool()?])
            }),
            seam_overlap_mm: num("seamOverlapMm").filter(|value| (0.0..=200.0).contains(value)),
            margins: margins(v.get("margins")),
            page_margins: false,
            hyperlinks: v.get("hyperlinks").and_then(Value::as_bool).unwrap_or(true),
            plot_transparency: v.get("plotTransparency").and_then(Value::as_bool),
        }
    }
}

fn offset(v: Option<&Value>) -> Option<[f64; 2]> {
    let a = v?.as_array()?;
    if a.len() != 2 {
        return None;
    }
    let x = a.first()?.as_f64()?;
    let y = a.get(1)?.as_f64()?;
    valid_plot_offset([x, y]).then_some([x, y])
}

fn valid_plot_offset(offset: [f64; 2]) -> bool {
    offset.iter().all(|v| v.is_finite() && v.abs() <= paper::MAX_PLOT_OFFSET_MM)
}

/// Reject malformed offsets at the JSON plotting boundary, before a permissive option parser
/// could treat them as absent.
pub(crate) fn validate_plot_offset_json(v: &Value) -> Result<()> {
    if let Some(raw) = v.get("offset")
        && offset(Some(raw)).is_none()
    {
        return Err(IoError::Format(format!(
            "plot offset must contain exactly two finite values from -{} to {} mm",
            paper::MAX_PLOT_OFFSET_MM,
            paper::MAX_PLOT_OFFSET_MM
        )));
    }
    Ok(())
}

/// `[l, b, r, t]` margins in mm (finite, 0–1000), else `None`.
pub(crate) fn margins(v: Option<&Value>) -> Option<[f64; 4]> {
    let a: Vec<f64> = v?.as_array()?.iter().take(5).filter_map(Value::as_f64).collect();
    match a[..] {
        [l, b, r, t] if a.iter().all(|x| x.is_finite() && (0.0..=1000.0).contains(x)) => Some([l, b, r, t]),
        _ => None,
    }
}

/// A plot window from `[[x1, y1], [x2, y2]]`: finite corners and a positive size, else `None`.
fn window(v: Option<&Value>) -> Option<Bounds2> {
    let corner = |i: usize| {
        let c = v?.get(i)?;
        let p = Vec2::new(c.get(0)?.as_f64()?, c.get(1)?.as_f64()?);
        p.is_finite().then_some(p)
    };
    let (a, b) = (corner(0)?, corner(1)?);
    let w = Bounds2::new(a, b);
    (w.width() > 0.0 && w.height() > 0.0).then_some(w)
}

/// Plot with JSON options (the engine's `plot` hook).
pub fn plot(d: &Drawing, space: &Space, opts: &Value) -> Result<Vec<u8>> {
    plot_with_budget(d, space, opts, None, None)
}

pub fn plot_with_budget(d: &Drawing, space: &Space, opts: &Value, cancel: Option<Arc<AtomicBool>>, max_prims: Option<usize>) -> Result<Vec<u8>> {
    validate_plot_offset_json(opts)?;
    // `layouts: [name | "Model"] | "all"` plots those sheets into one file, one page each,
    // instead of `space`; "all" = Model if it has objects, then the layouts in tab order.
    let mut o = PdfOptions::from_json(opts);
    o.cancel = cancel;
    o.max_prims = max_prims;
    if let Some(outline) = opts.get("outline") {
        PlotFrame::from_outline(outline)?;
    }
    if let Some(pages) = opts.get("pages") {
        let list = pages.as_array().ok_or_else(|| IoError::Format("`pages` must be a list of page objects".into()))?;
        if list.len() > MAX_PAGES {
            return Err(IoError::Format(format!("at most {MAX_PAGES} sheets fit in one PDF; publish it in parts (split)")));
        }
        let items = list
            .iter()
            .map(|pg| {
                if ["window", "outline", "layout"].iter().filter(|key| pg.get(**key).is_some()).count() != 1 {
                    return Err(IoError::Format("each page needs exactly one of window, outline, layout".into()));
                }
                let page_space = if let Some(raw) = pg.get("layout") {
                    let name = raw.as_str().ok_or_else(|| IoError::Format("page layout must be a layout name".into()))?;
                    let layout = d.layout(name).ok_or_else(|| IoError::Format(format!("no layout `{name}`")))?;
                    Space::Paper(layout.name.clone())
                } else if pg.get("window").is_some() {
                    Space::Model
                } else if let Some(raw) = pg.get("space") {
                    // Mixed-space frame plans must crop each outline in its owning space.
                    let name = raw.as_str().ok_or_else(|| IoError::Format("page space must be a space name".into()))?;
                    if name.eq_ignore_ascii_case("Model") {
                        Space::Model
                    } else {
                        let layout = d.layout(name).ok_or_else(|| IoError::Format(format!("no layout `{name}`")))?;
                        Space::Paper(layout.name.clone())
                    }
                } else {
                    space.clone()
                };
                let mut po = PdfOptions::from_json(&serde_json::json!({
                    "paper": pg.get("paper"), "landscape": pg.get("landscape"), "scale": pg.get("scale"), "fit": pg.get("fit"),
                    "width": pg.get("width"), "height": pg.get("height"), "label": pg.get("label"),
                    "seams": pg.get("seams"), "seamOverlapMm": pg.get("seamOverlapMm"),
                    "plotStyleTable": opts.get("plotStyleTable"), "lineweights": opts.get("lineweights"),
                    "margins": pg.get("margins").or_else(|| opts.get("margins")), "text": opts.get("text"),
                }));
                po.page_margins = pg.get("margins").is_some();
                if let Some(raw) = pg.get("outline") {
                    po.frame = Some(PlotFrame::from_outline(raw)?);
                } else if let Some(raw) = pg.get("window") {
                    po.window = Some(window(Some(raw)).ok_or_else(|| IoError::Format("each page needs a `window` with two corners".into()))?);
                }
                if po.landscape.is_none()
                    && let Some((w, h)) = po.paper_mm
                {
                    po.landscape = Some(w > h);
                }
                po.compress = o.compress;
                po.cancel = o.cancel.clone();
                po.max_prims = o.max_prims;
                Ok((page_space, po))
            })
            .collect::<Result<Vec<_>>>()?;
        return pdf_items(d, &items, &o);
    }
    match opts.get("layouts") {
        Some(Value::String(a)) if a.eq_ignore_ascii_case("all") => {
            let mut layouts: Vec<_> = d.layouts.iter().collect();
            layouts.sort_by_key(|l| l.tab_order);
            let model = (!d.model.is_empty()).then_some(Space::Model);
            let spaces: Vec<Space> = model.into_iter().chain(layouts.iter().map(|l| Space::Paper(l.name.clone()))).collect();
            pdf_sheets(d, &spaces, &o)
        }
        Some(v) => {
            let names = v.as_array().ok_or_else(|| IoError::Format("`layouts` must be a list of layout names".into()))?;
            if names.len() > MAX_PAGES {
                return Err(IoError::Format(format!("at most {MAX_PAGES} sheets fit in one PDF; publish it in parts (split)")));
            }
            let spaces = names
                .iter()
                .map(|n| match n.as_str() {
                    Some(n) if n.eq_ignore_ascii_case("model") => Ok(Space::Model),
                    Some(n) => d.layout(n).map(|l| Space::Paper(l.name.clone())).ok_or_else(|| IoError::Format(format!("no layout `{n}`"))),
                    None => Err(IoError::Format("`layouts` must be a list of layout names".into())),
                })
                .collect::<Result<Vec<_>>>()?;
            pdf_sheets(d, &spaces, &o)
        }
        None => pdf(d, space, &o),
    }
}

/// `[{handle: "1A" | 26, url}]` → (handle, url) pairs; malformed entries are skipped.
fn links_json(v: Option<&Value>) -> Vec<(Handle, String)> {
    let Some(a) = v.and_then(Value::as_array) else { return Vec::new() };
    a.iter()
        .take(MAX_LINKS)
        .filter_map(|l| {
            let h = l.get("handle")?;
            let h = h.as_str().and_then(Handle::parse_hex).or_else(|| h.as_u64().map(Handle))?;
            let url = l.get("url")?.as_str()?.trim();
            (!url.is_empty()).then(|| (h, url.to_string()))
        })
        .collect()
}

/// The page setup a plot of `space` uses, with overrides applied.
pub fn page_for(d: &Drawing, space: &Space, o: &PdfOptions) -> Result<PageSetup> {
    let mut page = match space.layout() {
        Some(n) => d.layout(n).map(|l| l.page.clone()).ok_or_else(|| IoError::Format(format!("no layout `{n}`")))?,
        None => {
            let mut p = PageSetup::default();
            if paper::paper_unit_mm(d) == 1.0
                && let Some(a4) = paper::paper_size("A4")
            {
                p.paper = a4.name.into();
                p.width_mm = a4.width_mm;
                p.height_mm = a4.height_mm;
            }
            p
        }
    };
    if let Some(name) = &o.paper {
        let ps = paper::paper_size(name).ok_or_else(|| IoError::Format(format!("unknown paper size `{name}`")))?;
        page.paper = ps.name.into();
        page.width_mm = ps.width_mm;
        page.height_mm = ps.height_mm;
    }
    if let Some((w, h)) = o.paper_mm {
        page.paper = format!("User ({w:.2} x {h:.2} MM)");
        page.width_mm = w;
        page.height_mm = h;
    }
    if let Some(l) = o.landscape {
        page.landscape = l;
    }
    if let Some(t) = &o.plot_style_table {
        page.plot_style_table = t.clone();
    }
    // A layout keeps its page setup margins unless a publish page names its own.
    if let Some(m) = o.margins
        && (matches!(space, Space::Model) || o.page_margins)
    {
        page.margins_mm = m;
    }
    if [page.width_mm, page.height_mm].iter().any(|side| !side.is_finite() || *side <= 0.0 || *side > MAX_PAPER_MM) {
        return Err(IoError::Format(format!("paper sides must be finite, positive and at most {MAX_PAPER_MM} mm")));
    }
    page.width_mm = page.width_mm.max(1.0);
    page.height_mm = page.height_mm.max(1.0);
    Ok(page)
}

/// World → paper-unit mapping: `p' = (p - from) * s + to`.
#[derive(Clone, Copy)]
struct Map {
    from: Vec2,
    s: f64,
    to: Vec2,
    /// Points per paper unit.
    k: f64,
}

impl Map {
    fn pt(&self, p: Vec2) -> Vec2 {
        ((p - self.from) * self.s + self.to) * self.k
    }
}

/// Write a one-page vector PDF of a space.
pub fn pdf(d: &Drawing, space: &Space, o: &PdfOptions) -> Result<Vec<u8>> {
    pdf_sheets(d, std::slice::from_ref(space), o)
}

/// The name of a sheet: `Model` or the layout's.
fn sheet_name(space: &Space) -> String {
    space.tab_name().to_string()
}

/// Write a vector PDF with one page per space (at most [`MAX_PAGES`]), each labelled with its
/// layout name. The options apply to every sheet; layouts use their own page setups.
pub fn pdf_sheets(d: &Drawing, spaces: &[Space], o: &PdfOptions) -> Result<Vec<u8>> {
    let items: Vec<(Space, PdfOptions)> = spaces.iter().take(MAX_PAGES + 1).map(|s| (s.clone(), o.clone())).collect();
    pdf_items(d, &items, o)
}

/// Write a vector PDF with one page per `(space, options)` item (at most [`MAX_PAGES`]): each
/// page plots with its own options (window, paper, orientation); `o` supplies the file-wide
/// ones (compression, title, layers, links).
pub fn pdf_items(d: &Drawing, items: &[(Space, PdfOptions)], o: &PdfOptions) -> Result<Vec<u8>> {
    check_cancelled(o)?;
    let bounded;
    let items = if o.cancel.is_some() || o.max_prims.is_some() {
        bounded = items
            .iter()
            .take(MAX_PAGES + 1)
            .map(|(space, options)| {
                let mut options = options.clone();
                if o.cancel.is_some() {
                    options.cancel = o.cancel.clone();
                }
                options.max_prims = match (options.max_prims, o.max_prims) {
                    (Some(page), Some(file)) => Some(page.min(file)),
                    (page, file) => page.or(file),
                };
                (space.clone(), options)
            })
            .collect::<Vec<_>>();
        bounded.as_slice()
    } else {
        items
    };
    if items.iter().any(|(_, io)| io.offset.is_some_and(|offset| !valid_plot_offset(offset))) {
        return Err(IoError::Format("plot offset must contain two finite values within ±100000 mm".into()));
    }
    if items.is_empty() {
        return Err(IoError::Format("there are no sheets to plot".into()));
    }
    if items.len() > MAX_PAGES {
        return Err(IoError::Format(format!("at most {MAX_PAGES} sheets fit in one PDF; publish it in parts (split)")));
    }
    // Layers that are off plot on PDF layers that start hidden: draw them from a copy with them on.
    let with_off;
    let drawn = if o.layers && o.include_off_layers && d.layers.iter().any(|l| !l.on && !l.frozen) {
        let mut c = d.clone();
        for l in c.layers.iter_mut().filter(|l| !l.frozen) {
            l.on = true;
        }
        with_off = c;
        &with_off
    } else {
        d
    };
    let links = all_links(d, o);
    let fonts = if !o.text_as_geometry && items.len() > 1 {
        let mut glyphs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (space, options) in items {
            let page = sheet(drawn, space, options, &links)?;
            for (face, gid, ch) in fonts::glyphs(&page.list) {
                // Repeated pages need only one entry per face/glyph, preserving first-use order.
                if seen.insert((face.bytes.as_ptr() as usize, face.bytes.len(), face.index, gid)) {
                    glyphs.push((face, gid, ch));
                }
            }
        }
        fonts::Fonts::collect(glyphs)
    } else {
        fonts::Fonts::default()
    };
    let bytes = assemble(d, items.iter().map(|(space, options)| sheet(drawn, space, options, &links)), fonts, o)?;
    check_cancelled(o)?;
    for (_, options) in items {
        check_cancelled(options)?;
    }
    Ok(bytes)
}

/// One plotted sheet, ready to be written as a page.
struct SheetPage {
    /// Page size in points.
    media: Vec2,
    user_unit: f64,
    /// What it draws, the mapping to points and the plot area (points).
    list: cadcraft_render::DisplayList,
    map: Map,
    clip_pt: Bounds2,
    /// The drawing layers (indices in the layer table) of the content's `/L<i>` marks.
    layers: Vec<u32>,
    /// Link annotations: rectangle in points and target.
    links: Vec<(Bounds2, String)>,
    /// Page label: the layout name.
    label: String,
    seams: Option<[bool; 4]>,
    seam_overlap_mm: f64,
}

fn check_cancelled(o: &PdfOptions) -> Result<()> {
    if o.cancel.as_ref().is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
        return Err(IoError::Format("plot cancelled".into()));
    }
    Ok(())
}

/// Plot one space.
fn sheet(d: &Drawing, space: &Space, o: &PdfOptions, links_to: &[(Handle, String)]) -> Result<SheetPage> {
    check_cancelled(o)?;
    let page = page_for(d, space, o)?;
    let unit_mm = paper::paper_unit_mm(d);
    let sheet = Sheet::from_page(&page, unit_mm);
    let lineweights = o.lineweights.unwrap_or(if space.is_paper() { page.lineweights } else { true });
    let plot_style_table = cadcraft_doc::plot_style_table(d, &page.plot_style_table).map(std::sync::Arc::new);
    let ropts = cadcraft_render::Options {
        transparency: o.plot_transparency.unwrap_or(page.plot_transparency),
        tolerance: 0.001,
        min_dash: 0.0,
        text: true,
        fill: true,
        lineweights,
        view_height: 0.0,
        plot_style_table,
        view: None,
        skip_viewport: None,
        cancel: o.cancel.clone(),
    };
    let media_pt = sheet.size * (unit_mm * PT_PER_MM);
    let user_unit = (media_pt.x.max(media_pt.y) / 14400.0).ceil().max(1.0);
    let k = unit_mm * PT_PER_MM / user_unit;
    let frame_bounds = o.frame.map(PlotFrame::bounds).transpose()?;
    let window = frame_bounds.or_else(|| o.window.filter(|w| matches!(space, Space::Model) && !w.is_empty()));
    let fit = o.fit.unwrap_or(o.scale.is_none() && (window.is_some() || matches!(space, Space::Model)));
    // Chord tolerance: about 0.05 mm on paper.
    let est = plot_scale(&window.unwrap_or_else(|| d.extents(space)), &sheet, fit, o.scale);
    let tol = 0.05 / unit_mm / est.max(1e-300);
    let tolerance = if tol.is_finite() && tol > 0.0 { tol } else { ropts.tolerance };
    // Relative point sizes follow the plotted window (else the extents).
    let view_height = window.map(|w| w.height()).unwrap_or(0.0);
    let mut list = cadcraft_render::build_plot(d, space, &cadcraft_render::Options { tolerance, view_height, ..ropts });
    check_cancelled(o)?;
    if let Some(frame) = o.frame {
        list.rotate_about(frame.center, -frame.rotation);
    }
    let b = window.unwrap_or(list.bounds);
    let s = plot_scale(&b, &sheet, fit, o.scale);
    if let Some(window) = window.filter(|_| matches!(space, Space::Model)) {
        cull_window(&mut list, window, s * unit_mm);
    }
    if o.max_prims.is_some_and(|limit| list.prims.len() > limit) {
        return Err(IoError::Format(format!("SHEET_TOO_LARGE: {} primitives exceed the sheet budget", list.prims.len())));
    }
    let map = if fit || window.is_some() || matches!(space, Space::Model) {
        let (from, to) = placement(space, &b, fit || window.is_some(), &sheet, o);
        Map { from, s, to, k }
    } else {
        Map { from: Vec2::ZERO, s, to: Vec2::ZERO, k }
    };
    // Plot style ends, joins and fill patterns at the plot scale (a layout's are already drawn).
    cadcraft_render::pen::apply(&mut list, map.s * unit_mm);
    check_cancelled(o)?;
    if o.max_prims.is_some_and(|limit| list.prims.len() > limit) {
        return Err(IoError::Format(format!("SHEET_TOO_LARGE: {} primitives exceed the sheet budget", list.prims.len())));
    }
    let media = sheet.size * k;
    let clip_pt = if fit || window.is_some() || o.page_margins || matches!(space, Space::Model) {
        Bounds2::new(sheet.printable.min * k, sheet.printable.max * k)
    } else {
        Bounds2::new(Vec2::ZERO, media)
    };
    // The layers drawn on, in order of first use.
    let mut layers: Vec<u32> = Vec::new();
    if o.layers {
        let mut seen = std::collections::HashSet::new();
        for l in &list.prim_layers {
            if (*l as usize) < d.layers.len() && layers.len() < MAX_LAYERS && seen.insert(*l) {
                layers.push(*l);
            }
        }
    }
    let links = links(&list, &map, &clip_pt, links_to, user_unit);
    Ok(SheetPage {
        media,
        user_unit,
        list,
        map,
        clip_pt,
        layers,
        links,
        label: o.label.clone().unwrap_or_else(|| sheet_name(space)),
        seams: o.seams,
        seam_overlap_mm: o.seam_overlap_mm.unwrap_or(20.0),
    })
}

fn cull_window(list: &mut cadcraft_render::DisplayList, window: Bounds2, mm_per_unit: f64) {
    if !(mm_per_unit.is_finite() && mm_per_unit > 0.0) {
        return;
    }
    let max_lw = list.prims.iter().map(|p| f64::from(p.lw)).filter(|lw| lw.is_finite()).fold(0.05, f64::max);
    // Five full widths cover pen::apply's ten-half-width miter limit; 0.05 mm also allows for hairlines.
    let area = window.expand(5.0 * max_lw / mm_per_unit);
    if area.is_empty() || !area.min.is_finite() || !area.max.is_finite() {
        return;
    }
    let intersects = |b: Bounds2| b.is_empty() || !b.min.is_finite() || !b.max.is_finite() || area.intersects(&b);
    let mut next = 0u32;
    let remap: Vec<Option<u32>> = list
        .prims
        .iter()
        .map(|p| {
            let points = p.start.checked_add(p.len).and_then(|end| {
                let buffer = if matches!(p.kind, Kind::Tris | Kind::Mask) { &list.tris } else { &list.verts };
                buffer.get(p.start as usize..end as usize)
            });
            let bounds = match points {
                Some(points) if points.iter().all(|point| point.is_finite()) => Bounds2::from_points(points.iter().copied()),
                _ => Bounds2::EMPTY,
            };
            let keep = matches!(p.kind, Kind::Infinite { .. }) || !p.lw.is_finite() || intersects(bounds);
            if keep {
                let index = next;
                next = next.saturating_add(1);
                Some(index)
            } else {
                None
            }
        })
        .collect();
    let mut indices = remap.iter();
    list.prims.retain(|_| indices.next().is_some_and(Option::is_some));
    let mut indices = remap.iter();
    list.prim_layers.retain(|_| indices.next().is_some_and(Option::is_some));
    let mut indices = remap.iter();
    list.pens.retain(|_| indices.next().is_some_and(Option::is_some));
    list.texts.retain_mut(|text| match remap.get(text.prim as usize) {
        Some(Some(index)) => {
            text.prim = *index;
            true
        }
        Some(None) => false,
        None => intersects(text.bounds),
    });
}

/// Source and destination origins for the shared model-space PDF/PNG placement behavior.
pub(crate) fn placement(space: &Space, bounds: &Bounds2, fit: bool, sheet: &Sheet, o: &PdfOptions) -> (Vec2, Vec2) {
    if matches!(space, Space::Model) {
        let from = if bounds.is_empty() {
            Vec2::ZERO
        } else if o.center.unwrap_or(true) {
            bounds.center()
        } else {
            bounds.min
        };
        let to = if o.center.unwrap_or(true) {
            sheet.printable.center()
        } else {
            let offset = o.offset.unwrap_or([0.0, 0.0]);
            sheet.printable.min + Vec2::new(offset[0] / sheet.unit_mm, offset[1] / sheet.unit_mm)
        };
        (from, to)
    } else if fit {
        (if bounds.is_empty() { Vec2::ZERO } else { bounds.center() }, sheet.printable.center())
    } else {
        (Vec2::ZERO, Vec2::ZERO)
    }
}

/// The hyperlinks to plot: the options' own, then (with [`PdfOptions::hyperlinks`]) those the
/// drawing file gave its entities as `PE_URL` extended data, at most [`MAX_LINKS`] in all.
fn all_links(d: &Drawing, o: &PdfOptions) -> Vec<(Handle, String)> {
    let mut out: Vec<(Handle, String)> = o.links.iter().take(MAX_LINKS).cloned().collect();
    if !o.hyperlinks {
        return out;
    }
    let given: std::collections::HashSet<Handle> = out.iter().map(|(h, _)| *h).collect();
    let mut read: Vec<(Handle, String)> = std::iter::once(&d.model)
        .chain(d.layouts.iter().map(|l| &l.entities))
        .chain(d.blocks.values().map(|b| &b.entities))
        .flat_map(|s| s.iter())
        .filter(|e| !given.contains(&e.handle))
        .filter_map(|e| Some((e.handle, pe_url(&e.common.xdata)?)))
        .collect();
    read.sort_by_key(|(h, _)| *h);
    read.dedup_by_key(|(h, _)| *h);
    out.extend(read.into_iter().take(MAX_LINKS.saturating_sub(out.len())));
    out
}

/// The link target of `PE_URL` extended data: the first string (group 1000) after
/// `1001 PE_URL`, before the next application's data.
fn pe_url(xdata: &[cadcraft_doc::RawTag]) -> Option<String> {
    let start = xdata.iter().position(|t| t.code == 1001 && t.value.trim().eq_ignore_ascii_case("PE_URL"))?;
    let url = xdata.get(start + 1..)?.iter().take_while(|t| t.code != 1001).find(|t| t.code == 1000)?.value.trim();
    (!url.is_empty() && url.len() <= MAX_URL).then(|| url.to_string())
}

/// Link annotation rectangles: the area (in points) each linked entity covers on the sheet,
/// clipped to the plot area.
fn links(list: &cadcraft_render::DisplayList, map: &Map, clip_pt: &Bounds2, links: &[(Handle, String)], user_unit: f64) -> Vec<(Bounds2, String)> {
    if links.is_empty() {
        return Vec::new();
    }
    let mut areas: std::collections::HashMap<Handle, Bounds2> = std::collections::HashMap::new();
    for p in &list.prims {
        if matches!(p.kind, Kind::Infinite { .. }) || !links.iter().any(|(h, _)| *h == p.handle) {
            continue;
        }
        let b = areas.entry(p.handle).or_insert(Bounds2::EMPTY);
        for q in list.points(p) {
            let q = map.pt(*q);
            if q.is_finite() {
                b.add(q);
            }
        }
    }
    let mut out = Vec::new();
    for (h, url) in links {
        let Some(b) = areas.get(h).filter(|b| !b.is_empty()) else { continue };
        // A little slack so thin lines are easy to click.
        let b = b.expand(2.0 / user_unit);
        let r = Bounds2 { min: b.min.max(clip_pt.min), max: b.max.min(clip_pt.max) };
        if r.min.x < r.max.x && r.min.y < r.max.y {
            out.push((r, url.clone()));
        }
    }
    out
}

/// Paper units per drawing unit for a plot.
pub(crate) fn plot_scale(b: &Bounds2, sheet: &Sheet, fit: bool, scale: Option<f64>) -> f64 {
    if let Some(s) = scale.filter(|s| s.is_finite() && *s > 0.0)
        && !fit
    {
        return s;
    }
    if !fit || b.is_empty() {
        return 1.0;
    }
    let pw = sheet.printable.width();
    let ph = sheet.printable.height();
    let s = (pw / b.width().max(1e-12)).min(ph / b.height().max(1e-12));
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
}

/// A number for a content stream: short, finite, no exponent.
fn num(out: &mut String, v: f64) {
    let v = if v.is_finite() { v.clamp(-1.0e7, 1.0e7) } else { 0.0 };
    let mut s = format!("{v:.3}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    out.push_str(&s);
}

fn pt(out: &mut String, p: Vec2) {
    num(out, p.x);
    out.push(' ');
    num(out, p.y);
}

fn color(out: &mut String, c: Rgb, op: &str) {
    for v in [c.0, c.1, c.2] {
        num(out, f64::from(v) / 255.0);
        out.push(' ');
    }
    out.push_str(op);
    out.push('\n');
}

/// The page's marks; those of primitives on `layers` are marked as optional content
/// (`/OC /L<layer> BDC` … `EMC`).
///
/// Primitives in `texts` (filled glyph outlines) are written as text instead: the glyph in its
/// embedded font (`/F<font>`), placed by a text matrix.
///
/// Transparent primitives are drawn with a graphics state of their alpha (`/T<percent> gs`);
/// the transparencies used are returned for the page's `/ExtGState` (see [`ext_gstates`]). A
/// list without transparency writes no graphics states.
fn content_stream(
    list: &cadcraft_render::DisplayList,
    map: &Map,
    clip_pt: &Bounds2,
    layers: &[u32],
    texts: &std::collections::HashMap<u32, (usize, &cadcraft_render::PlotText)>,
    user_unit: f64,
) -> (String, Vec<u8>) {
    let marked: std::collections::HashSet<u32> = layers.iter().copied().collect();
    let white = Rgb(255, 255, 255);
    let mut c = String::new();
    c.push_str("q\n1 J 1 j\n");
    // Clip to the plot area.
    pt(&mut c, clip_pt.min);
    c.push(' ');
    num(&mut c, clip_pt.width());
    c.push(' ');
    num(&mut c, clip_pt.height());
    c.push_str(" re W n\n");
    let mut stroke: Option<Rgb> = None;
    let mut fill: Option<Rgb> = None;
    let mut width: Option<f64> = None;
    let mut pending = false;
    let flush = |c: &mut String, pending: &mut bool| {
        if *pending {
            c.push_str("S\n");
            *pending = false;
        }
    };
    // The PDF layer (optional content) the marks are in: `/OC /L<i> BDC` … `EMC`.
    let mut layer: Option<u32> = None;
    // Inside a text object (`BT` … `ET`), with this font set.
    let mut text_font: Option<Option<usize>> = None;
    // The transparency (percent) marks are drawn with, and those used on the page.
    let mut alpha: u8 = 0;
    let mut alphas: Vec<u8> = Vec::new();
    for (i, p) in list.prims.iter().enumerate() {
        let want = list.prim_layers.get(i).copied().filter(|l| marked.contains(l));
        let text = u32::try_from(i).ok().and_then(|i| texts.get(&i));
        if text_font.is_some() && (text.is_none() || want != layer) {
            c.push_str("ET\n");
            text_font = None;
        }
        if want != layer {
            flush(&mut c, &mut pending);
            if layer.is_some() {
                c.push_str("EMC\n");
            }
            if let Some(l) = want {
                let _ = writeln!(c, "/OC /L{l} BDC");
            }
            layer = want;
        }
        let rgb = p.display_rgb(white);
        let raw = list.points(p);
        if raw.iter().any(|q| !q.is_finite()) {
            continue;
        }
        // A wipeout's mask always hides what is under it.
        let t = if p.kind == Kind::Mask { 0 } else { p.transparency.min(cadcraft_render::MAX_TRANSPARENCY) };
        if t != alpha {
            flush(&mut c, &mut pending);
            let _ = writeln!(c, "/T{t} gs");
            if !alphas.contains(&t) {
                alphas.push(t);
            }
            alpha = t;
        }
        let w = f64::from(p.lw) * PT_PER_MM;
        let w = if w.is_finite() && w > 0.0 { w.min(100.0) / user_unit } else { 0.0 };
        match p.kind {
            Kind::Polyline | Kind::Infinite { .. } => {
                let pts: Vec<Vec2> = match p.kind {
                    Kind::Infinite { ray } => match (raw.first(), raw.get(1)) {
                        (Some(base), Some(dir)) => {
                            let a = map.pt(*base);
                            let dir_pt = map.pt(*base + *dir) - a;
                            match clip::clip_infinite(a, dir_pt, ray, clip_pt) {
                                Some((x, y)) => vec![x, y],
                                None => continue,
                            }
                        }
                        _ => continue,
                    },
                    _ => raw.iter().map(|q| map.pt(*q)).collect(),
                };
                if pts.len() < 2 {
                    continue;
                }
                // A transparent primitive is stroked on its own (one path is painted once), so
                // where primitives overlap they build up, as in PNG plots and on screen. Block
                // contents and viewport contents share their reference's handle, so this goes
                // by primitive, not by object.
                if t > 0 {
                    flush(&mut c, &mut pending);
                }
                if stroke != Some(rgb) || width != Some(w) {
                    flush(&mut c, &mut pending);
                    if stroke != Some(rgb) {
                        color(&mut c, rgb, "RG");
                        stroke = Some(rgb);
                    }
                    if width != Some(w) {
                        num(&mut c, w);
                        c.push_str(" w\n");
                        width = Some(w);
                    }
                }
                for (i, q) in pts.iter().enumerate() {
                    pt(&mut c, *q);
                    c.push_str(if i == 0 { " m\n" } else { " l\n" });
                }
                pending = true;
            }
            Kind::Tris | Kind::Mask => {
                flush(&mut c, &mut pending);
                // A wipeout's mask is paper white.
                let rgb = if p.kind == Kind::Mask { white } else { rgb };
                if fill != Some(rgb) {
                    color(&mut c, rgb, "rg");
                    fill = Some(rgb);
                }
                if let Some((font, t)) = text {
                    glyph(&mut c, &mut text_font, *font, t, map);
                    continue;
                }
                let mut any = false;
                for t in raw.as_chunks::<3>().0 {
                    for (i, q) in t.iter().enumerate() {
                        pt(&mut c, map.pt(*q));
                        c.push_str(if i == 0 { " m\n" } else { " l\n" });
                    }
                    c.push_str("h\n");
                    any = true;
                }
                if any {
                    c.push_str("f\n");
                }
            }
            Kind::Point => {
                let Some(q) = raw.first() else { continue };
                flush(&mut c, &mut pending);
                if fill != Some(rgb) {
                    color(&mut c, rgb, "rg");
                    fill = Some(rgb);
                }
                let half = w.max(0.5 / user_unit) / 2.0;
                pt(&mut c, map.pt(*q) - Vec2::new(half, half));
                c.push(' ');
                num(&mut c, half * 2.0);
                c.push(' ');
                num(&mut c, half * 2.0);
                c.push_str(" re f\n");
            }
        }
    }
    flush(&mut c, &mut pending);
    if text_font.is_some() {
        c.push_str("ET\n");
    }
    if layer.is_some() {
        c.push_str("EMC\n");
    }
    c.push_str("Q\n");
    (c, alphas)
}

/// The page resources' graphics states for transparencies `alphas` (percent): `/T<percent>`
/// sets the stroke and fill alpha (`/CA`, `/ca`). Empty when there are none.
fn ext_gstates(alphas: &[u8]) -> String {
    if alphas.is_empty() {
        return String::new();
    }
    let mut out = String::from(" /ExtGState <<");
    for t in alphas {
        let mut a = String::new();
        num(&mut a, f64::from(100 - u32::from((*t).min(cadcraft_render::MAX_TRANSPARENCY))) / 100.0);
        let _ = write!(out, " /T{t} << /Type /ExtGState /CA {a} /ca {a} >>");
    }
    out.push_str(" >>");
    out
}

/// Write one glyph as text: opens a text object and sets the font when needed, then places the
/// glyph with a text matrix (font size 1: the matrix axes are one em).
fn glyph(c: &mut String, text_font: &mut Option<Option<usize>>, font: usize, t: &cadcraft_render::PlotText, map: &Map) {
    if text_font.is_none() {
        c.push_str("BT\n");
        *text_font = Some(None);
    }
    if *text_font != Some(Some(font)) {
        let _ = writeln!(c, "/F{font} 1 Tf");
        *text_font = Some(Some(font));
    }
    let s = map.s * map.k;
    let g = &t.glyph;
    for v in [g.x.x * s, g.x.y * s, g.y.x * s, g.y.y * s] {
        num(c, v);
        c.push(' ');
    }
    pt(c, map.pt(g.origin));
    let _ = writeln!(c, " Tm <{:04X}> Tj", g.gid);
}

/// A PDF text string: a literal with `(`, `)` and `\` escaped when it is printable ASCII,
/// else UTF-16BE with a byte order mark (hex). At most 1000 characters.
fn pdf_string(s: &str) -> String {
    let s: String = s.chars().take(1000).collect();
    if s.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        let mut out = String::from("(");
        for ch in s.chars() {
            if matches!(ch, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push(')');
        return out;
    }
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        let _ = write!(out, "{u:04X}");
    }
    out.push('>');
    out
}

/// A link target as a PDF URI string: 7-bit ASCII (other bytes percent-encoded), `(`, `)` and
/// `\` escaped. `None` for script and data URLs, which a plot never carries.
fn uri_string(url: &str) -> Option<String> {
    let url = url.trim();
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    if url.is_empty() || url.len() > MAX_URL || matches!(scheme.as_str(), "javascript" | "vbscript" | "data") {
        return None;
    }
    let mut out = String::from("(");
    for b in url.bytes() {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x21..=0x7E => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out.push(')');
    Some(out)
}

/// A stream object body, Flate-compressed when asked.
fn stream(data: &[u8], compress: bool, extra: &str) -> Vec<u8> {
    let (data, filter) = if compress { (miniz_oxide::deflate::compress_to_vec_zlib(data, 6), " /Filter /FlateDecode") } else { (data.to_vec(), "") };
    let mut out = format!("<< /Length {}{filter}{extra} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// PDF objects being written: bodies by object number - 1.
#[derive(Default)]
struct Objects {
    bodies: Vec<Vec<u8>>,
    bytes: usize,
}

impl Objects {
    /// Reserve an object number (its body is set later).
    fn reserve(&mut self) -> usize {
        self.bodies.push(Vec::new());
        self.bodies.len()
    }
    fn set(&mut self, n: usize, body: impl Into<Vec<u8>>) {
        if let Some(b) = n.checked_sub(1).and_then(|i| self.bodies.get_mut(i)) {
            let body = body.into();
            self.bytes = self.bytes.saturating_sub(b.len()).saturating_add(body.len());
            *b = body;
        }
    }
    fn add(&mut self, body: impl Into<Vec<u8>>) -> usize {
        let n = self.reserve();
        self.set(n, body);
        n
    }
}

/// `n 0 R` references, space-separated.
fn refs(ids: impl IntoIterator<Item = usize>) -> String {
    ids.into_iter().map(|i| format!("{i} 0 R")).collect::<Vec<_>>().join(" ")
}

fn check_pdf_size(bytes: usize, page: usize) -> Result<()> {
    if bytes > MAX_PDF_BYTES {
        return Err(IoError::Format(format!("the PDF would exceed 512 MiB at page {page}; publish it in parts (split)")));
    }
    Ok(())
}

fn seam_marks(c: &mut String, page: &SheetPage) {
    let Some(edges) = page.seams.filter(|edges| edges.iter().any(|edge| *edge)) else { return };
    let k = PT_PER_MM / page.user_unit;
    let overlap = page.seam_overlap_mm * k;
    let bounds = page.clip_pt;
    let x0 = bounds.min.x;
    let y0 = bounds.min.y;
    let x1 = bounds.max.x;
    let y1 = bounds.max.y;
    let [left, bottom, right, top] = edges;
    let line = |c: &mut String, a: Vec2, b: Vec2| {
        pt(c, a);
        c.push_str(" m ");
        pt(c, b);
        c.push_str(" l S\n");
    };
    c.push_str("q\n0 G\n");
    num(c, 0.25 * k);
    c.push_str(" w\n");
    for (enabled, a, b, vertical) in [
        (left, Vec2::new((x0 + overlap).min(x1), y0), Vec2::new((x0 + overlap).min(x1), y1), true),
        (bottom, Vec2::new(x0, (y0 + overlap).min(y1)), Vec2::new(x1, (y0 + overlap).min(y1)), false),
        (right, Vec2::new((x1 - overlap).max(x0), y0), Vec2::new((x1 - overlap).max(x0), y1), true),
        (top, Vec2::new(x0, (y1 - overlap).max(y0)), Vec2::new(x1, (y1 - overlap).max(y0)), false),
    ] {
        if !enabled {
            continue;
        }
        c.push('[');
        num(c, 3.0 * k);
        c.push(' ');
        num(c, 2.0 * k);
        c.push_str("] 0 d\n");
        line(c, a, b);
        c.push_str("[] 0 d\n");
        for end in [a, b] {
            let (from, to) = if vertical {
                let start = (end.x - 5.0 * k).max(x0).min((x1 - 10.0 * k).max(x0));
                (Vec2::new(start, end.y), Vec2::new((start + 10.0 * k).min(x1), end.y))
            } else {
                let start = (end.y - 5.0 * k).max(y0).min((y1 - 10.0 * k).max(y0));
                (Vec2::new(end.x, start), Vec2::new(end.x, (start + 10.0 * k).min(y1)))
            };
            line(c, from, to);
        }
    }
    c.push_str("Q\n");
}

fn assemble(d: &Drawing, pages: impl ExactSizeIterator<Item = Result<SheetPage>>, mut fonts: fonts::Fonts, o: &PdfOptions) -> Result<Vec<u8>> {
    let page_count = pages.len();
    let mut title = o.title.clone();
    let mut labels = Vec::with_capacity(page_count);
    let mut large_page = false;
    let mut objs = Objects::default();
    let catalog = objs.reserve();
    let tree = objs.reserve();
    // Optional content groups shared by all pages, by layer index: (layer, object).
    let mut ocg_ids: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut ocgs: Vec<(u32, usize)> = Vec::new();
    let mut kids = Vec::new();
    // Page dictionaries are completed once the fonts are written: (page, its fonts, the rest).
    let mut page_dicts: Vec<(usize, Vec<usize>, String, String, String)> = Vec::new();
    for (index, plotted) in pages.enumerate() {
        let p = plotted?;
        if index == 0 && title.is_empty() {
            title.clone_from(&p.label);
        }
        if page_count == 1 && !o.text_as_geometry {
            fonts = fonts::Fonts::collect(fonts::glyphs(&p.list));
        }
        let page = objs.reserve();
        let texts = fonts.texts(&p.list);
        let (mut marks, alphas) = content_stream(&p.list, &p.map, &p.clip_pt, &p.layers, &texts, p.user_unit);
        seam_marks(&mut marks, &p);
        let content = objs.add(stream(marks.as_bytes(), o.compress, ""));
        drop(marks);
        let mut used: Vec<usize> = texts.values().map(|(f, _)| *f).collect();
        used.sort_unstable();
        used.dedup();
        let mut props = String::new();
        for l in &p.layers {
            let Some(layer) = d.layers.get(*l as usize) else { continue };
            let id = match ocg_ids.get(l) {
                Some(id) => *id,
                None => {
                    let id = objs.add(format!("<< /Type /OCG /Name {} >>", pdf_string(&layer.name)));
                    ocg_ids.insert(*l, id);
                    ocgs.push((*l, id));
                    id
                }
            };
            let _ = write!(props, " /L{l} {id} 0 R");
        }
        let mut annots = Vec::new();
        for (r, url) in &p.links {
            let Some(uri) = uri_string(url) else { continue };
            let mut rect = String::new();
            pt(&mut rect, r.min);
            rect.push(' ');
            pt(&mut rect, r.max);
            annots.push(objs.add(format!("<< /Type /Annot /Subtype /Link /Rect [{rect}] /Border [0 0 0] /A << /S /URI /URI {uri} >> >>")));
        }
        let mut mb = String::new();
        pt(&mut mb, p.media);
        let user_unit = if p.user_unit > 1.0 {
            large_page = true;
            format!(" /UserUnit {}", p.user_unit)
        } else {
            String::new()
        };
        let annots = if annots.is_empty() { String::new() } else { format!(" /Annots [{}]", refs(annots)) };
        page_dicts.push((page, used, props, ext_gstates(&alphas), format!("/MediaBox [0 0 {mb}]{user_unit} /Contents {content} 0 R{annots}")));
        kids.push(page);
        labels.push(format!("{index} << /P {} >>", pdf_string(&p.label)));
        check_pdf_size(objs.bytes, index + 1)?;
    }
    let font_ids = if fonts.is_empty() { Vec::new() } else { fonts.write(&mut objs, o.compress) };
    check_pdf_size(objs.bytes, page_count)?;
    drop(fonts);
    for (page, used, props, gstates, rest) in page_dicts {
        let mut res = String::new();
        if !props.is_empty() {
            let _ = write!(res, " /Properties <<{props} >>");
        }
        res.push_str(&gstates);
        let f: Vec<String> = used.iter().filter_map(|i| Some(format!("/F{i} {} 0 R", font_ids.get(*i)?))).collect();
        if !f.is_empty() {
            let _ = write!(res, " /Font << {} >>", f.join(" "));
        }
        objs.set(page, format!("<< /Type /Page /Parent {tree} 0 R /Resources <<{res} >> {rest} >>"));
    }
    objs.set(tree, format!("<< /Type /Pages /Kids [{}] /Count {} >>", refs(kids.iter().copied()), kids.len()));
    let mut cat = format!("<< /Type /Catalog /Pages {tree} 0 R");
    if !ocgs.is_empty() {
        // Optional content needs PDF 1.5; the catalog's version overrides the header's.
        let off: Vec<usize> = ocgs.iter().filter(|(l, _)| d.layers.get(*l as usize).is_some_and(|l| !l.on)).map(|(_, id)| *id).collect();
        let all = refs(ocgs.iter().map(|(_, id)| *id));
        let version = if large_page { "1.6" } else { "1.5" };
        let _ = write!(cat, " /Version /{version} /OCProperties << /OCGs [{all}] /D << /Name (Layers) /Order [{all}] /OFF [{}] >> >>", refs(off));
        if page_count == 1 {
            cat.push_str(" /PageMode /UseOC");
        }
    }
    if large_page && ocgs.is_empty() {
        cat.push_str(" /Version /1.6");
    }
    let _ = write!(cat, " /PageLabels << /Nums [{}] >> >>", labels.join(" "));
    objs.set(catalog, cat);
    let info = objs.add(format!("<< /Producer (CADCraft) /Creator (CADCraft) /Title {} >>", pdf_string(&title)));
    const HEADER: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";
    let mut size = HEADER.len();
    let mut offsets = Vec::with_capacity(objs.bodies.len());
    for (i, body) in objs.bodies.iter().enumerate() {
        offsets.push(size);
        size = size.saturating_add(format!("{} 0 obj\n", i + 1).len()).saturating_add(body.len()).saturating_add(b"\nendobj\n".len());
        check_pdf_size(size, page_count)?;
    }
    let xref = size;
    let mut x = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for offset in &offsets {
        let _ = writeln!(x, "{offset:010} 00000 n ");
    }
    let _ = write!(x, "trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1);
    size = size.saturating_add(x.len());
    check_pdf_size(size, page_count)?;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(HEADER);
    for (i, body) in objs.bodies.iter().enumerate() {
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    out.extend_from_slice(x.as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_render::pen::Pen;
    use cadcraft_render::{DPrim, DisplayList, PlotText};

    fn prim(handle: u64, kind: Kind, start: u32, len: u32) -> DPrim {
        DPrim { handle: Handle(handle), color: Rgb(0, 0, 0), aci7: false, lw: 0.0, transparency: 0, kind, start, len }
    }

    fn text(prim: u32, bounds: Bounds2) -> PlotText {
        PlotText {
            prim,
            verts: 3,
            bounds,
            glyph: cadcraft_fonts::TextGlyph {
                face: cadcraft_fonts::ttf::Face { bytes: Arc::new(Vec::new()), index: 0 },
                gid: 1,
                ch: 'A',
                outline: 0,
                origin: bounds.min,
                x: Vec2::X,
                y: Vec2::Y,
            },
        }
    }

    #[test]
    fn window_culling_remaps_metadata_and_keeps_unknown_bounds() {
        let near = Bounds2::new(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let far = Bounds2::new(Vec2::new(1000.0, 1000.0), Vec2::new(1010.0, 1010.0));
        let pens: Vec<Pen> = [cadcraft_doc::EndStyle::Butt, cadcraft_doc::EndStyle::Square, cadcraft_doc::EndStyle::Round]
            .into_iter()
            .cycle()
            .take(9)
            .map(|end| Pen { end, ..Pen::default() })
            .collect();
        let mut list = DisplayList {
            prims: vec![
                prim(0, Kind::Polyline, 0, 2),
                prim(1, Kind::Polyline, 2, 2),
                prim(2, Kind::Tris, 0, 3),
                prim(3, Kind::Mask, 3, 3),
                prim(4, Kind::Polyline, 4, 2),
                prim(5, Kind::Infinite { ray: false }, 6, 2),
                prim(6, Kind::Polyline, u32::MAX, 2),
                prim(7, Kind::Point, 0, 0),
                DPrim { lw: f32::NAN, ..prim(8, Kind::Polyline, 0, 2) },
            ],
            verts: vec![far.min, far.max, near.min, near.max, Vec2::new(f64::NAN, 0.0), far.min, far.min, Vec2::X],
            tris: vec![near.min, Vec2::new(10.0, 0.0), near.max, far.min, Vec2::new(1010.0, 1000.0), far.max],
            prim_layers: (0..9).collect(),
            pens: pens.clone(),
            texts: vec![text(2, near), text(3, far), text(PlotText::NONE, near), text(PlotText::CLIPPED, far), text(PlotText::NONE, Bounds2::EMPTY)],
            ..DisplayList::default()
        };
        cull_window(&mut list, near, 1.0);
        assert_eq!(list.prims.iter().map(|p| p.handle.0).collect::<Vec<_>>(), [1, 2, 4, 5, 6, 7, 8]);
        assert_eq!(list.prim_layers, [1, 2, 4, 5, 6, 7, 8]);
        assert_eq!(list.pens, [pens[1], pens[2], pens[4], pens[5], pens[6], pens[7], pens[8]]);
        assert_eq!(list.texts.iter().map(|t| t.prim).collect::<Vec<_>>(), [1, PlotText::NONE, PlotText::NONE]);
        assert_eq!(list.texts.first().map(|t| t.bounds), Some(near));
    }

    #[test]
    fn window_culling_converts_lineweight_margin_to_drawing_units() {
        for mm_per_unit in [0.01, 1.0, 25.4] {
            let window = Bounds2::new(Vec2::ZERO, Vec2::new(10.0, 10.0));
            let mut list = DisplayList {
                prims: vec![DPrim { lw: 2.0, ..prim(1, Kind::Polyline, 0, 2) }, DPrim { lw: 2.0, ..prim(2, Kind::Polyline, 2, 2) }],
                verts: vec![
                    Vec2::new(-0.5 / mm_per_unit, 2.0),
                    Vec2::new(-0.5 / mm_per_unit, 8.0),
                    Vec2::new(-100.0 / mm_per_unit, 2.0),
                    Vec2::new(-100.0 / mm_per_unit, 8.0),
                ],
                ..DisplayList::default()
            };
            cull_window(&mut list, window, mm_per_unit);
            assert_eq!(list.prims.iter().map(|p| p.handle).collect::<Vec<_>>(), [Handle(1)]);
        }
    }

    #[test]
    fn window_culling_keeps_layouts_and_sheet_links_consistent() {
        use cadcraft_doc::{Common, EntityKind, Layer, Line};

        let mut d = Drawing::new_metric();
        d.layers.push(Layer::new("Inside"));
        d.layers.push(Layer::new("Outside"));
        let mut handles = Vec::new();
        for space in [Space::Model, Space::Paper("Layout1".into())] {
            for (layer, at) in [("Outside", Vec2::new(1000.0, 1000.0)), ("Inside", Vec2::new(2.0, 2.0))] {
                let line = Line { a: at.to3(0.0), b: (at + Vec2::new(1.0, 1.0)).to3(0.0) };
                let handle = d.add(&space, Common { layer: layer.into(), ..Common::default() }, EntityKind::Line(line)).unwrap();
                handles.push((handle, format!("https://example.com/{layer}")));
            }
        }
        let frame = PlotFrame { center: Vec2::new(5.0, 5.0), size: Vec2::new(10.0, 10.0), rotation: 0.0 };
        for frame in [None, Some(frame)] {
            let options =
                PdfOptions { window: Some(Bounds2::new(Vec2::ZERO, Vec2::new(10.0, 10.0))), frame, max_prims: Some(1), ..PdfOptions::default() };
            let page = sheet(&d, &Space::Model, &options, &handles).unwrap();
            assert_eq!(page.list.prims.len(), 1);
            assert_eq!(page.layers, [d.layers.iter().position(|layer| layer.name == "Inside").unwrap() as u32]);
            assert_eq!(page.links.len(), 1);
            assert_eq!(page.links.first().map(|(_, url)| url.as_str()), Some("https://example.com/Inside"));
            assert!(sheet(&d, &Space::Paper("Layout1".into()), &options, &handles).is_err());
            let options = PdfOptions { max_prims: None, ..options };
            assert_eq!(sheet(&d, &Space::Paper("Layout1".into()), &options, &handles).unwrap().list.prims.len(), 2);
        }
    }
}
