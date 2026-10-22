//! Painting the tool bars: docked rows on the four window edges, floating bars, and the geometry
//! the drag code hit-tests against.

use craft_ui::layout::{ToolbarFit, fit_toolbar};
use egui::{Id, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::menu;
use super::{Command, docked, drag, placeholder_label, resolve};
use crate::CadApp;
use crate::icons;
use crate::theme::Tokens;
use crate::wsprofile::schema::{Dock, ToolbarState};
use crate::wsprofile::screen::from_ratio;

/// Thickness of one tool bar row.
pub(super) const BAR: f32 = 34.0;
/// Points of a floating bar that stay inside the window.
pub(super) const MIN_VISIBLE: f32 = 24.0;
const BUTTON: f32 = 26.0;
const STEP: f32 = BUTTON + 6.0;
const GROUP_GAP: f32 = 24.0;
const OVERFLOW: f32 = 30.0;
const GRIP: f32 = 10.0;
const BAR_GAP: f32 = 8.0;
const BUILDING: &str = "cc_toolbar_geometry_building";
const LAST: &str = "cc_toolbar_geometry";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug)]
pub(super) struct RowRect {
    pub dock: Dock,
    pub row: u32,
    pub rect: Rect,
}

#[derive(Clone, Debug)]
pub(super) struct BarRect {
    pub id: String,
    pub rect: Rect,
}

/// Where the bars were drawn in one frame; the drag code decides drop targets from it.
#[derive(Clone, Debug)]
pub(super) struct Geometry {
    pass: u64,
    /// What the window leaves for the drawing after the four edges.
    pub center: Rect,
    /// Per edge (top, bottom, left, right): the strip its rows fill, flat on the edge when it has none.
    pub edges: [Rect; 4],
    pub rows: Vec<RowRect>,
    pub bars: Vec<BarRect>,
    /// Every keyboard stop of a bar with the bar it belongs to, for the key that opens its menu.
    pub stops: Vec<(Id, Id)>,
}

impl Default for Geometry {
    fn default() -> Self {
        Self { pass: 0, center: Rect::NOTHING, edges: [Rect::NOTHING; 4], rows: Vec::new(), bars: Vec::new(), stops: Vec::new() }
    }
}

pub(super) fn edge_index(dock: Dock) -> Option<usize> {
    match dock {
        Dock::Top => Some(0),
        Dock::Bottom => Some(1),
        Dock::Left => Some(2),
        Dock::Right => Some(3),
        Dock::Floating => None,
    }
}

fn building(ctx: &egui::Context, add: impl FnOnce(&mut Geometry)) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let slot = data.get_temp_mut_or_insert_with(Id::new(BUILDING), Geometry::default);
        if slot.pass != pass {
            *slot = Geometry { pass, ..Geometry::default() };
        }
        add(slot);
    });
}

/// Keeps this frame's geometry for the frames that follow (keyboard routing, the next drag step).
pub(super) fn finish_frame(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    let done = ctx.data(|data| data.get_temp::<Geometry>(Id::new(BUILDING)).filter(|geometry| geometry.pass == pass)).unwrap_or_default();
    ctx.data_mut(|data| data.insert_temp(Id::new(LAST), done));
}

pub(super) fn current(ctx: &egui::Context) -> Geometry {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<Geometry>(Id::new(BUILDING)).filter(|geometry| geometry.pass == pass)).unwrap_or_default()
}

pub(super) fn last(ctx: &egui::Context) -> Geometry {
    ctx.data(|data| data.get_temp::<Geometry>(Id::new(LAST))).unwrap_or_default()
}

type Group = Vec<(String, Command)>;

/// The command groups of `bar`: the shipped separators when it still holds the shipped commands,
/// otherwise one run.
fn groups_of(bar: &ToolbarState) -> Vec<Group> {
    if let Some(shipped) = super::builtin_groups(&bar.id) {
        let ids: Vec<&str> = shipped.iter().flatten().map(|tool| tool.1).collect();
        if ids.len() == bar.commands.len() && ids.iter().zip(&bar.commands).all(|(shipped, held)| shipped == held) {
            return shipped.iter().map(|group| group.iter().map(|tool| (tool.1.to_string(), resolve(tool.1))).collect()).collect();
        }
    }
    if bar.commands.is_empty() {
        return Vec::new();
    }
    vec![bar.commands.iter().map(|id| (id.clone(), resolve(id))).collect()]
}

