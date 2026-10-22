use cadcraft_engine::prompt::Mark;
use egui::{Align2, FontId, Shape, Stroke};

use crate::canvas::Xf;
use crate::theme::Tokens;

pub(crate) fn draw_marks(p: &egui::Painter, xf: &Xf, marks: &[Mark], t: &Tokens) {
    for mark in marks.iter().take(cadcraft_engine::sheetplan::MAX_SCAN_FRAMES) {
        if mark.outline.len() < 3 || mark.outline.len() > 64 {
            continue;
        }
        let points: Vec<_> = mark.outline.iter().map(|point| xf.to_screen(*point)).collect();
        if points.iter().any(|point| !point.x.is_finite() || !point.y.is_finite()) {
            continue;
        }
        let color = if mark.warn { t.warn } else { t.accent };
        let n = points.len() as f32;
        let center = points.iter().fold(egui::Vec2::ZERO, |sum, point| sum + point.to_vec2() / n).to_pos2();
        p.add(Shape::closed_line(points, Stroke::new(2.0, color)));
        p.circle_filled(center, 9.0, t.canvas);
        p.circle_stroke(center, 9.0, Stroke::new(2.0, color));
        p.text(center, Align2::CENTER_CENTER, &mark.label, FontId::proportional(12.0), color);
    }
}
