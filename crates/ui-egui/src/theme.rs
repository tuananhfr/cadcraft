//! Design tokens. Colours were measured from the reference look (dark slate chrome, near-black
//! navy model space) and are CADCraft's own values; every widget reads them from here.
//!
//! Two palettes: [`Tokens::DARK`] (UbotBuild's navy look) and [`Tokens::LIGHT`]. The interface theme
//! changes the chrome (title and tool bar, menus, palettes, tabs, command line, status bar,
//! dialogs); the drawing area keeps its model-space colours in both, as drawing colours are not
//! part of the interface theme. [`Tokens::get`] returns the palette of the theme in use, which
//! [`CadApp::logic`](crate::CadApp::logic) resolves once per frame from the user's [`ThemePref`].

use std::cell::Cell;

use egui::{Color32, FontFamily, FontId, Theme, Visuals};
use serde::{Deserialize, Serialize};

use crate::wsprofile::schema::Density;

/// Control sizes and spacing of one interface density, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub interact_height: f32,
    pub button_padding: egui::Vec2,
    pub item_spacing: egui::Vec2,
    pub ribbon_button: f32,
    pub toolbar_button: f32,
    pub tab_height: f32,
}

impl Density {
    pub fn metrics(self) -> Metrics {
        match self {
            Density::Compact => Metrics {
                interact_height: 18.0,
                button_padding: egui::vec2(4.0, 2.0),
                item_spacing: egui::vec2(4.0, 2.0),
                ribbon_button: 20.0,
                toolbar_button: 20.0,
                tab_height: 24.0,
            },
            Density::Normal => Metrics {
                interact_height: 20.0,
                button_padding: egui::vec2(6.0, 3.0),
                item_spacing: egui::vec2(6.0, 4.0),
                ribbon_button: 24.0,
                toolbar_button: 24.0,
                tab_height: 28.0,
            },
            Density::Comfortable => Metrics {
                interact_height: 32.0,
                button_padding: egui::vec2(8.0, 6.0),
                item_spacing: egui::vec2(8.0, 6.0),
                ribbon_button: 32.0,
                toolbar_button: 32.0,
                tab_height: 36.0,
            },
        }
    }
}

const DENSITY_ID: &str = "cc_density";

/// The density [`apply_density`] last installed in `ctx`.
pub fn density(ctx: &egui::Context) -> Density {
    ctx.data(|data| data.get_temp(egui::Id::new(DENSITY_ID))).unwrap_or_default()
}

/// Installs `density`'s spacing in the styles of both themes.
pub fn apply_density(ctx: &egui::Context, density: Density) {
    let id = egui::Id::new(DENSITY_ID);
    if ctx.data(|data| data.get_temp::<Density>(id)) == Some(density) {
        return;
    }
    let metrics = density.metrics();
    ctx.data_mut(|data| data.insert_temp(id, density));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = metrics.item_spacing;
        style.spacing.button_padding = metrics.button_padding;
        style.spacing.interact_size.y = metrics.interact_height;
    });
}

/// WCAG 2.x contrast ratio (1 to 21) of two opaque sRGB colours.
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let luminance = |color: Color32| {
        let linear = |channel: u8| {
            let srgb = f32::from(channel) / 255.0;
            if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The interface theme the user chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    /// Follow the operating system's light/dark appearance, live.
    System,
    Light,
    /// The default: CADCraft's original look.
    #[default]
    Dark,
}

/// The theme `System` uses when the OS (or browser) reports no appearance.
pub const SYSTEM_FALLBACK: Theme = Theme::Dark;

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<ThemePref> {
        ThemePref::ALL.into_iter().find(|t| t.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// The theme to show, given what the OS reports (`ctx.system_theme()`).
    pub fn resolve(self, system: Option<Theme>) -> Theme {
        match self {
            ThemePref::System => system.unwrap_or(SYSTEM_FALLBACK),
            ThemePref::Light => Theme::Light,
            ThemePref::Dark => Theme::Dark,
        }
    }
}

thread_local! {
    /// The theme in use on the UI thread (set by [`set_active`] each frame).
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::Dark) };
}

