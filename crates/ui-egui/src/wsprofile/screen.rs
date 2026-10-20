//! Screen geometry: rectangles stored as fractions of the content area, so a layout saved on one
//! screen opens on a smaller one with every floating window still reachable.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenProfile {
    pub monitor_count: u32,
    /// `[x, y, width, height]` in points.
    pub work_area: [f32; 4],
    pub pixel_ratio: f32,
    pub window_mode: String,
}

impl Default for ScreenProfile {
    fn default() -> Self {
        Self { monitor_count: 1, work_area: [0.0, 0.0, 1280.0, 800.0], pixel_ratio: 1.0, window_mode: "windowed".into() }
    }
}

fn unit(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

fn usable(area: egui::Rect) -> bool {
    area.is_finite() && area.width() > 0.0 && area.height() > 0.0
}

/// `rect` as `[x, y, width, height]` fractions of `area`, each within 0..=1.
pub fn to_ratio(rect: egui::Rect, area: egui::Rect) -> [f32; 4] {
    if !usable(area) || !rect.is_finite() {
        return [0.0; 4];
    }
    let (width, height) = (area.width(), area.height());
    [unit((rect.min.x - area.min.x) / width), unit((rect.min.y - area.min.y) / height), unit(rect.width() / width), unit(rect.height() / height)]
}

/// The rectangle `r` describes in `area`, no larger than `area` and moved so that at least
/// `min_visible` points of it (or all of it, when smaller) stay inside.
pub fn from_ratio(r: [f32; 4], area: egui::Rect, min_visible: f32) -> egui::Rect {
    if !usable(area) {
        return egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
    }
    let [x, y, w, h] = r.map(unit);
    let min_visible = if min_visible.is_finite() { min_visible.max(1.0) } else { 1.0 };
    let width = (w * area.width()).clamp(min_visible.min(area.width()), area.width());
    let height = (h * area.height()).clamp(min_visible.min(area.height()), area.height());
    let keep_x = min_visible.min(width);
    let keep_y = min_visible.min(height);
    let left = (area.min.x + x * area.width()).clamp(area.min.x - (width - keep_x), area.max.x - keep_x);
    let top = (area.min.y + y * area.height()).clamp(area.min.y, area.max.y - keep_y);
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height))
}
