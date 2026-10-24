//! The title row's own parts outside the integrated macOS title bar: the product mark on the left
//! and the menu, Options and Help buttons on the right.

use egui::{Align, Align2, Color32, FontId, Layout, Rect, UiBuilder, pos2, vec2};

use crate::CadApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::wsprofile::schema::Density;

/// Below this window height a host's [`CadApp::auto_hide_menu_bar`] gives the menu bar's row to
/// the drawing; its menus stay one click away behind the title row's menu button.
pub(crate) const MENU_BAR_MIN_HEIGHT: f32 = 760.0;

/// Window height from which the Normal-density title row takes its full height.
const TALL_WINDOW: f32 = 800.0;

pub(crate) fn height(density: Density, window_height: f32) -> f32 {
    match density {
        Density::Compact => 28.0,
        Density::Normal if window_height < TALL_WINDOW => 28.0,
        Density::Normal => 36.0,
        Density::Comfortable => 40.0,
    }
}

pub(crate) fn menu_bar_hidden(app: &CadApp, ctx: &egui::Context) -> bool {
    app.auto_hide_menu_bar && app.ui.in_window_menu && ctx.content_rect().height() < MENU_BAR_MIN_HEIGHT
}

/// Paints the product mark from `left`, centred in `row`; returns its right edge.
pub(crate) fn brand(ui: &egui::Ui, row: Rect, left: f32) -> f32 {
    let t = Tokens::get();
    let side = (row.height() - 14.0).clamp(16.0, 22.0);
    let mark = Rect::from_min_size(pos2(left, row.center().y - side / 2.0), vec2(side, side));
    let painter = ui.painter();
    painter.rect_filled(mark, 4.0, t.accent);
    painter.text(mark.center(), Align2::CENTER_CENTER, "U", FontId::proportional(side * 0.72), Color32::WHITE);
    let galley = painter.layout_no_wrap("UbotCAD".to_owned(), FontId::proportional(14.0), t.text);
    let at = pos2(mark.right() + 7.0, row.center().y - galley.size().y / 2.0);
    let right = at.x + galley.size().x;
    painter.galley(at, galley, t.text);
    right
}

/// Lays out the right-hand buttons inside `row`; returns the left edge they leave free.
pub(crate) fn trailing(app: &mut CadApp, ui: &mut egui::Ui, row: Rect) -> f32 {
    let size = (row.height() - 8.0).clamp(18.0, 32.0);
    let mut buttons = ui.new_child(UiBuilder::new().max_rect(row.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    buttons.spacing_mut().item_spacing.x = 4.0;
    let mut chosen = None;
    if icons::button(&mut buttons, Icon::Help, size, crate::tl!("Help"), false).clicked() {
        chosen = Some("ui.dialog.commands");
    }
    if icons::button(&mut buttons, Icon::Gear, size, crate::tl!("Options"), false).clicked() {
        chosen = Some("options");
    }
    if menu_bar_hidden(app, ui.ctx()) {
        crate::menus::menu_button(app, &mut buttons, size);
    }
    if let Some(id) = chosen {
        crate::menus::activate(app, id);
    }
    buttons.min_rect().left() - 8.0
}
