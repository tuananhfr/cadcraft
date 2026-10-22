//! Keeps dock splits and floating groups within the usable work area.

use craft_ui::docking::{Layout, MAX_DEPTH, MAX_PANELS, Node};
use craft_ui::layout::{SplitAxis, SplitSize};

use crate::docking::Panel;
use crate::wsprofile::apply::MIN_VISIBLE;

pub const MIN_RATIO: f32 = 0.15;
pub const MAX_RATIO: f32 = 0.85;
pub const MAX_FLOAT_SHARE: f32 = 0.85;

/// Clamps splits on the canvas path and floating rects so the canvas keeps at least 15% per axis.
pub fn clamp_layout(layout: &mut Layout<Panel>, content: egui::Rect) -> bool {
    if !content.is_finite() || !content.is_positive() || !content.size().is_finite() || layout.validate().is_err() {
        return false;
    }
    let mut changed = false;
    if let Some(root) = &mut layout.root
        && let Some(path) = canvas_path(root)
    {
        let mut node = root;
        for second_branch in path {
            let Node::Split { axis, size, first, second } = node else { break };
            let total = if *axis == SplitAxis::Horizontal { content.width() } else { content.height() };
            let next = match *size {
                SplitSize::Ratio(ratio) => SplitSize::Ratio(ratio.clamp(MIN_RATIO, MAX_RATIO)),
                SplitSize::FixedFirst(points) => SplitSize::FixedFirst(points.clamp(MIN_RATIO * total, MAX_RATIO * total)),
                SplitSize::FixedSecond(points) => SplitSize::FixedSecond(points.clamp(MIN_RATIO * total, MAX_RATIO * total)),
            };
            if *size != next {
                *size = next;
                changed = true;
            }
            node = if second_branch { second } else { first };
        }
    }
    for group in &mut layout.floating {
        let [x, y, width, height] = group.rect;
        let maximum = MAX_FLOAT_SHARE * content.size();
        let width = width.clamp(MIN_VISIBLE.min(maximum.x), maximum.x);
        let height = height.clamp(MIN_VISIBLE.min(maximum.y), maximum.y);
        let x = x.max(content.left()).min(content.right() - width);
        let y = y.max(content.top()).min(content.bottom() - height);
        let next = [x, y, width, height];
        if group.rect != next {
            group.rect = next;
            changed = true;
        }
    }
    changed
}

fn canvas_path(node: &Node<Panel>) -> Option<Vec<bool>> {
    let mut pending = vec![(node, Vec::new())];
    // Public callers may supply an unvalidated tree; traversal must still stay bounded.
    for _ in 0..MAX_PANELS * 2 {
        let (node, path) = pending.pop()?;
        match node {
            Node::Tabs { panels, .. } if panels.iter().take(MAX_PANELS).any(|panel| *panel == Panel::Canvas) => return Some(path),
            Node::Stack { entries } if entries.iter().take(MAX_PANELS).any(|entry| entry.panel == Panel::Canvas) => return Some(path),
            Node::Split { first, second, .. } if path.len() < MAX_DEPTH => {
                let mut second_path = path.clone();
                second_path.push(true);
                pending.push((second.as_ref(), second_path));
                let mut first_path = path;
                first_path.push(false);
                pending.push((first.as_ref(), first_path));
            }
            _ => {}
        }
    }
    None
}