/// The theme [`Tokens::get`] returns colours for.
pub fn active() -> Theme {
    ACTIVE.with(Cell::get)
}

pub fn set_active(theme: Theme) {
    ACTIVE.with(|a| a.set(theme));
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub chrome: Color32,
    pub chrome_dark: Color32,
    /// The title bar above the menus and ribbon.
    pub title_bar: Color32,
    /// The ribbon's tab row, a step darker than its panels.
    pub ribbon_tabs: Color32,
    /// Palette headers and the ribbon's group titles.
    pub header: Color32,
    pub panel: Color32,
    pub tab_active: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub border: Color32,
    pub canvas: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub icon: Color32,
    pub icon_accent: Color32,
    pub icon_point: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub grip: Color32,
    pub grip_hot: Color32,
    /// Grips of dynamic block references (AutoCAD's GRIPDYNCOLOR).
    pub grip_dyn: Color32,
    pub snap: Color32,
    /// COMPARE: objects only in this drawing, only in the other one, and the areas of change.
    pub compare_this: Color32,
    pub compare_other: Color32,
    pub compare_area: Color32,
    pub window_stroke: Color32,
    pub crossing_stroke: Color32,
    pub cmd_bg: Color32,
    pub toggle_on: Color32,
    /// Text fields and other inset (extreme) backgrounds.
    pub field: Color32,
    /// Selected text and selected list rows.
    pub select_bg: Color32,
    /// Lines and labels drawn straight onto the drawing area (UCS icon, viewport label, rubber
    /// bands): they follow the model-space background, not the interface theme.
    pub canvas_ink: Color32,
    /// The command-line history lines behind the input bar.
    pub cmd_history: Color32,
    pub cmd_border: Color32,
    /// Command-line keywords (`[Undo/Close]`) and their hover background.
    pub cmd_keyword: Color32,
    pub cmd_keyword_hover: Color32,
    /// The command-line AutoComplete list: background and the highlighted first row.
    pub list_bg: Color32,
    pub list_row: Color32,
    /// Transient warnings (status bar messages).
    pub warn: Color32,
    /// The drawing-area crosshair cursor and its pick box.
    pub crosshair: Color32,
    /// The sheet of paper on a layout tab.
    pub paper: Color32,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        chrome: Color32::from_rgb(0x2b, 0x3a, 0x4d),
        chrome_dark: Color32::from_rgb(0x20, 0x2c, 0x3b),
        title_bar: Color32::from_rgb(0x10, 0x1c, 0x2f),
        ribbon_tabs: Color32::from_rgb(0x25, 0x32, 0x44),
        header: Color32::from_rgb(0x30, 0x3e, 0x50),
        panel: Color32::from_rgb(0x29, 0x38, 0x4a),
        tab_active: Color32::from_rgb(0x28, 0x70, 0xbe),
        control: Color32::from_rgb(0x34, 0x46, 0x5b),
        control_hover: Color32::from_rgb(0x3f, 0x54, 0x6d),
        border: Color32::from_rgb(0x17, 0x23, 0x34),
        canvas: Color32::from_rgb(0x0c, 0x15, 0x21),
        grid_minor: Color32::from_rgb(0x13, 0x1e, 0x2c),
        grid_major: Color32::from_rgb(0x1a, 0x28, 0x39),
        axis_x: Color32::from_rgb(0x4e, 0x22, 0x28),
        axis_y: Color32::from_rgb(0x1f, 0x44, 0x2e),
        text: Color32::from_rgb(0xca, 0xd3, 0xdf),
        text_dim: Color32::from_rgb(0xae, 0xbc, 0xcd),
        text_faint: Color32::from_rgb(0xa3, 0xb2, 0xc4),
        icon: Color32::from_rgb(0xb5, 0xd3, 0xec),
        icon_accent: Color32::from_rgb(0x4f, 0xa3, 0xf7),
        icon_point: Color32::from_rgb(0xf0, 0x7a, 0x3a),
        accent: Color32::from_rgb(0x4a, 0x97, 0xff),
        selection: Color32::from_rgb(0x4a, 0x8f, 0xff),
        hover: Color32::from_rgb(0x9c, 0xc6, 0xff),
        grip: Color32::from_rgb(0x2f, 0x6b, 0xff),
        grip_hot: Color32::from_rgb(0xff, 0x3b, 0x3b),
        grip_dyn: Color32::from_rgb(0x26, 0xc6, 0xe8),
        snap: Color32::from_rgb(0x2f, 0xd0, 0x62),
        compare_this: Color32::from_rgb(0x4c, 0xd9, 0x7b),
        compare_other: Color32::from_rgb(0xff, 0x5c, 0x5c),
        compare_area: Color32::from_rgb(0xff, 0xb4, 0x3c),
        window_stroke: Color32::from_rgb(0x6f, 0xa8, 0xff),
        crossing_stroke: Color32::from_rgb(0x7f, 0xe0, 0x7f),
        cmd_bg: Color32::from_rgba_premultiplied(0x1a, 0x28, 0x38, 0xf0),
        toggle_on: Color32::from_rgb(0x3d, 0x8b, 0xfd),
        field: Color32::from_rgb(0x22, 0x32, 0x46),
        select_bg: Color32::from_rgb(0x28, 0x70, 0xbe),
        canvas_ink: Color32::from_rgb(0xa9, 0xb6, 0xc6),
        cmd_history: Color32::from_rgba_unmultiplied_const(0x1a, 0x28, 0x38, 200),
        cmd_border: Color32::from_rgb(0x46, 0x53, 0x68),
        cmd_keyword: Color32::from_rgb(0x8f, 0xc1, 0xff),
        cmd_keyword_hover: Color32::from_rgb(0x2f, 0x5e, 0xa8),
        list_bg: Color32::from_rgb(0x1f, 0x2c, 0x3c),
        list_row: Color32::from_rgb(0x2c, 0x3e, 0x54),
        warn: Color32::from_rgb(0xff, 0xd0, 0x80),
        crosshair: Color32::from_rgb(0xe8, 0xe8, 0xe8),
        paper: Color32::WHITE,
    };

    /// Light grey chrome with dark text. The drawing-area colours (canvas, grid, axes, selection,
    /// grips, snap markers, selection windows) are the same as [`Tokens::DARK`]'s.
    pub const LIGHT: Tokens = Tokens {
        chrome: Color32::from_rgb(0xe4, 0xe7, 0xeb),
        chrome_dark: Color32::from_rgb(0xd5, 0xd9, 0xdf),
        title_bar: Color32::from_rgb(0xd0, 0xd6, 0xde),
        ribbon_tabs: Color32::from_rgb(0xd9, 0xde, 0xe4),
        header: Color32::from_rgb(0xdb, 0xe0, 0xe6),
        panel: Color32::from_rgb(0xee, 0xf0, 0xf3),
        tab_active: Color32::from_rgb(0xc8, 0xd3, 0xe2),
        control: Color32::from_rgb(0xdc, 0xe0, 0xe6),
        control_hover: Color32::from_rgb(0xcb, 0xd2, 0xdc),
        border: Color32::from_rgb(0xb3, 0xba, 0xc4),
        text: Color32::from_rgb(0x1d, 0x23, 0x2b),
        text_dim: Color32::from_rgb(0x47, 0x50, 0x5c),
        text_faint: Color32::from_rgb(0x57, 0x5f, 0x6b),
        icon: Color32::from_rgb(0x34, 0x3c, 0x48),
        icon_accent: Color32::from_rgb(0x1b, 0x6f, 0xd6),
        icon_point: Color32::from_rgb(0xd3, 0x5a, 0x1c),
        accent: Color32::from_rgb(0x28, 0x75, 0xef),
        cmd_bg: Color32::from_rgba_unmultiplied_const(0xf9, 0xfa, 0xfb, 0xf0),
        toggle_on: Color32::from_rgb(0x2f, 0x7c, 0xf6),
        field: Color32::from_rgb(0xff, 0xff, 0xff),
        select_bg: Color32::from_rgb(0xb6, 0xd3, 0xfb),
        cmd_history: Color32::from_rgba_unmultiplied_const(0xf0, 0xf2, 0xf5, 0xdc),
        cmd_border: Color32::from_rgb(0xa9, 0xb0, 0xba),
        cmd_keyword: Color32::from_rgb(0x15, 0x59, 0xc0),
        cmd_keyword_hover: Color32::from_rgb(0xcf, 0xe0, 0xfa),
        list_bg: Color32::from_rgb(0xfb, 0xfc, 0xfd),
        list_row: Color32::from_rgb(0xe3, 0xe8, 0xef),
        warn: Color32::from_rgb(0x8a, 0x53, 0x00),
        ..Tokens::DARK
    };

    /// The palette of `theme`.
    pub fn of(theme: Theme) -> Tokens {
        match theme {
            Theme::Dark => Tokens::DARK,
            Theme::Light => Tokens::LIGHT,
        }
    }

    /// The palette of the theme in use, with the user's drawing-area colours ([`set_canvas`]).
    pub fn get() -> Tokens {
        CANVAS.with(Cell::get).over(Tokens::of(active()))
    }

    /// These tokens with `bg` as the model-space background: the grid, axes, crosshair and the
    /// lines drawn on the drawing area are recomputed to stay visible on it.
    pub fn with_model_background(self, bg: Color32) -> Tokens {
        let light = luminance(bg) > 0.5;
        let ink = if light { Color32::BLACK } else { Color32::WHITE };
        Tokens {
            canvas: bg,
            grid_minor: mix(bg, ink, 0.05),
            grid_major: mix(bg, ink, 0.11),
            axis_x: mix(bg, Color32::from_rgb(0xd0, 0x30, 0x30), 0.3),
            axis_y: mix(bg, Color32::from_rgb(0x30, 0xb0, 0x40), 0.3),
            canvas_ink: if light { Color32::from_rgb(0x47, 0x50, 0x5c) } else { Tokens::DARK.canvas_ink },
            crosshair: if light { Color32::from_rgb(0x20, 0x20, 0x20) } else { Tokens::DARK.crosshair },
            ..self
        }
    }
}