fn group_width(buttons: usize) -> f32 {
    buttons as f32 * STEP - (STEP - BUTTON)
}

fn grip_width(bar: &ToolbarState) -> f32 {
    if bar.locked { 0.0 } else { GRIP }
}

/// How much of a bar's main axis `fit` uses, grip included.
fn extent(bar: &ToolbarState, fit: &ToolbarFit) -> f32 {
    let shown: Vec<f32> = fit.shown.iter().map(|(_, width)| *width).collect();
    let shown_total = shown.iter().sum::<f32>() + GROUP_GAP * shown.len().saturating_sub(1) as f32;
    let tail = match (fit.overflow.is_empty(), shown.is_empty()) {
        (true, _) => 0.0,
        (false, true) => OVERFLOW,
        (false, false) => GROUP_GAP + OVERFLOW,
    };
    grip_width(bar) + shown_total + tail
}

fn fitted(bar: &ToolbarState, groups: &[Group], available: f32) -> ToolbarFit {
    let widths: Vec<f32> = groups.iter().map(|group| group_width(group.len())).collect();
    let room = (available - grip_width(bar)).max(0.0);
    // Too narrow even for the overflow button: it alone stands for every group.
    fit_toolbar(&widths, room, GROUP_GAP, OVERFLOW).unwrap_or_else(|| ToolbarFit {
        shown: Vec::new(),
        overflow: (0..groups.len()).collect(),
        overflow_width: OVERFLOW,
    })
}

fn along(axis: Axis, band: Rect, main: f32, cross: f32, length: f32, breadth: f32) -> Rect {
    match axis {
        Axis::Horizontal => Rect::from_min_size(pos2(band.min.x + main, band.min.y + cross), vec2(length, breadth)),
        Axis::Vertical => Rect::from_min_size(pos2(band.min.x + cross, band.min.y + main), vec2(breadth, length)),
    }
}

fn paint_grip(ui: &egui::Ui, rect: Rect, axis: Axis, color: egui::Color32) {
    let center = rect.center();
    for i in 0..3 {
        for j in 0..2 {
            let (along_axis, across) = ((i as f32 - 1.0) * 5.0, (j as f32 - 0.5) * 4.0);
            let offset = if axis == Axis::Horizontal { vec2(across, along_axis) } else { vec2(along_axis, across) };
            ui.painter().circle_filled(center + offset, 1.0, color);
        }
    }
}

fn initials(label: &str) -> String {
    label.chars().filter(|ch| ch.is_alphanumeric()).take(2).flat_map(char::to_uppercase).collect()
}

fn known_button(ui: &mut egui::Ui, rect: Rect, id: Id, command: &Command, mac: bool) -> egui::Response {
    let t = Tokens::get();
    let response = ui.interact(rect, id, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.control_hover.gamma_multiply(0.6));
    }
    match command.icon {
        Some(icon) => icons::paint(ui.painter(), rect.shrink(4.0), icon, false),
        None => {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials(&command.label), crate::theme::small(), t.icon);
        }
    }
    let label = crate::i18n::t(&command.label);
    let tip = match command.shortcut {
        Some(shortcut) => format!("{label} ({})", crate::menus::shortcut_label(shortcut, mac)),
        None => label.to_string(),
    };
    icons::describe_control(ui, &response, &tip, None, false);
    response.on_hover_text(tip)
}

fn placeholder_button(ui: &mut egui::Ui, rect: Rect, id: Id, command: &Command) {
    let t = Tokens::get();
    let label = placeholder_label(command);
    let response = ui.add_enabled_ui(false, |ui| ui.interact(rect, id, Sense::click())).inner;
    ui.painter().rect_stroke(rect.shrink(3.0), 3.0, Stroke::new(1.0, t.text_faint), StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "?", crate::theme::small(), t.text_faint);
    icons::describe_control(ui, &response, &label, None, false);
    response.on_disabled_hover_text(label);
}

