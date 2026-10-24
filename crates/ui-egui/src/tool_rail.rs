//! The Tools rail at the left edge: one row per ribbon tab, opening that tab's commands beside it,
//! so the drawing tools stay a click away when the ribbon is minimized or showing another tab.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::CadApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::wsprofile::schema::{Density, RibbonTab};

pub(crate) const WIDTH: f32 = 150.0;
const HEADER_HEIGHT: f32 = 30.0;

fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 30.0,
        Density::Normal => 36.0,
        Density::Comfortable => 44.0,
    }
}

fn tab_icon(tab: &RibbonTab) -> Option<Icon> {
    let known = match tab.id.as_str() {
        "draw" => Some(Icon::Line),
        "modify" => Some(Icon::Move),
        "annotate" => Some(Icon::DimLinear),
        "layers" => Some(Icon::Layers),
        "insert" => Some(Icon::Insert),
        "properties" => Some(Icon::Properties),
        "view" => Some(Icon::Viewports),
        "layout" => Some(Icon::Layout),
        "output" => Some(Icon::Plot),
        "tools" => Some(Icon::Gear),
        _ => None,
    };
    known.or_else(|| {
        tab.groups
            .iter()
            .flat_map(|group| &group.commands)
            .find_map(|id| crate::toolbars::resolve(id).icon.or_else(|| crate::ribbon::icons::fallback(id)))
    })
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    // Home repeats the other tabs' most used commands, so it has no row of its own.
    let tabs: Vec<RibbonTab> = crate::ribbon::visible_tabs(app).into_iter().filter(|tab| tab.contextual.is_none() && tab.id != "home").collect();
    if tabs.is_empty() {
        return;
    }
    let row = row_height(app.ws.effective.layout.density);
    egui::Panel::left("cc_tool_rail")
        .exact_size(WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.cmd_border)))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
            ui.painter().rect_filled(header, 0.0, t.header);
            ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, t.border));
            ui.painter().text(pos2(header.left() + 10.0, header.center().y), Align2::LEFT_CENTER, crate::tl!("Tools"), crate::theme::body(), t.text);
            let close = Rect::from_center_size(pos2(header.right() - 14.0, header.center().y), vec2(18.0, 18.0));
            let close_response = ui.interact(close, ui.id().with("close"), Sense::click());
            if close_response.hovered() {
                ui.painter().rect_filled(close, 3.0, t.control_hover);
            }
            icons::paint(ui.painter(), close.shrink(3.0), Icon::Close, false);
            icons::describe_control(ui, &close_response, crate::tl!("Hide Tool Rail"), None, false);
            if close_response.on_hover_text(crate::tl!("Hide Tool Rail")).clicked() {
                if let Err(error) = app.run("ui.toggle.toolrail", json!({"on": false})) {
                    app.set_status(error);
                }
                return;
            }
            let body = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            egui::ScrollArea::vertical().id_salt("cc_tool_rail_scroll").auto_shrink([false, false]).show(&mut body_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for tab in &tabs {
                    item(app, ui, tab, row);
                }
            });
        });
}

fn item(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab, height: f32) {
    let t = Tokens::get();
    let label = crate::ribbon::render::tab_label(tab).to_owned();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let popup_id = response.id.with("tool_rail_flyout");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let fill = if open {
        t.select_bg
    } else if response.hovered() || response.has_focus() {
        t.control_hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, t.list_row));
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0));
    if let Some(icon) = tab_icon(tab) {
        icons::paint(ui.painter(), icon_rect, icon, false);
    }
    let text = if open { t.text } else { t.text_dim };
    let galley = ui.painter().layout(label.clone(), crate::theme::body(), text, rect.right() - icon_rect.right() - 22.0);
    ui.painter().galley(pos2(icon_rect.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, text);
    ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::CENTER_CENTER, "›", crate::theme::body(), t.text_faint);
    icons::describe_control(ui, &response, &label, Some(open), false);
    egui::Popup::menu(&response).id(popup_id).align(egui::RectAlign::RIGHT_START).gap(2.0).show(|ui| {
        crate::ribbon::render::command_list(app, ui, tab);
    });
}