/// The user's drawing-area preferences (Options ▸ Display and Drafting); `None` keeps the theme's
/// colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub model: Option<Color32>,
    pub paper: Option<Color32>,
    /// The side of an object snap marker, in points.
    pub snap_marker: f32,
    /// The colour selected objects are highlighted in.
    pub selection: Option<Color32>,
}

/// The default object snap marker size.
pub const SNAP_MARKER: f32 = 11.0;

impl Default for Canvas {
    fn default() -> Self {
        Canvas { model: None, paper: None, snap_marker: SNAP_MARKER, selection: None }
    }
}

impl Canvas {
    fn over(self, t: Tokens) -> Tokens {
        let t = match self.model {
            Some(bg) => t.with_model_background(bg),
            None => t,
        };
        Tokens { paper: self.paper.unwrap_or(t.paper), selection: self.selection.unwrap_or(t.selection), ..t }
    }
}

thread_local! {
    /// The drawing-area preferences in use on the UI thread (set by [`set_canvas`] each frame).
    static CANVAS: Cell<Canvas> = Cell::new(Canvas::default());
}

pub fn set_canvas(c: Canvas) {
    CANVAS.with(|v| v.set(c));
}

/// Half the side of an object snap marker, in points.
pub fn snap_marker_half() -> f32 {
    CANVAS.with(Cell::get).snap_marker / 2.0
}