/// Draws `bar` in `band` from `start` (along the bar's axis) up to `end`; returns the extent it used.
fn draw_bar(ui: &mut egui::Ui, app: &mut CadApp, bar: &ToolbarState, band: Rect, axis: Axis, start: f32, end: f32) -> f32 {
    let t = Tokens::get();
    let groups = groups_of(bar);
    let fit = fitted(bar, &groups, (end - start).max(0.0));
    let used = extent(bar, &fit);
    let bar_rect = along(axis, band, start, 0.0, used, BAR);
    let owner = ui.id().with(("toolbar", &bar.id));
    let mac = ui.ctx().os().is_mac();
    let being_dragged = matches!(&app.ws.toolbar_drag, drag::DragPhase::Dragging { id, .. } if *id == bar.id);
    let policy_locked =
        crate::wsprofile::merge::check(&app.ws.effective, &format!("/toolbars/{}", crate::wsprofile::merge::escape_token(&bar.id))).is_err();

    // Created first so every button above it takes the pointer; it catches right clicks in the gaps.
    let body = ui.interact(bar_rect, owner.with("body"), Sense::CLICK);
    let mut open = if body.secondary_clicked() { Some(egui::SetOpenCommand::Bool(true)) } else { None };
    let mut by_keyboard = false;
    if menu::take_menu_key(ui.ctx(), owner) {
        open = Some(egui::SetOpenCommand::Bool(true));
        by_keyboard = true;
    }

    if !bar.locked {
        let grip_rect = along(axis, band, start, 5.0, GRIP, BAR - 10.0);
        let grip_id = owner.with("grip");
        let grip = ui.interact(grip_rect, grip_id, Sense::click_and_drag());
        let color = if grip.hovered() || grip.has_focus() { t.text } else { t.text_dim };
        paint_grip(ui, grip_rect, axis, color);
        let name = if bar.label.is_empty() { bar.id.as_str() } else { bar.label.as_str() };
        let label = crate::tf!("{name} toolbar grip", name = crate::i18n::t(name));
        icons::describe_control(ui, &grip, &label, None, false);
        building(ui.ctx(), |geometry| geometry.stops.push((grip_id, owner)));
        if !by_keyboard && grip.secondary_clicked() {
            open = Some(egui::SetOpenCommand::Bool(true));
        } else if !by_keyboard && grip.clicked() {
            open = Some(egui::SetOpenCommand::Toggle);
            by_keyboard = ui.input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space));
        }
        if grip.drag_started() {
            let origin = ui.input(|input| input.pointer.press_origin()).unwrap_or_else(|| grip_rect.center());
            drag::begin(app, bar, origin - bar_rect.min);
        }
        grip.on_hover_cursor(egui::CursorIcon::Grab);
    }

    let mut clicked: Option<String> = None;
    let mut secondary_command = None;
    let mut cursor = grip_width(bar);
    for (shown, (index, width)) in fit.shown.iter().enumerate() {
        let Some(group) = groups.get(*index) else { continue };
        for (position, (command_id, command)) in group.iter().enumerate() {
            let rect = along(axis, band, start + cursor + position as f32 * STEP, (BAR - BUTTON) / 2.0, BUTTON, BUTTON);
            let id = owner.with(("button", *index, position));
            if command.known {
                building(ui.ctx(), |geometry| geometry.stops.push((id, owner)));
                let response = known_button(ui, rect, id, command, mac);
                if response.secondary_clicked() {
                    secondary_command = Some(command_id.clone());
                }
                if response.clicked() {
                    clicked = Some(command_id.clone());
                }
            } else {
                placeholder_button(ui, rect, id, command);
            }
        }
        cursor += width + if shown + 1 < fit.shown.len() || !fit.overflow.is_empty() { GROUP_GAP } else { 0.0 };
    }
    if !fit.overflow.is_empty() {
        let rect = along(axis, band, start + cursor, (BAR - BUTTON) / 2.0, fit.overflow_width, BUTTON);
        building(ui.ctx(), |geometry| geometry.stops.push((owner.with("more"), owner)));
        let response = ui.interact(rect, owner.with("more"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 3.0, t.control_hover.gamma_multiply(0.6));
        }
        for offset in [-5.0, 0.0, 5.0] {
            let offset = if axis == Axis::Horizontal { vec2(offset, 0.0) } else { vec2(0.0, offset) };
            ui.painter().circle_filled(rect.center() + offset, 1.5, t.text);
        }
        icons::describe_control(ui, &response, "More toolbar commands", None, false);
        egui::Popup::menu(&response).show(|ui| {
            for (position, index) in fit.overflow.iter().enumerate() {
                let Some(group) = groups.get(*index) else { continue };
                if position > 0 {
                    ui.separator();
                }
                for (command_id, command) in group {
                    let label = if command.known { crate::i18n::t(&command.label).to_string() } else { placeholder_label(command) };
                    let item = ui.add_enabled(command.known, egui::Button::new(label.as_str()));
                    if command.known {
                        building(ui.ctx(), |geometry| geometry.stops.push((item.id, owner)));
                    }
                    crate::control::record_widget(ui, &item, &label, "button", None);
                    if item.clicked() {
                        clicked = Some(command_id.clone());
                        ui.close();
                    }
                    if command.known {
                        item.context_menu(|ui| {
                            let pinned = app.ws.effective.layout.quick_access.iter().any(|id| id == command_id);
                            if let Some((command, params)) = crate::quick_access::pin_menu(ui, pinned, command_id)
                                && let Err(error) = app.run(command, params)
                            {
                                app.set_status(error);
                            }
                        });
                    }
                }
            }
        });
        response.on_hover_text(crate::tl!("More toolbar commands"));
    }
    let selected_key = body.id.with("command");
    if let Some(command_id) = secondary_command {
        ui.ctx().data_mut(|data| data.insert_temp(selected_key, command_id));
        open = Some(egui::SetOpenCommand::Bool(true));
    } else if open.is_some() {
        ui.ctx().data_mut(|data| data.remove::<String>(selected_key));
    }
    menu::context_menu(app, bar, policy_locked, &body, open, by_keyboard);
    if being_dragged {
        ui.painter().rect_filled(bar_rect, 0.0, t.chrome.gamma_multiply(0.55));
    }
    if let Some(command) = clicked {
        app.start(&command);
    }
    building(ui.ctx(), |geometry| {
        geometry.bars.push(BarRect { id: bar.id.clone(), rect: bar_rect });
    });
    used
}

