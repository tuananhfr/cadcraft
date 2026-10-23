//! Moving a tool bar by its grip: the drag state, where a drop would land, the preview, and the
//! one commit when the button is released.

use egui::{Key, Modifiers, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::render::{self, Geometry};
use crate::CadApp;
use crate::theme::Tokens;
use crate::wsprofile::LOCKED_MESSAGE;
use crate::wsprofile::merge::{check, escape_token};
use crate::wsprofile::schema::{Dock, ToolbarState};

/// How close to an edge of the drawing area (or beyond it) a pointer docks a bar instead of floating it.
const SNAP: f32 = 40.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum DragPhase {
    #[default]
    Idle,
    Dragging {
        id: String,
        /// Pointer position relative to the bar's top left corner when the grip was taken.
        grab: Vec2,
        target: Option<DropTarget>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DropTarget {
    /// `order` counts the bars of the row, hidden ones included.
    Edge {
        dock: Dock,
        row: u32,
        order: u32,
    },
    /// Top left corner as fractions of the window's content area.
    Float {
        pos_ratio: [f32; 2],
    },
    Ribbon {
        tab: String,
    },
}

/// Starts dragging `bar`; a bar the company policy holds stays where it is.
pub(super) fn begin(app: &mut CadApp, bar: &ToolbarState, grab: Vec2) {
    if check(&app.ws.effective, &format!("/toolbars/{}", escape_token(&bar.id))).is_err() {
        app.set_status(LOCKED_MESSAGE);
        return;
    }
    app.ws.toolbar_drag = DragPhase::Dragging { id: bar.id.clone(), grab, target: None };
}

fn unit(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

/// Orders for a row after the moving bar takes slot `at` among `members` (existing order, frozen).
/// A frozen bar keeps its order, so the bars after the moving one are only pushed when they are free to move.
fn arrange(members: &[(u32, bool)], at: usize) -> Option<Vec<u32>> {
    let before = members.iter().take(at).map(|member| Some(*member));
    let after = members.iter().skip(at).map(|member| Some(*member));
    let mut orders = Vec::with_capacity(members.len() + 1);
    let mut previous: Option<(u32, bool)> = None;
    let mut previous_original = None;
    for slot in before.chain(std::iter::once(None)).chain(after) {
        let (value, free) = match slot {
            None => (previous.map_or(0, |(order, _)| order.saturating_add(1)), true),
            Some((order, false)) => (previous.map_or(order, |(last, _)| if order > last { order } else { last.saturating_add(1) }), true),
            Some((order, true)) => {
                if previous.is_some_and(|(last, free)| free && order < last) {
                    return None;
                }
                // Existing ties are valid, but inserting a bar must not create a new tie at a frozen bar.
                if previous.is_some_and(|(last, free)| free && order == last) && previous_original != Some(order) {
                    return None;
                }
                (order, false)
            }
        };
        orders.push(value);
        previous = Some((value, free));
        previous_original = slot.map(|(order, _)| order);
    }
    Some(orders)
}

/// The slot closest to `wanted` where the moving bar fits without changing a frozen bar's order.
fn nearest_slot(members: &[(u32, bool)], wanted: usize) -> (usize, Vec<u32>) {
    for distance in 0..=members.len() {
        for slot in [wanted.saturating_add(distance), wanted.saturating_sub(distance)] {
            if slot <= members.len()
                && let Some(orders) = arrange(members, slot)
            {
                return (slot, orders);
            }
        }
    }
    // Conflicting existing orders can leave no legal slot; preserve them and append after the maximum.
    let mut orders: Vec<u32> = members.iter().map(|(order, _)| *order).collect();
    orders.push(members.iter().map(|(order, _)| *order).max().map_or(0, |order| order.saturating_add(1)));
    (members.len(), orders)
}

fn edge_plan(toolbars: &[ToolbarState], id: &str, dock: Dock, row: u32, wanted: u32, frozen: &[String]) -> (Vec<usize>, usize, Vec<u32>) {
    let mut members: Vec<usize> =
        toolbars.iter().enumerate().filter(|(_, bar)| bar.id != id && bar.dock == dock && bar.row == row).map(|(index, _)| index).collect();
    members.sort_by_key(|index| toolbars.get(*index).map_or(0, |bar| bar.order));
    let state: Vec<(u32, bool)> = members.iter().filter_map(|index| toolbars.get(*index)).map(|bar| (bar.order, frozen.contains(&bar.id))).collect();
    let (slot, orders) = nearest_slot(&state, (wanted as usize).min(state.len()));
    (members, slot, orders)
}

/// The legal slot used by both the drag preview and the committed drop.
fn snapped_target(toolbars: &[ToolbarState], id: &str, target: &DropTarget, frozen: &[String]) -> DropTarget {
    match target {
        DropTarget::Edge { dock, row, order } => {
            let (_, slot, _) = edge_plan(toolbars, id, *dock, *row, *order, frozen);
            DropTarget::Edge { dock: *dock, row: *row, order: u32::try_from(slot).unwrap_or(u32::MAX) }
        }
        DropTarget::Float { .. } | DropTarget::Ribbon { .. } => target.clone(),
    }
}

/// Puts the bar `id` where `target` says. Bars in `frozen` (order held by policy) are never renumbered;
/// a drop that would need that lands in the nearest slot that does not.
pub fn apply_drop(toolbars: &mut [ToolbarState], id: &str, target: &DropTarget, frozen: &[String]) -> Result<(), String> {
    let at = toolbars
        .iter()
        .position(|bar| bar.id == id)
        .ok_or_else(|| crate::tf!("Unknown toolbar: {id}", id = id.chars().take(64).collect::<String>()))?;
    match target {
        DropTarget::Ribbon { .. } => return Err(crate::tl!("Use ui.toolbar.to_ribbon to convert a toolbar").to_string()),
        DropTarget::Float { pos_ratio } => {
            let Some(bar) = toolbars.get_mut(at) else { return Ok(()) };
            bar.dock = Dock::Floating;
            bar.x_ratio = unit(pos_ratio[0]);
            bar.y_ratio = unit(pos_ratio[1]);
        }
        DropTarget::Edge { dock, row, order } => {
            if *dock == Dock::Floating {
                return Err(crate::tl!("A tool bar docks to the top, bottom, left or right edge").into());
            }
            let Some(bar) = toolbars.get_mut(at) else { return Ok(()) };
            bar.dock = *dock;
            bar.row = *row;
            let (mut row_members, slot, orders) = edge_plan(toolbars, id, *dock, *row, *order, frozen);
            row_members.insert(slot, at);
            for (i, order) in row_members.into_iter().zip(orders) {
                if let Some(bar) = toolbars.get_mut(i) {
                    bar.order = order;
                }
            }
        }
    }
    Ok(())
}

/// Bars whose order the company policy holds; a drop must leave them alone.
pub(super) fn frozen_orders(app: &CadApp) -> Vec<String> {
    let held = app.ws.user.layout.toolbars.iter();
    held.filter(|bar| check(&app.ws.effective, &format!("/toolbars/{}/order", escape_token(&bar.id))).is_err()).map(|bar| bar.id.clone()).collect()
}

/// The bars of `dock` row `row` other than `moving`, by position in the row: full index in the
/// row (hidden bars count) and the rectangle when the bar is on screen.
fn row_members(toolbars: &[ToolbarState], geometry: &Geometry, moving: &str, dock: Dock, row: u32) -> Vec<Option<Rect>> {
    let mut members: Vec<&ToolbarState> = toolbars.iter().filter(|bar| bar.dock == dock && bar.row == row && bar.id != moving).collect();
    members.sort_by_key(|bar| bar.order);
    members.iter().map(|bar| geometry.bars.iter().find(|drawn| drawn.id == bar.id).map(|drawn| drawn.rect)).collect()
}

fn edge_target(dock: Dock, pointer: Pos2, moving: &str, toolbars: &[ToolbarState], geometry: &Geometry) -> DropTarget {
    let horizontal = matches!(dock, Dock::Top | Dock::Bottom);
    let (cross, main) = if horizontal { (pointer.y, pointer.x) } else { (pointer.x, pointer.y) };
    let rows: Vec<&render::RowRect> = geometry.rows.iter().filter(|row| row.dock == dock).collect();
    let contains = |row: &render::RowRect| {
        let (low, high) = if horizontal { (row.rect.top(), row.rect.bottom()) } else { (row.rect.left(), row.rect.right()) };
        cross >= low && cross < high
    };
    let toward_center = |inner: &render::RowRect| match dock {
        Dock::Top => cross >= inner.rect.bottom(),
        Dock::Left => cross >= inner.rect.right(),
        Dock::Bottom => cross < inner.rect.top(),
        Dock::Right | Dock::Floating => cross < inner.rect.left(),
    };
    let row = match (rows.iter().find(|row| contains(row)), rows.last()) {
        (Some(row), _) => row.row,
        (None, None) => 0,
        (None, Some(inner)) if toward_center(inner) => {
            // A bar alone in the innermost row stays there rather than opening a row for itself.
            let alone = toolbars.iter().filter(|bar| bar.dock == dock && bar.row == inner.row).all(|bar| bar.id == moving);
            if alone { inner.row } else { inner.row.saturating_add(1) }
        }
        (None, Some(_)) => rows.first().map_or(0, |outer| outer.row),
    };
    let members = row_members(toolbars, geometry, moving, dock, row);
    let before = |rect: &Rect| if horizontal { rect.center().x < main } else { rect.center().y < main };
    // Index of the first visible bar the pointer is before; hidden bars keep their place around it.
    let order = members
        .iter()
        .position(|rect| rect.as_ref().is_some_and(|rect| !before(rect)))
        .or_else(|| members.iter().rposition(Option::is_some).map(|last| last + 1))
        .unwrap_or(members.len());
    DropTarget::Edge { dock, row, order: u32::try_from(order).unwrap_or(u32::MAX) }
}

/// Where dropping with the pointer at `pointer` would put the bar `moving`.
pub(super) fn target_at(pointer: Pos2, grab: Vec2, moving: &str, toolbars: &[ToolbarState], geometry: &Geometry, content: Rect) -> DropTarget {
    if let Some((rect, active)) = &geometry.ribbon
        && rect.contains(pointer)
    {
        let tab = geometry.ribbon_tabs.iter().find(|(rect, _)| rect.contains(pointer)).map_or(active, |(_, tab)| tab);
        return DropTarget::Ribbon { tab: tab.clone() };
    }
    let center = geometry.center;
    let nearest = if center.is_positive() {
        [
            (Dock::Top, pointer.y - center.top()),
            (Dock::Bottom, center.bottom() - pointer.y),
            (Dock::Left, pointer.x - center.left()),
            (Dock::Right, center.right() - pointer.x),
        ]
        .into_iter()
        .filter(|(_, distance)| *distance < SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1))
    } else {
        None
    };
    match nearest {
        Some((dock, _)) => edge_target(dock, pointer, moving, toolbars, geometry),
        None => {
            let corner = pointer - grab;
            let size = content.size().max(vec2(1.0, 1.0));
            DropTarget::Float { pos_ratio: [unit((corner.x - content.left()) / size.x), unit((corner.y - content.top()) / size.y)] }
        }
    }
}

fn paint_preview(ctx: &egui::Context, target: &DropTarget, moving: &str, toolbars: &[ToolbarState], geometry: &Geometry, content: Rect) {
    let t = Tokens::get();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cc_toolbar_preview")));
    let stroke = Stroke::new(2.0, t.accent);
    let height = render::bar_height(crate::theme::density(ctx));
    match target {
        DropTarget::Ribbon { tab } => {
            if let Some((rect, _)) = geometry.ribbon_tabs.iter().find(|(_, id)| id == tab).or(geometry.ribbon.as_ref()) {
                painter.rect_stroke(*rect, 3.0, stroke, StrokeKind::Inside);
            }
        }
        DropTarget::Float { pos_ratio } => {
            let size = render::drawn_size(geometry, moving, crate::theme::density(ctx));
            let min = pos2(content.left() + pos_ratio[0] * content.width(), content.top() + pos_ratio[1] * content.height());
            let rect = Rect::from_min_size(min, size).intersect(content);
            painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
        }
        DropTarget::Edge { dock, row, order } => {
            let horizontal = matches!(dock, Dock::Top | Dock::Bottom);
            let band = geometry.rows.iter().find(|held| held.dock == *dock && held.row == *row).map(|held| held.rect).or_else(|| {
                let edge = render::edge_index(*dock).and_then(|i| geometry.edges.get(i)).copied()?;
                let inward = match dock {
                    Dock::Top => vec2(0.0, height),
                    Dock::Bottom => vec2(0.0, -height),
                    Dock::Left => vec2(height, 0.0),
                    Dock::Right | Dock::Floating => vec2(-height, 0.0),
                };
                let inner = geometry.rows.iter().filter(|held| held.dock == *dock).map(|held| held.rect).fold(edge, |all, rect| all.union(rect));
                let new_row = match dock {
                    Dock::Top => Rect::from_min_max(pos2(inner.left(), inner.bottom()), pos2(inner.right(), inner.bottom() + inward.y)),
                    Dock::Bottom => Rect::from_min_max(pos2(inner.left(), inner.top() + inward.y), pos2(inner.right(), inner.top())),
                    Dock::Left => Rect::from_min_max(pos2(inner.right(), inner.top()), pos2(inner.right() + inward.x, inner.bottom())),
                    Dock::Right | Dock::Floating => {
                        Rect::from_min_max(pos2(inner.left() + inward.x, inner.top()), pos2(inner.left(), inner.bottom()))
                    }
                };
                Some(new_row)
            });
            let Some(band) = band else { return };
            painter.rect_stroke(band, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
            let members = row_members(toolbars, geometry, moving, *dock, *row);
            let index = (*order as usize).min(members.len());
            let before = members.iter().skip(index).flatten().next();
            let after = members.iter().take(index).flatten().last();
            let along = match (before, after, horizontal) {
                (Some(rect), _, true) => rect.left() - 2.0,
                (Some(rect), _, false) => rect.top() - 2.0,
                (None, Some(rect), true) => rect.right() + 2.0,
                (None, Some(rect), false) => rect.bottom() + 2.0,
                (None, None, true) => band.left() + 4.0,
                (None, None, false) => band.top() + 4.0,
            };
            let mark = if horizontal {
                Rect::from_min_max(pos2(along - 1.5, band.top() + 3.0), pos2(along + 1.5, band.bottom() - 3.0))
            } else {
                Rect::from_min_max(pos2(band.left() + 3.0, along - 1.5), pos2(band.right() - 3.0, along + 1.5))
            };
            painter.rect_filled(mark, 1.5, t.accent);
        }
    }
}

fn commit(app: &mut CadApp, id: &str, target: &DropTarget) {
    if let DropTarget::Ribbon { tab } = target {
        match app.run("ui.toolbar.to_ribbon", serde_json::json!({"toolbar": id, "tab": tab})) {
            Ok(_) => app.ws.autosave.interaction_ended(),
            Err(error) => app.set_status(error),
        }
        return;
    }
    let frozen = frozen_orders(app);
    match crate::wsprofile::edit_toolbars(app, |toolbars| apply_drop(toolbars, id, target, &frozen)) {
        Ok(true) => app.ws.autosave.interaction_ended(),
        Ok(false) => {}
        Err(error) => app.set_status(error),
    }
}

/// Runs the drag in progress for this frame: Esc cancels it, the pointer moves its target, and the
/// release commits it as one layout change.
pub(super) fn tick(ctx: &egui::Context, app: &mut CadApp, content: Rect) {
    let DragPhase::Dragging { id, grab, .. } = app.ws.toolbar_drag.clone() else { return };
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        app.ws.toolbar_drag = DragPhase::Idle;
        return;
    }
    let (down, pointer) = ctx.input(|input| (input.pointer.primary_down(), input.pointer.latest_pos()));
    if down {
        let geometry = render::current(ctx);
        let Some(pointer) = pointer else { return };
        let target = target_at(pointer, grab, &id, &app.ws.applied.toolbars, &geometry, content);
        let target = snapped_target(&app.ws.applied.toolbars, &id, &target, &frozen_orders(app));
        paint_preview(ctx, &target, &id, &app.ws.applied.toolbars, &geometry, content);
        app.ws.toolbar_drag = DragPhase::Dragging { id, grab, target: Some(target) };
        ctx.request_repaint();
        return;
    }
    let phase = std::mem::take(&mut app.ws.toolbar_drag);
    if let DragPhase::Dragging { id, target: Some(target), .. } = phase {
        commit(app, &id, &target);
    }
}