/// Relative luminance of `c` (0 black … 1 white).
pub fn luminance(c: Color32) -> f32 {
    (0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())) / 255.0
}

/// `a` moved the fraction `f` towards `b`.
fn mix(a: Color32, b: Color32, f: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

pub fn small() -> FontId {
    FontId::new(12.0, FontFamily::Proportional)
}
pub fn body() -> FontId {
    FontId::new(13.0, FontFamily::Proportional)
}
pub fn mono() -> FontId {
    FontId::new(12.0, FontFamily::Monospace)
}

/// Load a system UI font at run time when one is installed (not bundled), else keep egui's (on
/// the web, the embedded Noto Sans); then CJK fonts after the Latin ones, so Latin text keeps its look, in every interface
/// language: file, layout and layer names can be Japanese or Chinese whatever the interface
/// shows (#513). In a Chinese interface (`language` "zh…") the Chinese font comes first, else a
/// Japanese one, so names take their own glyph forms; in an Arabic interface an installed
/// Arabic font follows. A font embedded from craft-fonts comes
/// last, for systems without any (and the web build, which has no system fonts). A Korean
/// interface adds a Hangul font only when the CJK font found doesn't cover Hangul
/// ([`korean_font`]).
pub fn install_fonts(ctx: &egui::Context, language: &str) {
    let mut fonts = egui::FontDefinitions::default();
    let mut added: Vec<(String, std::sync::Arc<egui::FontData>)> = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    let chinese = language.starts_with("zh");
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(bytes) = system_ui_font() {
            fonts.font_data.insert("system-ui".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes.as_ref().clone())));
            if let Some(f) = fonts.families.get_mut(&FontFamily::Proportional) {
                f.insert(0, "system-ui".into());
            }
            if language == "vi"
                && let Some(f) = fonts.families.get_mut(&FontFamily::Monospace)
            {
                f.push("system-ui".into());
            }
        }
        let mut paths: Vec<&std::path::Path> = Vec::new();
        if !chinese {
            paths.extend(japanese_font());
        }
        paths.extend(cjk_font());
        paths.dedup();
        added.extend(
            paths
                .into_iter()
                .filter_map(|p| read_capped(p, MAX_UI_FONT_BYTES))
                .enumerate()
                .map(|(i, bytes)| (format!("system-cjk-{i}"), std::sync::Arc::new(egui::FontData::from_owned(bytes)))),
        );
        // العربية: an installed Arabic face after the Latin ones (egui's bundled faces carry no
        // Arabic glyphs), for the Arabic interface.
        if language == "ar"
            && let Some(bytes) = arabic_font().and_then(|p| read_capped(p, MAX_UI_FONT_BYTES))
        {
            added.push(("system-arabic".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes))));
        }
    }
    // The web has no system fonts, and egui's own faces stop short of Vietnamese (U+1EA0…).
    #[cfg(target_arch = "wasm32")]
    if let Some(f) = cadcraft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans") {
        let name = format!("craft-{}", f.stem());
        fonts.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_static(f.bytes)));
        if let Some(p) = fonts.families.get_mut(&FontFamily::Proportional) {
            p.insert(0, name.clone());
        }
        if let Some(m) = fonts.families.get_mut(&FontFamily::Monospace) {
            m.push(name);
        }
    }
    let mut embedded: Vec<_> = cadcraft_fonts::CRAFT_FONTS
        .iter()
        .filter(|f| f.style == "Regular" && f.scripts.iter().any(|s| matches!(*s, "Jpan" | "Hans" | "Hant")))
        .collect();
    // A sans-serif face reads best in the interface.
    embedded.sort_by_key(|f| !f.family.contains("Gothic") && !f.family.contains("Sans"));
    added.extend(embedded.first().map(|f| (format!("craft-{}", f.stem()), std::sync::Arc::new(egui::FontData::from_static(f.bytes)))));
    // Korean: Noto Sans CJK and most CJK fonts above already cover Hangul; else a Hangul font.
    if language == "ko"
        && !cjk_covers_hangul()
        && let Some(face) = korean_font()
    {
        let mut data = egui::FontData::from_owned(face.bytes.as_ref().clone());
        data.index = face.index;
        added.push(("korean-ui".into(), std::sync::Arc::new(data)));
    }
    for (name, data) in added {
        fonts.font_data.insert(name.clone(), data);
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(f) = fonts.families.get_mut(&family) {
                f.push(name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(not(target_arch = "wasm32"))]
fn system_ui_font() -> Option<&'static std::sync::Arc<Vec<u8>>> {
    static FOUND: std::sync::OnceLock<Option<std::sync::Arc<Vec<u8>>>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let candidates: &[&str] = if cfg!(target_os = "macos") {
                &["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc", "/Library/Fonts/Arial.ttf"]
            } else if cfg!(windows) {
                &["C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
            } else {
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                    "/usr/share/fonts/TTF/DejaVuSans.ttf",
                    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                ]
            };
            candidates.iter().find_map(|path| read_capped(std::path::Path::new(path), MAX_UI_FONT_BYTES).map(std::sync::Arc::new))
        })
        .as_ref()
}