fn visible_docked(app: &CadApp, dock: Dock) -> Vec<ToolbarState> {
    let standard_shown = app.ui.show_toolbar;
    docked(&app.ws.applied.toolbars, dock)
        .into_iter()
        .filter(|bar| if bar.id == "standard" { standard_shown } else { bar.visible })
        .cloned()
        .collect()
}

fn distinct_rows(bars: &[ToolbarState]) -> Vec<u32> {
    let mut rows: Vec<u32> = bars.iter().map(|bar| bar.row).collect();
    rows.dedup();
    rows
}

/// Row `position` counted from the outer edge: rows fill toward the drawing.
fn row_band(dock: Dock, panel: Rect, position: usize) -> Rect {
    let offset = position as f32 * BAR;
    match dock {
        Dock::Top => Rect::from_min_size(pos2(panel.left(), panel.top() + offset), vec2(panel.width(), BAR)),
        Dock::Bottom => Rect::from_min_size(pos2(panel.left(), panel.bottom() - offset - BAR), vec2(panel.width(), BAR)),
        Dock::Left => Rect::from_min_size(pos2(panel.left() + offset, panel.top()), vec2(BAR, panel.height())),
        Dock::Right | Dock::Floating => Rect::from_min_size(pos2(panel.right() - offset - BAR, panel.top()), vec2(BAR, panel.height())),
    }
}

fn flat_edge(dock: Dock, free: Rect) -> Rect {
    match dock {
        Dock::Top => Rect::from_min_max(free.left_top(), free.right_top()),
        Dock::Bottom => Rect::from_min_max(free.left_bottom(), free.right_bottom()),
        Dock::Left => Rect::from_min_max(free.left_top(), free.left_bottom()),
        Dock::Right | Dock::Floating => Rect::from_min_max(free.right_top(), free.right_bottom()),
    }
}

pub(crate) fn docked_height(app: &CadApp, dock: Dock) -> f32 {
    distinct_rows(&visible_docked(app, dock)).len() as f32 * BAR
}

