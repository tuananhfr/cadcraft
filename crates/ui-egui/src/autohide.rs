//! Auto-hide keeps placement in the profile and only the temporary peek in memory.

use craft_ui::docking::{Location, Placement, Zone};
use serde_json::{Value, json};

use crate::CadApp;
use crate::docking::Panel;
use crate::wsprofile::schema::{Dock, PanelMode, PanelState};

const EDGE: f32 = 28.0;
const CLOSE_DELAY: f64 = 0.5;

#[derive(Default)]
pub struct State {
    pub active: Option<Panel>,
    last_inside: Option<f64>,
    focus_owned: bool,
    area: Option<egui::Rect>,
}

pub fn can_change(app: &CadApp, panel: Panel) -> bool {
    let prefix = format!("/panels/{}", panel.id());
    crate::wsprofile::merge::check(&app.ws.effective, &prefix).is_ok() && crate::wsprofile::merge::check(&app.ws.effective, "/dock_tree").is_ok()
}

pub fn edge_panels(app: &CadApp) -> Vec<PanelState> {
    app.ws
        .effective
        .layout
        .panels
        .iter()
        .filter(|state| state.mode == PanelMode::AutoHide && Panel::parse(&state.id).is_ok_and(|panel| panel != Panel::Canvas))
        .cloned()
        .collect()
}

pub fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(id, "ui.panel.autohide" | "ui.panel.pin" | "ui.panel.peek") {
        return None;
    }
    Some(command(app, id, params))
}

fn command(app: &mut CadApp, id: &str, params: &Value) -> Result<Value, String> {
    let panel = Panel::parse(params.get("panel").and_then(Value::as_str).ok_or("panel is required")?)?;
    if panel == Panel::Canvas {
        return Err("The drawing cannot auto-hide".into());
    }
    if id == "ui.panel.peek" {
        if !edge_panels(app).iter().any(|state| state.id == panel.id()) {
            return Err("The panel is not in auto-hide mode".into());
        }
        if app.autohide.active != Some(panel) {
            app.autohide = State { active: Some(panel), ..State::default() };
        }
        return Ok(json!({"panel": panel.id()}));
    }
    let pin = id == "ui.panel.pin";
    if pin && app.ui.docking.layout.contains(&panel) && app.ui.docking.layout.floating.iter().all(|group| !group.panels.contains(&panel)) {
        return Ok(json!({"panel": panel.id(), "mode": "dock"}));
    }
    if !can_change(app, panel) {
        return Err(crate::wsprofile::LOCKED_MESSAGE.into());
    }
    let side: Option<Dock> = params.get("side").cloned().map(serde_json::from_value).transpose().map_err(|error| error.to_string())?;
    if side == Some(Dock::Floating) {
        return Err("Auto-hide needs a dock edge".into());
    }
    let before = app.ws.effective.layout.panels.iter().find(|state| state.id == panel.id());
    let default_side = if panel == Panel::ToolSets { Dock::Left } else { Dock::Right };
    let saved_side = before.and_then(|state| state.side).filter(|side| *side != Dock::Floating).unwrap_or(default_side);
    let dock = side.unwrap_or(saved_side);
    let mut workspace = app.ui.docking.clone();
    workspace.set_visible(panel, false)?;
    if side.is_some_and(|side| side != saved_side) || before.is_none_or(|state| state.side == Some(Dock::Floating)) {
        let zone = match dock {
            Dock::Left => Zone::Left,
            Dock::Top => Zone::Top,
            Dock::Bottom => Zone::Bottom,
            Dock::Right | Dock::Floating => Zone::Right,
        };
        workspace.hidden.retain(|(id, _)| *id != panel);
        workspace.hidden.push((
            panel,
            Location { floating: None, anchor: Some(Panel::Canvas), placement: Placement::Split(zone), size: None, stack: None, sibling: Vec::new() },
        ));
    }
    if pin {
        workspace.set_visible(panel, true)?;
    }
    let (panels, tree) = crate::wsprofile::apply::capture_docking(&workspace, app.ws.area, &app.ws.effective.layout.panels);
    crate::wsprofile::edit_layout(app, |layout| {
        layout.panels = panels;
        layout.dock_tree = Some(tree);
        let state = layout.panels.iter_mut().find(|state| state.id == panel.id()).ok_or("Panel placement is missing")?;
        state.side = Some(dock);
        state.mode = if pin { PanelMode::Dock } else { PanelMode::AutoHide };
        state.visible = true;
        Ok(())
    })?;
    app.autohide = State::default();
    Ok(json!({"panel": panel.id(), "mode": if id == "ui.panel.pin" { "dock" } else { "auto_hide" }}))
}

fn side(state: &PanelState) -> Dock {
    match state.side {
        Some(dock @ (Dock::Left | Dock::Top | Dock::Bottom | Dock::Right)) => dock,
        _ => Dock::Right,
    }
}