/// Check the face installed by `install_fonts`, so selecting Vietnamese cannot produce boxes.
pub fn vietnamese_ui_font() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            system_ui_font().and_then(|bytes| cadcraft_fonts::ttf::Face::new(bytes.clone(), 0)).is_some_and(|face| face.has_glyph('ạ'))
        }
        #[cfg(target_arch = "wasm32")]
        {
            cadcraft_fonts::CRAFT_FONTS
                .iter()
                .find(|font| font.family == "Noto Sans")
                .filter(|font| font.bytes.len() as u64 <= MAX_UI_FONT_BYTES)
                .and_then(|font| cadcraft_fonts::ttf::Face::new(std::sync::Arc::new(font.bytes.to_vec()), 0))
                .is_some_and(|face| face.has_glyph('ạ'))
        }
    })
}

/// Largest interface font file read or embedded face copied (CJK collections are 15–60 MB).
const MAX_UI_FONT_BYTES: u64 = 96 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
fn read_capped(path: &std::path::Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 || len > max {
        return None;
    }
    // Enforce the cap during the read too, even if the file grows after metadata was read.
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    (!bytes.is_empty() && bytes.len() as u64 <= max).then_some(bytes)
}

/// An installed font with Chinese characters, for the Chinese interface; fonts are never bundled
/// (only looked for where the OS or common packages put them). Checked once.
pub fn cjk_font() -> Option<&'static std::path::Path> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static FOUND: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
        FOUND
            .get_or_init(|| {
                let candidates: &[&str] = if cfg!(target_os = "macos") {
                    &["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/STHeiti Light.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"]
                } else if cfg!(windows) {
                    &["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf", "C:\\Windows\\Fonts\\simsun.ttc"]
                } else {
                    &[
                        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
                        "/usr/local/share/fonts/noto/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                        "/usr/share/fonts/wqy-microhei/wqy-microhei.ttc",
                        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
                        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                    ]
                };
                candidates.iter().map(std::path::PathBuf::from).find(|p| p.is_file())
            })
            .as_deref()
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// An installed font with Arabic glyphs, for the العربية interface; fonts are never bundled
/// (only looked for where the OS or common packages put them). Checked once.
pub fn arabic_font() -> Option<&'static std::path::Path> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static FOUND: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
        FOUND
            .get_or_init(|| {
                let candidates: &[&str] = if cfg!(target_os = "macos") {
                    &[
                        "/System/Library/Fonts/GeezaPro.ttc",
                        "/System/Library/Fonts/Supplemental/Geeza Pro.ttf",
                        "/System/Library/Fonts/Supplemental/Al Nile.ttc",
                        "/System/Library/Fonts/Supplemental/Baghdad.ttf",
                    ]
                } else if cfg!(windows) {
                    &[
                        "C:\\Windows\\Fonts\\segoeui.ttf",
                        "C:\\Windows\\Fonts\\tahoma.ttf",
                        "C:\\Windows\\Fonts\\trado.ttf",
                        "C:\\Windows\\Fonts\\arabtype.ttf",
                        "C:\\Windows\\Fonts\\arial.ttf",
                    ]
                } else {
                    &[
                        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                        "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
                        "/usr/share/fonts/opentype/noto/NotoSansArabic-Regular.ttf",
                        "/usr/share/fonts/truetype/noto/NotoNaskhArabic-Regular.ttf",
                        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                        "/usr/share/fonts/TTF/DejaVuSans.ttf",
                        "/usr/share/fonts/truetype/amiri/Amiri-Regular.ttf",
                        "/usr/share/fonts/truetype/kacst/KacstOne.ttf",
                    ]
                };
                candidates.iter().map(std::path::PathBuf::from).find(|p| p.is_file())
            })
            .as_deref()
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// An installed Japanese font, for Japanese names in a non-Chinese interface. Checked once.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn japanese_font() -> Option<&'static std::path::Path> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static FOUND: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
        FOUND
            .get_or_init(|| {
                let candidates: &[&str] = if cfg!(target_os = "macos") {
                    &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"]
                } else if cfg!(windows) {
                    &["C:\\Windows\\Fonts\\YuGothR.ttc", "C:\\Windows\\Fonts\\meiryo.ttc", "C:\\Windows\\Fonts\\msgothic.ttc"]
                } else {
                    &[
                        "/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf",
                        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
                        "/usr/share/fonts/ipa-pgothic/ipagp.ttf",
                        "/usr/share/fonts/vl-gothic/VL-PGothic-Regular.ttf",
                    ]
                };
                candidates.iter().map(std::path::PathBuf::from).find(|p| p.is_file())
            })
            .as_deref()
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// The Hangul letters the Korean interface shows: its catalog and its own name.
fn hangul_letters() -> &'static std::collections::BTreeSet<char> {
    static LETTERS: std::sync::OnceLock<std::collections::BTreeSet<char>> = std::sync::OnceLock::new();
    LETTERS.get_or_init(|| {
        include_str!("i18n/ko.tsv")
            .lines()
            .filter_map(|line| line.split('\t').nth(2))
            .flat_map(str::chars)
            .chain("한국어".chars())
            .filter(|c| matches!(c, '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' | '\u{a960}'..='\u{a97f}' | '\u{ac00}'..='\u{d7ff}'))
            .collect()
    })
}