/// The tool bars docked to `dock`, in a panel as thick as its rows.
pub fn show_docked(ui: &mut egui::Ui, app: &mut CadApp, dock: Dock) {
    let Some(edge) = edge_index(dock) else { return };
    let ctx = ui.ctx().clone();
    let bars = visible_docked(app, dock);
    let rows = distinct_rows(&bars);
    let free = ui.available_rect_before_wrap();
    if rows.is_empty() {
        building(&ctx, |geometry| {
            geometry.edges[edge] = flat_edge(dock, free);
            geometry.center = free;
        });
        return;
    }
    let t = Tokens::get();
    let panel = match dock {
        Dock::Top => egui::Panel::top("cc_toolbars_top"),
        Dock::Bottom => egui::Panel::bottom("cc_toolbars_bottom"),
        Dock::Left => egui::Panel::left("cc_toolbars_left"),
        Dock::Right | Dock::Floating => egui::Panel::right("cc_toolbars_right"),
    };
    let axis = if matches!(dock, Dock::Top | Dock::Bottom) { Axis::Horizontal } else { Axis::Vertical };
    let area = panel.exact_size(rows.len() as f32 * BAR).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
        let panel_rect = ui.max_rect();
        for (position, row) in rows.iter().enumerate() {
            let band = row_band(dock, panel_rect, position);
            building(ui.ctx(), |geometry| geometry.rows.push(RowRect { dock, row: *row, rect: band }));
            let end = match axis {
                Axis::Horizontal => band.width() - 12.0,
                Axis::Vertical => band.height() - 12.0,
            };
            let in_row: Vec<&ToolbarState> = bars.iter().filter(|bar| bar.row == *row).collect();
            let lead = match (dock, position) {
                (Dock::Top, 0) if app.integrated_titlebar => 120.0,
                (Dock::Top | Dock::Bottom, _) => 12.0,
                _ => 6.0,
            };
            let mut cursor = if in_row.first().is_some_and(|bar| !bar.locked) { (lead - GRIP).max(2.0) } else { lead };
            for bar in in_row {
                cursor += draw_bar(ui, app, bar, band, axis, cursor, end) + BAR_GAP;
            }
        }
        panel_rect
    });
    let rect = area.inner;
    building(&ctx, |geometry| {
        geometry.edges[edge] = rect;
        geometry.center = ui.available_rect_before_wrap();
    });
}

/// Places the floating bar at its stored ratios, clamped entirely inside the content area.
pub fn floating_rect(bar: &ToolbarState, content: Rect) -> Rect {
    let fit = fitted(bar, &groups_of(bar), 100_000.0);
    let size = vec2(extent(bar, &fit).max(GRIP), BAR);
    from_ratio([bar.x_ratio, bar.y_ratio, size.x / content.width(), size.y / content.height()], content, MIN_VISIBLE)
}

/// Floating tool bars, then the drag in progress (its preview and its drop).
pub fn show_floating(ctx: &egui::Context, app: &mut CadApp) {
    let content = ctx.content_rect();
    let bars: Vec<ToolbarState> = app.ws.applied.toolbars.iter().filter(|bar| bar.dock == Dock::Floating && bar.visible).cloned().collect();
    let t = Tokens::get();
    if content.is_positive() {
        for bar in &bars {
            let rect = floating_rect(bar, content);
            egui::Area::new(Id::new(("cc_toolbar_float", &bar.id)))
                .fixed_pos(rect.min)
                .order(egui::Order::Middle)
                .constrain(false)
                .movable(false)
                .show(ctx, |ui| {
                    ui.allocate_rect(rect, Sense::hover());
                    ui.painter().rect(rect, 3.0, t.chrome, Stroke::new(1.0, t.border), StrokeKind::Inside);
                    draw_bar(ui, app, bar, rect, Axis::Horizontal, 0.0, rect.width());
                });
        }
    }
    drag::tick(ctx, app, content);
    finish_frame(ctx);
}

/// Size of the bar `id` as drawn this frame, for the drag preview.
pub(super) fn drawn_size(geometry: &Geometry, id: &str) -> Vec2 {
    geometry.bars.iter().find(|bar| bar.id == id).map_or(vec2(160.0, BAR), |bar| bar.rect.size())
}