pub fn edges(app: &mut CadApp, ui: &mut egui::Ui) {
    let panels = edge_panels(app);
    let t = crate::theme::Tokens::get();
    for dock in [Dock::Top, Dock::Bottom, Dock::Left, Dock::Right] {
        let group: Vec<_> = panels.iter().filter(|state| side(state) == dock).collect();
        if group.is_empty() {
            continue;
        }
        let vertical = matches!(dock, Dock::Left | Dock::Right);
        let panel = match dock {
            Dock::Top => egui::Panel::top("cc_autohide_top"),
            Dock::Bottom => egui::Panel::bottom("cc_autohide_bottom"),
            Dock::Left => egui::Panel::left("cc_autohide_left"),
            Dock::Right | Dock::Floating => egui::Panel::right("cc_autohide_right"),
        };
        panel.exact_size(EDGE).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
            let layout = if vertical { egui::Layout::top_down(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Center) };
            ui.with_layout(layout, |ui| {
                egui::ScrollArea::new([!vertical, vertical]).id_salt(("auto-hide-edges", dock as u8)).show(ui, |ui| {
                    for state in group {
                        let Ok(panel) = Panel::parse(&state.id) else { continue };
                        let response = edge_button(ui, panel, vertical);
                        if response.hovered() || response.clicked() {
                            if (app.autohide.active != Some(panel) || response.clicked())
                                && let Err(error) = app.run("ui.panel.peek", json!({"panel": panel.id()}))
                            {
                                app.set_status(error);
                            }
                            app.autohide.last_inside = Some(ui.input(|input| input.time));
                        }
                    }
                });
            });
        });
    }
    app.autohide.area = Some(ui.available_rect_before_wrap());
}

fn edge_button(ui: &mut egui::Ui, panel: Panel, vertical: bool) -> egui::Response {
    let label = crate::i18n::t(panel.label());
    let accessible = crate::tf!("Show {panel}", panel = label);
    if !vertical {
        let response = ui.button(label);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &accessible));
        return response;
    }
    let t = crate::theme::Tokens::get();
    let galley = ui.painter().layout_no_wrap(label.to_string(), crate::theme::body(), t.text);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(EDGE, galley.size().x + 16.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &accessible));
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 2.0, t.tab_active);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let pos = egui::pos2(rect.center().x - galley.size().y * 0.5, rect.bottom() - 8.0);
    ui.painter().add(egui::epaint::TextShape::new(pos, galley, t.text).with_angle(-std::f32::consts::FRAC_PI_2));
    response
}

pub fn overlay(app: &mut CadApp, ctx: &egui::Context) {
    let Some(panel) = app.autohide.active else { return };
    let Some(state) = edge_panels(app).into_iter().find(|state| state.id == panel.id()) else {
        app.autohide = State::default();
        return;
    };
    let area = app.autohide.area.unwrap_or_else(|| ctx.content_rect());
    if !area.is_finite() || !area.is_positive() {
        return;
    }
    let vertical = matches!(side(&state), Dock::Left | Dock::Right);
    let size = if vertical {
        egui::vec2((area.width() * state.width_ratio).max(280.0).min(area.width()), area.height())
    } else {
        egui::vec2(area.width(), (area.height() * state.height_ratio).max(180.0).min(area.height()))
    };
    let pos = match side(&state) {
        Dock::Right => egui::pos2(area.right() - size.x, area.top()),
        Dock::Bottom => egui::pos2(area.left(), area.bottom() - size.y),
        _ => area.min,
    };
    let mut pin = false;
    let popup_before = ctx.any_popup_open();
    let shown =
        egui::Area::new(egui::Id::new(("cc_autohide_overlay", panel))).order(egui::Order::Foreground).fixed_pos(pos).movable(false).show(ctx, |ui| {
            egui::Frame::new().fill(crate::theme::Tokens::get().panel).show(ui, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::t(panel.label()));
                    let response = ui.add_enabled(can_change(app, panel), egui::Button::new(crate::i18n::t("Pin panel")));
                    pin = response.clicked();
                    response.on_disabled_hover_text(crate::i18n::t(crate::wsprofile::LOCKED_MESSAGE));
                });
                ui.separator();
                egui::ScrollArea::both().id_salt(("auto-hide-body", panel)).show(ui, |ui| crate::docking::body(app, ui, panel));
            });
        });
    if pin {
        if let Err(error) = app.run("ui.panel.pin", json!({"panel": panel.id()})) {
            app.set_status(error);
        }
        return;
    }
    let now = ctx.input(|input| input.time);
    if ctx.pointer_hover_pos().is_some_and(|pos| shown.response.rect.contains(pos)) {
        app.autohide.last_inside = Some(now);
    }
    let owns_focus = ctx
        .memory(|memory| memory.focused())
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|response| response.layer_id == shown.response.layer_id);
    // Popups render on their own layer; those created by this body still belong to its peek.
    let body_popup = !popup_before && ctx.any_popup_open();
    let editing = owns_focus && ctx.text_edit_focused();
    let focus_left = app.autohide.focus_owned && !owns_focus && !body_popup;
    if !body_popup {
        app.autohide.focus_owned = owns_focus;
    }
    if body_popup || editing {
        app.autohide.last_inside = Some(now);
    }
    let last = *app.autohide.last_inside.get_or_insert(now);
    let dismissed = ctx.input(|input| {
        (input.key_pressed(egui::Key::Escape) && !body_popup) || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
    });
    if focus_left || (!body_popup && !editing && now - last >= CLOSE_DELAY) || dismissed {
        app.autohide = State::default();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