/// Whether `face` has every Hangul letter the Korean interface shows (rejects malformed fonts
/// and Chinese- or Japanese-only faces).
fn covers_hangul(face: &cadcraft_fonts::ttf::Face) -> bool {
    hangul_letters().iter().all(|c| face.has_glyph(*c))
}

/// The first face of a font file (the one egui uses) when it covers the Korean interface.
fn korean_face(bytes: std::sync::Arc<Vec<u8>>) -> Option<cadcraft_fonts::ttf::Face> {
    cadcraft_fonts::ttf::faces(&bytes).into_iter().find(covers_hangul)
}

/// Whether the CJK fonts [`install_fonts`] loads in every language already show Hangul (Noto Sans
/// CJK, Microsoft YaHei, PingFang do). Only asked about Korean, so other interfaces never read the
/// font twice. Checked once.
pub fn cjk_covers_hangul() -> bool {
    static COVERS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *COVERS.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // egui uses a collection's first face, and each letter from the first font with it.
            let first_covers = |p: &std::path::Path| {
                read_capped(p, MAX_UI_FONT_BYTES)
                    .and_then(|bytes| cadcraft_fonts::ttf::faces(&std::sync::Arc::new(bytes)).into_iter().next())
                    .is_some_and(|face| covers_hangul(&face))
            };
            if cjk_font().is_some_and(first_covers) {
                return true;
            }
        }
        cadcraft_fonts::CRAFT_FONTS
            .iter()
            .filter(|f| f.style == "Regular" && f.scripts.iter().any(|s| matches!(*s, "Jpan" | "Hans" | "Hant")))
            .any(|f| cadcraft_fonts::ttf::faces(&std::sync::Arc::new(f.bytes.to_vec())).first().is_some_and(covers_hangul))
    })
}

/// A Hangul font for the Korean interface when the CJK fonts don't cover it: a Korean face from
/// the optional craft-fonts input, else one installed where the OS or common packages put it.
/// Nothing is bundled or downloaded; without one the interface stays English. Only looked for
/// when Korean is asked about. Checked once; collections are scanned with the fonts crate's
/// face-count cap and reads are capped.
pub fn korean_font() -> Option<&'static cadcraft_fonts::ttf::Face> {
    static FOUND: std::sync::OnceLock<Option<cadcraft_fonts::ttf::Face>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            for font in cadcraft_fonts::CRAFT_FONTS.iter().filter(|f| f.scripts.iter().any(|s| matches!(*s, "Hang" | "Kore"))) {
                if font.bytes.len() as u64 > MAX_UI_FONT_BYTES {
                    continue;
                }
                if let Some(face) = korean_face(std::sync::Arc::new(font.bytes.to_vec())) {
                    return Some(face);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let candidates: &[&str] = if cfg!(target_os = "macos") {
                    &[
                        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
                        "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
                        "/Library/Fonts/NanumGothic.ttf",
                    ]
                } else if cfg!(windows) {
                    &["C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\gulim.ttc"]
                } else {
                    &[
                        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
                        "/usr/share/fonts/nanum/NanumGothic.ttf",
                        "/usr/share/fonts/opentype/noto/NotoSansCJKkr-Regular.otf",
                        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
                        "/usr/local/share/fonts/nanum/NanumGothic.ttf",
                        "/usr/local/share/fonts/noto/NotoSansCJK-Regular.ttc",
                    ]
                };
                for path in candidates {
                    if let Some(face) =
                        read_capped(std::path::Path::new(path), MAX_UI_FONT_BYTES).and_then(|bytes| korean_face(std::sync::Arc::new(bytes)))
                    {
                        return Some(face);
                    }
                }
            }
            None
        })
        .as_ref()
}

/// Whether the interface in `language` shows Hangul (the Korean name in the language picker):
/// in every language when the CJK fonts cover it, in a Korean interface with [`korean_font`].
pub fn shows_hangul(language: &str) -> bool {
    cjk_covers_hangul() || (language == "ko" && korean_font().is_some())
}

/// Whether the interface can show `language` without missing-glyph boxes: Chinese needs an
/// installed CJK font, Korean a font with Hangul, العربية an installed Arabic font.
pub fn can_show(language: &str) -> bool {
    match language {
        "ko" => shows_hangul("ko"),
        "vi" => vietnamese_ui_font(),
        "ar" => arabic_font().is_some(),
        l if l.starts_with("zh") => cjk_font().is_some(),
        _ => true,
    }
}

/// egui's visuals for `theme`, from its tokens.
fn visuals(theme: Theme) -> Visuals {
    let t = Tokens::of(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    v.panel_fill = t.panel;
    v.window_fill = t.chrome;
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.chrome_dark;
    v.override_text_color = Some(t.text);
    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.noninteractive.fg_stroke.color = t.text;
    v.widgets.noninteractive.bg_stroke.color = t.border;
    v.widgets.inactive.bg_fill = t.control;
    v.widgets.inactive.weak_bg_fill = t.control;
    v.widgets.inactive.fg_stroke.color = t.text;
    v.widgets.hovered.bg_fill = t.control_hover;
    v.widgets.hovered.weak_bg_fill = t.control_hover;
    v.widgets.active.bg_fill = t.accent;
    v.widgets.active.weak_bg_fill = t.accent;
    v.selection.bg_fill = t.select_bg;
    // egui's default fades the text with alpha, which falls below WCAG AA on the chrome.
    v.weak_text_color = Some(t.text_dim);
    v.window_stroke = egui::Stroke::new(1.0, t.border);
    let shadow = Color32::from_black_alpha(if theme == Theme::Dark { 90 } else { 40 });
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: shadow };
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(5);
    v
}

/// Install the styles of both themes and show `theme` (egui's own widgets, popups and menus
/// follow it; our own painting follows [`Tokens::get`]). `follow_system` is true for the
/// [`ThemePref::System`] choice, whose `theme` was resolved from the OS report.
pub fn apply(ctx: &egui::Context, theme: Theme, follow_system: bool) {
    set_active(theme);
    ctx.set_visuals_of(Theme::Dark, visuals(Theme::Dark));
    ctx.set_visuals_of(Theme::Light, visuals(Theme::Light));
    // A System choice keeps egui's own System preference: a concrete Light/Dark would pin the native
    // window appearance (winit then stops reporting OS changes) and the app would never follow again.
    ctx.set_theme(if follow_system { egui::ThemePreference::System } else { egui::ThemePreference::from(theme) });
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, body());
        s.text_styles.insert(egui::TextStyle::Button, body());
        s.text_styles.insert(egui::TextStyle::Small, small());
        s.text_styles.insert(egui::TextStyle::Monospace, mono());
    });
    apply_density(ctx, density(ctx));
}

#[cfg(test)]
mod tests {
    #[test]
    fn korean_font_rejects_invalid_or_glyphless_files() {
        for bytes in [vec![], vec![0xff; 32], vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]] {
            assert!(super::korean_face(std::sync::Arc::new(bytes)).is_none());
        }
    }
}
