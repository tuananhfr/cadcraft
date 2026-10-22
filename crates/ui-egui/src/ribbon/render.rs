//! Responsive ribbon groups collapse before the remaining groups need scrolling.

use egui::{Align, Id, Key, Layout, Modifiers, vec2};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;
use crate::wsprofile::schema::{Density, RibbonGroup, RibbonMode, RibbonTab};

fn preset_tabs() -> &'static [RibbonTab] {
    static TABS: std::sync::OnceLock<Vec<RibbonTab>> = std::sync::OnceLock::new();
    TABS.get_or_init(|| {
        crate::wsprofile::presets::PRESET_IDS
            .iter()
            .filter_map(|id| crate::wsprofile::presets::preset(id))
            .flat_map(|preset| preset.layout.ribbon.tabs)
            .collect()
    })
}

// Matching both identity and the shipped label keeps custom and renamed labels as user data.
pub(crate) fn tab_label(tab: &RibbonTab) -> &str {
    if preset_tabs().iter().any(|preset| preset.id == tab.id && preset.label == tab.label) { crate::i18n::ribbon(&tab.label) } else { &tab.label }
}

pub(crate) fn group_label<'a>(tab: &RibbonTab, group: &'a RibbonGroup) -> &'a str {
    let builtin = preset_tabs()
        .iter()
        .filter(|preset| preset.id == tab.id)
        .any(|preset| preset.groups.iter().any(|built| built.id == group.id && built.label == group.label));
    if builtin { crate::i18n::ribbon(&group.label) } else { &group.label }
}

pub fn rendered_mode(app: &CadApp, available_height: f32) -> RibbonMode {
    let metrics = metrics(app.ws.effective.layout.density);
    let preferred = app.ws.effective.layout.ribbon.effective_mode();
    if matches!(preferred, RibbonMode::Hidden | RibbonMode::TabsOnly) {
        return preferred;
    }
    if !available_height.is_finite() || available_height <= 0.0 {
        return RibbonMode::TabsOnly;
    }
    // Drafting keeps most of the remaining height; the saved preference survives resizing.
    let mut reserved = crate::cmdline::MIN_CANVAS_HEIGHT.max(available_height * 0.6)
        + crate::toolbars::render::docked_height(app, crate::wsprofile::schema::Dock::Bottom);
    if app.ui.show_status_bar {
        reserved += crate::chrome::STATUS_BAR_HEIGHT;
    }
    if app.ui.show_file_tabs {
        reserved += crate::chrome::file_tabs_height(app.ws.effective.layout.density);
    }
    if app.ui.show_command_line && app.ws.effective.layout.command_line.dock != crate::wsprofile::schema::Dock::Floating {
        reserved += crate::cmdline::docked_height(app, available_height);
    }
    if preferred == RibbonMode::Expanded && available_height >= metrics.expanded_height + reserved {
        RibbonMode::Expanded
    } else if available_height >= metrics.compact_height + reserved {
        RibbonMode::Compact
    } else {
        RibbonMode::TabsOnly
    }
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let metrics = metrics(app.ws.effective.layout.density);
    let mode = rendered_mode(app, ui.available_height());
    let popup_id = Id::new("cc_ribbon_flyout");
    if mode != RibbonMode::TabsOnly {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    let tabs = super::visible_tabs(app);
    // A workspace without ribbon tabs (Simple) has nothing to restore, so it keeps no hidden strip.
    if mode == RibbonMode::Hidden && tabs.is_empty() {
        ui.ctx().data_mut(|data| data.remove::<Vec<Id>>(Id::new("cc_ribbon_menu_stops")));
        return;
    }
    let state_id = Id::new("cc_ribbon_active");
    let mut active: String = ui.ctx().data(|data| data.get_temp(state_id)).unwrap_or_default();
    let context_id = Id::new("cc_ribbon_context");
    let previous_context: String = ui.ctx().data(|data| data.get_temp(context_id)).unwrap_or_default();
    let context = tabs.iter().find(|tab| tab.contextual.is_some()).map(|tab| tab.id.clone()).unwrap_or_default();
    // A new context exposes its tools without overriding a later manual tab choice.
    if !context.is_empty() && context != previous_context {
        active = context.clone();
    }
    ui.ctx().data_mut(|data| data.insert_temp(context_id, context));
    if !tabs.iter().any(|tab| tab.id == active) {
        egui::Popup::close_id(ui.ctx(), popup_id);
        active = tabs.first().map(|tab| tab.id.clone()).unwrap_or_default();
    }
    let height = match mode {
        RibbonMode::Expanded => metrics.expanded_height,
        RibbonMode::Compact => metrics.compact_height,
        RibbonMode::TabsOnly => metrics.tab_height,
        RibbonMode::Hidden => 6.0,
    };
    let panel = egui::Panel::top("cc_ribbon").exact_size(height).frame(egui::Frame::NONE.fill(Tokens::get().chrome)).show(ui, |ui| {
        if mode == RibbonMode::Hidden {
            ui.ctx().data_mut(|data| data.remove::<Vec<Id>>(Id::new("cc_ribbon_menu_stops")));
            return;
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let background = ui.interact(ui.max_rect(), ui.id().with("ribbon_context"), egui::Sense::click());
        ui.ctx().data_mut(|data| data.insert_temp(state_id, active.clone()));
        let mut open = None;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), metrics.tab_height), Layout::left_to_right(Align::Center), |ui| {
            if let Some(index) = tab_strip(ui, app, &(0..tabs.len()).collect::<Vec<_>>())
                && let Some(tab) = tabs.get(index)
            {
                open = Some(egui::SetOpenCommand::Bool(active != tab.id || !egui::Popup::is_id_open(ui.ctx(), popup_id)));
                active = tab.id.clone();
            }
        });
        let ids: Vec<(usize, Id)> = ui.ctx().data(|data| data.get_temp(Id::new("cc_ribbon_tab_responses"))).unwrap_or_default();
        let responses: Vec<_> =
            (0..tabs.len()).map(|index| ids.iter().find(|(held, _)| *held == index).and_then(|(_, id)| ui.ctx().read_response(*id))).collect();
        if responses.iter().flatten().any(egui::Response::double_clicked) {
            run(app, "ui.ribbon.collapse", json!({"collapsed": mode != RibbonMode::TabsOnly}));
            open = Some(egui::SetOpenCommand::Bool(false));
        }
        let mut stops: Vec<_> = ids.iter().map(|(_, id)| *id).collect();
        stops.push(background.id);
        ui.ctx().data_mut(|data| data.insert_temp(Id::new("cc_ribbon_menu_stops"), stops));
        if !app.cmd.ime_composing() && !ui.ctx().text_edit_focused() {
            let focused = responses.iter().position(|response| response.as_ref().is_some_and(egui::Response::has_focus));
            let navigation =
                focused.or_else(|| if egui::Popup::is_id_open(ui.ctx(), popup_id) { tabs.iter().position(|tab| tab.id == active) } else { None });
            if let Some(index) = navigation {
                let direction = ui.input_mut(|input| {
                    if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                        1
                    } else if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                        -1
                    } else {
                        0
                    }
                });
                let next = match direction {
                    1 => index.checked_add(1).filter(|next| *next < tabs.len()).unwrap_or(0),
                    -1 => index.checked_sub(1).unwrap_or_else(|| tabs.len().saturating_sub(1)),
                    _ => index,
                };
                if direction != 0
                    && let Some(tab) = tabs.get(next)
                {
                    active = tab.id.clone();
                    ui.ctx().data_mut(|data| data.insert_temp(Id::new("cc_ribbon_focus_tab"), tab.id.clone()));
                    ui.ctx().request_repaint();
                    if let Some(response) = responses.get(next).and_then(Option::as_ref) {
                        response.request_focus();
                    }
                    // egui would otherwise move focus spatially at the end of the pass and override the tab change.
                    ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                }
                if focused.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter) || input.consume_key(Modifiers::NONE, Key::Space))
                {
                    open = Some(egui::SetOpenCommand::Bool(true));
                }
            }
        }
        if mode == RibbonMode::TabsOnly {
            match open {
                Some(egui::SetOpenCommand::Bool(true)) => egui::Popup::open_id(ui.ctx(), popup_id),
                Some(egui::SetOpenCommand::Bool(false)) => egui::Popup::close_id(ui.ctx(), popup_id),
                _ => {}
            }
        }
        if let Some(index) = tabs.iter().position(|tab| tab.id == active) {
            match mode {
                RibbonMode::Expanded => {
                    if let Some(tab) = tabs.get(index) {
                        groups(app, ui, tab);
                    }
                }
                RibbonMode::Compact => {
                    if let Some(tab) = tabs.get(index) {
                        compact_groups(app, ui, tab);
                    }
                }
                RibbonMode::TabsOnly => {
                    if let Some(response) = responses.get(index).and_then(Option::as_ref) {
                        let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
                        let screen = ui.ctx().content_rect();
                        let below = (screen.bottom() - response.rect.bottom()).max(1.0);
                        let width = screen.width().min(1000.0);
                        let frame = egui::Frame::popup(ui.style());
                        let margin = frame.total_margin().sum();
                        let content_width = (width - margin.x).max(1.0);
                        // Popup width excludes its frame, and a rightmost tab still needs room for the whole flyout.
                        let left = response.rect.left().clamp(screen.left(), (screen.right() - width).max(screen.left()));
                        let close_behavior = if matches!(open, Some(egui::SetOpenCommand::Bool(true))) {
                            egui::PopupCloseBehavior::IgnoreClicks
                        } else {
                            egui::PopupCloseBehavior::CloseOnClickOutside
                        };
                        egui::Popup::from_response(response)
                            .id(popup_id)
                            .open_memory(open)
                            .at_position(egui::pos2(left, response.rect.bottom()))
                            .align(egui::RectAlign::BOTTOM_START)
                            .align_alternatives(&[])
                            .gap(0.0)
                            .width(content_width)
                            .frame(frame)
                            .close_behavior(close_behavior)
                            .show(|ui| {
                                ui.set_width(content_width);
                                ui.set_max_height((below - margin.y).max(1.0));
                                flyout(ui, app, index);
                            });
                        if was_open && !egui::Popup::is_id_open(ui.ctx(), popup_id) && ui.input(|input| input.key_pressed(Key::Escape)) {
                            response.request_focus();
                        }
                    }
                }
                RibbonMode::Hidden => {}
            }
        }
        ribbon_context_menu(ui, app, &background, None);
    });
    if mode != RibbonMode::Hidden && !active.is_empty() {
        let responses: Vec<(usize, Id)> = ui.ctx().data(|data| data.get_temp(Id::new("cc_ribbon_tab_responses"))).unwrap_or_default();
        let targets = responses
            .iter()
            .filter_map(|(index, id)| {
                let tab = tabs.get(*index)?;
                let response = ui.ctx().read_response(*id)?;
                Some((response.rect, tab.id.clone()))
            })
            .collect();
        crate::toolbars::render::ribbon_drop_zone(ui.ctx(), app, panel.response.rect, &active, targets);
    }
    ui.ctx().data_mut(|data| data.insert_temp(state_id, active));
}

pub(crate) fn claim_menu_key(app: &CadApp, ctx: &egui::Context) {
    if app.cmd.ime_composing() || ctx.text_edit_focused() || app.ui.dialog.is_some() {
        return;
    }
    let Some(focused) = ctx.memory(|memory| memory.focused()) else { return };
    let stops: Vec<Id> = ctx.data(|data| data.get_temp(Id::new("cc_ribbon_menu_stops"))).unwrap_or_default();
    // Consume before global shortcuts so Shift+F10 cannot also toggle Polar Tracking.
    if stops.contains(&focused) && ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::F10)) {
        ctx.data_mut(|data| data.insert_temp(Id::new("cc_ribbon_menu_key"), focused));
    }
}

fn tab_strip(ui: &mut egui::Ui, app: &mut CadApp, tabs: &[usize]) -> Option<usize> {
    let metrics = metrics(app.ws.effective.layout.density);
    let visible = super::visible_tabs(app);
    let active: String = ui.ctx().data(|data| data.get_temp(Id::new("cc_ribbon_active"))).unwrap_or_default();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let gap = ui.spacing().item_spacing.x;
    let padding = ui.spacing().button_padding.x * 2.0;
    let comfortable = app.ws.effective.layout.density == Density::Comfortable;
    let width = |label: &str| {
        let width = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Tokens::get().text).size().x + padding;
        if comfortable { width.max(32.0) } else { width }
    };
    let widths: Vec<_> = tabs.iter().filter_map(|index| visible.get(*index).map(|tab| (*index, width(tab_label(tab))))).collect();
    let available = ui.available_width().max(0.0);
    let overflow = widths.iter().map(|(_, width)| width + gap).sum::<f32>() - gap > available;
    let budget = (available - if overflow { width(crate::tl!("»")).max(metrics.tab_height) + gap } else { 0.0 }).max(0.0);
    let selected = visible.iter().position(|tab| tab.id == active);
    let mut shown = Vec::new();
    let mut remaining = budget;
    // Reserve the active tab before filling the strip, including after an overflow selection.
    if let Some((index, width)) = widths.iter().find(|(index, _)| Some(*index) == selected) {
        shown.push(*index);
        remaining -= width.min(budget) + gap;
    }
    for (index, width) in &widths {
        if Some(*index) != selected && *width <= remaining {
            shown.push(*index);
            remaining -= width + gap;
        }
    }
    let focus: Option<String> = ui.ctx().data_mut(|data| data.remove_temp(Id::new("cc_ribbon_focus_tab")));
    let mut responses = Vec::new();
    let mut chosen = None;
    for (index, width) in &widths {
        let Some(tab) = visible.get(*index).filter(|_| shown.contains(index)) else { continue };
        let response = ui
            .push_id(&tab.id, |ui| {
                ui.add_sized(vec2(width.min(budget), metrics.tab_height), egui::Button::selectable(active == tab.id, tab_label(tab)).truncate())
            })
            .inner;
        crate::icons::describe_control(ui, &response, tab_label(tab), Some(active == tab.id), true);
        if focus.as_deref() == Some(tab.id.as_str()) {
            response.request_focus();
        }
        if response.clicked() {
            chosen = Some(*index);
            response.request_focus();
        }
        responses.push((*index, response.id));
        ribbon_context_menu(ui, app, &response, Some(&tab.id));
    }
    if overflow {
        egui::containers::menu::MenuButton::from_button(egui::Button::new(crate::tl!("»")).min_size(if comfortable {
            vec2(32.0, metrics.tab_height)
        } else {
            egui::Vec2::ZERO
        }))
        .ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.75).show(ui, |ui| {
                for index in tabs.iter().filter(|index| !shown.contains(index)) {
                    let Some(tab) = visible.get(*index) else { continue };
                    if ui.button(tab_label(tab)).clicked() {
                        chosen = Some(*index);
                        ui.ctx().data_mut(|data| data.insert_temp(Id::new("cc_ribbon_focus_tab"), tab.id.clone()));
                        ui.close();
                    }
                }
            });
        });
    }
    ui.ctx().data_mut(|data| data.insert_temp(Id::new("cc_ribbon_tab_responses"), responses));
    chosen
}

fn ribbon_context_menu(ui: &mut egui::Ui, app: &mut CadApp, anchor: &egui::Response, tab: Option<&str>) {
    let claimed = ui.ctx().data(|data| data.get_temp::<Id>(Id::new("cc_ribbon_menu_key"))) == Some(anchor.id);
    let keyboard = claimed || (anchor.has_focus() && !app.cmd.ime_composing() && ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::F10)));
    if claimed {
        ui.ctx().data_mut(|data| data.remove::<Id>(Id::new("cc_ribbon_menu_key")));
    }
    let popup_id = anchor.id.with("tab_context");
    let open = keyboard || anchor.secondary_clicked();
    if open {
        let position = if keyboard { anchor.rect.left_bottom() } else { ui.ctx().pointer_interact_pos().unwrap_or(anchor.rect.left_bottom()) };
        ui.ctx().data_mut(|data| data.insert_temp(popup_id.with("position"), position));
    }
    // Keep the anchor after the opening frame, including when the menu was opened by keyboard.
    let position = ui.ctx().data(|data| data.get_temp::<egui::Pos2>(popup_id.with("position"))).unwrap_or(anchor.rect.left_bottom());
    let mut chosen = None;
    egui::Popup::menu(anchor).id(popup_id).at_position(position).open_memory(open.then_some(egui::SetOpenCommand::Bool(true))).show(|ui| {
        let customize = ui.button(crate::tl!("Customize Ribbon…"));
        if keyboard {
            customize.request_focus();
        }
        if customize.clicked() {
            chosen = Some(("ui.customize.open", json!({})));
            ui.close();
        }
        ui.menu_button(crate::tl!("Display Mode"), |ui| {
            let current = app.ws.effective.layout.ribbon.effective_mode();
            for (label, mode, value) in [
                (crate::tl!("Expanded"), RibbonMode::Expanded, "expanded"),
                (crate::tl!("Compact"), RibbonMode::Compact, "compact"),
                (crate::tl!("Tabs Only"), RibbonMode::TabsOnly, "tabs_only"),
                (crate::tl!("Hidden"), RibbonMode::Hidden, "hidden"),
            ] {
                if ui.radio(current == mode, label).clicked() {
                    chosen = Some(("ui.ribbon.mode", json!({"mode": value})));
                    ui.close();
                }
            }
        });
        if ui.add_enabled(tab.is_some(), egui::Button::new(crate::tl!("Hide Tab"))).clicked() {
            chosen = Some(("ui.ribbon.tab.visible", json!({"tab": tab, "visible": false})));
            ui.close();
        }
        ui.menu_button(crate::tl!("Show Tabs"), |ui| {
            let mut tabs = app.ws.effective.layout.ribbon.tabs.clone();
            tabs.sort_by_key(|tab| tab.order);
            egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.75).show(ui, |ui| {
                for tab in tabs {
                    let mut visible = tab.visible;
                    if ui.checkbox(&mut visible, tab_label(&tab)).changed() {
                        chosen = Some(("ui.ribbon.tab.visible", json!({"tab": tab.id, "visible": visible})));
                        ui.close();
                    }
                }
            });
        });
        if ui.button(crate::tl!("Reset Ribbon")).clicked() {
            chosen = Some(("ui.ribbon.reset", json!({})));
            ui.close();
        }
    });
    if let Some((command, params)) = chosen {
        run(app, command, params);
    }
}

pub(crate) fn flyout(ui: &mut egui::Ui, app: &mut CadApp, tab: usize) {
    let Some(tab) = super::visible_tabs(app).get(tab).cloned() else { return };
    egui::ScrollArea::vertical()
        .id_salt((&tab.id, "flyout"))
        .min_scrolled_height(1.0)
        .max_width(ui.available_width())
        .max_height(ui.available_height())
        .show(ui, |ui| {
            let available = ui.available_width();
            ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), |ui| {
                ui.spacing_mut().item_spacing = vec2(GROUP_GAP, GROUP_GAP);
                for group in &tab.groups {
                    let columns = column_widths(ui, group);
                    expanded_group(app, ui, &tab, group, &columns, group_width(&columns).min(available));
                }
            });
        });
}

fn compact_groups(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab) {
    let metrics = metrics(app.ws.effective.layout.density);
    let state_id = Id::new(("cc_ribbon_compact_scroll", &tab.id));
    let mut offset: f32 = ui.ctx().data(|data| data.get_temp(state_id)).unwrap_or_default();
    let gap = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - 2.0 * (metrics.scroll_width + gap)).max(1.0);
    let step = (width - COMPACT_TEXT_WIDTH).max(1.0);
    ui.add_space(GROUP_PADDING);
    ui.horizontal(|ui| {
        if scroll_button(ui, "‹", crate::tl!("Previous ribbon groups"), offset > 0.5).clicked() {
            offset = (offset - step).max(0.0);
        }
        let scroll = ui
            .allocate_ui_with_layout(vec2(width, metrics.button_height), Layout::left_to_right(Align::Center), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt((&tab.id, "compact"))
                    .horizontal_scroll_offset(offset)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, false])
                    .max_width(width)
                    .max_height(metrics.button_height)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (index, group) in tab.groups.iter().enumerate() {
                                if index > 0 {
                                    ui.separator();
                                }
                                ui.push_id((&tab.id, &group.id), |ui| {
                                    ui.horizontal(|ui| {
                                        for id in &group.commands {
                                            button(app, ui, tab, group, id, ButtonStyle::Compact);
                                        }
                                    });
                                });
                            }
                        });
                    })
            })
            .inner;
        offset = scroll.state.offset.x;
        let max_offset = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
        if scroll_button(ui, "›", crate::tl!("Next ribbon groups"), offset < max_offset - 0.5).clicked() {
            offset = (offset + step).min(max_offset);
            ui.ctx().request_repaint();
        }
        ui.ctx().data_mut(|data| data.insert_temp(state_id, offset));
    });
}

fn run(app: &mut CadApp, command: &str, params: serde_json::Value) {
    if let Err(error) = app.run(command, params) {
        app.set_status(error);
    }
}

fn scroll_button(ui: &mut egui::Ui, glyph: &str, label: &str, enabled: bool) -> egui::Response {
    let metrics = metrics(crate::theme::density(ui.ctx()));
    let response = ui.add_enabled_ui(enabled, |ui| ui.add_sized(vec2(metrics.scroll_width, metrics.button_height), egui::Button::new(glyph))).inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.on_hover_text(label)
}

const MAX_TEXT_WIDTH: f32 = 160.0;
const COMPACT_TEXT_WIDTH: f32 = 120.0;
const COLUMN_GAP: f32 = 6.0;
const ROW_GAP: f32 = 4.0;
const ROWS: usize = 3;
const LABEL_HEIGHT: f32 = 18.0;
const GROUP_PADDING: f32 = 6.0;
/// Normal density's height remains available to existing geometry clients.
pub const COMPACT_HEIGHT: f32 = 64.0;

struct RibbonMetrics {
    tab_height: f32,
    button_height: f32,
    column_width: f32,
    columns_height: f32,
    group_height: f32,
    compact_height: f32,
    expanded_height: f32,
    scroll_width: f32,
}

fn metrics(density: Density) -> RibbonMetrics {
    let metrics = density.metrics();
    let columns_height = ROWS as f32 * metrics.ribbon_button + (ROWS - 1) as f32 * ROW_GAP;
    let group_height = columns_height + ROW_GAP + LABEL_HEIGHT;
    RibbonMetrics {
        tab_height: metrics.tab_height,
        button_height: metrics.ribbon_button,
        column_width: (metrics.ribbon_button + 4.0).max(28.0),
        columns_height,
        group_height,
        compact_height: metrics.tab_height + metrics.ribbon_button + 2.0 * GROUP_PADDING,
        expanded_height: metrics.tab_height + group_height + 2.0 * GROUP_PADDING,
        scroll_width: metrics.ribbon_button.max(24.0),
    }
}

const GROUP_GAP: f32 = 8.0;
const MIN_GROUP_WIDTH: f32 = 90.0;
const COMPACT_WIDTH: f32 = 76.0;

fn resolve(id: &str) -> crate::toolbars::Command {
    let mut command = crate::toolbars::resolve(id);
    if let Some(spec) = cadcraft_engine::find_command(id) {
        command.label = spec.label.to_string();
    }
    command
}

fn label(command: &crate::toolbars::Command) -> String {
    if command.known { crate::i18n::t(&command.label).to_string() } else { crate::toolbars::placeholder_label(command) }
}

fn column_widths(ui: &egui::Ui, group: &RibbonGroup) -> Vec<f32> {
    let metrics = metrics(crate::theme::density(ui.ctx()));
    let font = egui::TextStyle::Button.resolve(ui.style());
    group
        .commands
        .chunks(ROWS)
        .map(|commands| {
            commands.iter().fold(metrics.column_width, |width, id| {
                let command = resolve(id);
                if command.icon.is_some() {
                    return width;
                }
                // egui caches galleys by text and font, including changes of locale or font size.
                let text = ui.painter().layout_no_wrap(label(&command), font.clone(), ui.visuals().text_color());
                width.max((text.size().x + 2.0 * ui.spacing().button_padding.x).ceil().min(MAX_TEXT_WIDTH))
            })
        })
        .collect()
}

fn group_width(columns: &[f32]) -> f32 {
    (columns.iter().sum::<f32>() + columns.len().saturating_sub(1) as f32 * COLUMN_GAP + 2.0 * GROUP_PADDING).max(MIN_GROUP_WIDTH)
}

fn groups(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab) {
    let metrics = metrics(app.ws.effective.layout.density);
    let available = ui.available_width().max(0.0);
    let columns: Vec<_> = tab.groups.iter().map(|group| column_widths(ui, group)).collect();
    let widths: Vec<_> = columns.iter().map(|columns| group_width(columns)).collect();
    let mut collapsed = vec![false; widths.len()];
    let mut total = widths.iter().sum::<f32>() + widths.len().saturating_sub(1) as f32 * GROUP_GAP;
    for (index, width) in widths.iter().enumerate().rev() {
        if total <= available {
            break;
        }
        if let Some(slot) = collapsed.get_mut(index) {
            *slot = true;
        }
        total -= width - COMPACT_WIDTH;
    }
    let scroll = total > available;
    let scroll_id = Id::new(("cc_ribbon_scroll", &tab.id));
    let mut start: usize = ui.ctx().data(|data| data.get_temp(scroll_id)).unwrap_or(0);
    start = start.min(tab.groups.len().saturating_sub(1));
    if !scroll {
        start = 0;
    }
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GROUP_GAP;
        if scroll && scroll_button(ui, "‹", crate::tl!("Previous ribbon groups"), start > 0).clicked() {
            start = start.saturating_sub(1);
        }
        let mut remaining = (available - if scroll { 2.0 * (metrics.scroll_width + GROUP_GAP) } else { 0.0 }).max(0.0);
        let mut end = start;
        for (index, group) in tab.groups.iter().enumerate().skip(start) {
            let compact = collapsed.get(index).copied().unwrap_or(true);
            let width = if compact { COMPACT_WIDTH } else { widths.get(index).copied().unwrap_or(MIN_GROUP_WIDTH) };
            if remaining < width {
                break;
            }
            ui.push_id((&tab.id, &group.id), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(width, metrics.group_height), egui::Sense::hover());
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.set_min_size(vec2(width, metrics.group_height));
                    ui.spacing_mut().item_spacing.y = ROW_GAP;
                    if compact {
                        egui::containers::menu::MenuButton::from_button(
                            egui::Button::new(group_label(tab, group)).truncate().min_size(vec2(COMPACT_WIDTH, metrics.button_height)),
                        )
                        .ui(ui, |ui| {
                            for id in &group.commands {
                                button(app, ui, tab, group, id, ButtonStyle::Label);
                            }
                        });
                    } else {
                        expanded_group(app, ui, tab, group, columns.get(index).map(Vec::as_slice).unwrap_or_default(), width);
                    }
                });
            });
            remaining -= width + GROUP_GAP;
            end = index.saturating_add(1);
        }
        if scroll && scroll_button(ui, "›", crate::tl!("Next ribbon groups"), end < tab.groups.len()).clicked() {
            start = start.saturating_add(1).min(tab.groups.len().saturating_sub(1));
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(scroll_id, start));
}

fn expanded_group(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab, group: &RibbonGroup, columns: &[f32], width: f32) {
    let metrics = metrics(app.ws.effective.layout.density);
    let inner_width = (width - 2.0 * GROUP_PADDING).max(metrics.column_width);
    let mut left = 0.0;
    let mut top = 0.0;
    // Custom groups may be wider than the screen; wrap their columns as well as whole groups.
    let positions: Vec<_> = columns
        .iter()
        .map(|column| {
            let column = column.min(inner_width);
            if left > 0.0 && left + column > inner_width {
                left = 0.0;
                top += metrics.columns_height + ROW_GAP;
            }
            let position = (left, top, column);
            left += column + COLUMN_GAP;
            position
        })
        .collect();
    let height = top + metrics.columns_height;
    // Allocate in the wrapping parent before an identity scope resets the row's left edge.
    ui.allocate_ui_with_layout(vec2(width, height + ROW_GAP + LABEL_HEIGHT), Layout::top_down(Align::Min), |ui| {
        ui.push_id((&tab.id, &group.id), |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
            for (column, commands) in group.commands.chunks(ROWS).enumerate() {
                let (left, top, column_width) = positions.get(column).copied().unwrap_or((0.0, 0.0, metrics.column_width));
                let rect = egui::Rect::from_min_size(rect.min + vec2(GROUP_PADDING + left, top), vec2(column_width, metrics.columns_height));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.set_min_size(vec2(column_width, metrics.columns_height));
                    for id in commands {
                        button(app, ui, tab, group, id, ButtonStyle::Expanded);
                    }
                });
            }
            ui.add_sized(vec2(width, LABEL_HEIGHT), egui::Label::new(group_label(tab, group)).truncate());
        });
    });
}

#[derive(Clone, Copy)]
enum ButtonStyle {
    Expanded,
    Compact,
    Label,
}

fn button(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab, group: &RibbonGroup, id: &str, style: ButtonStyle) {
    let metrics = metrics(app.ws.effective.layout.density);
    ui.push_id(id, |ui| {
        let command = resolve(id);
        let label = label(&command);
        let enabled = command.known && cadcraft_engine::find_command(id).is_none_or(|spec| (spec.enabled)(&app.session).is_ok());
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                if matches!(style, ButtonStyle::Compact | ButtonStyle::Expanded)
                    && let Some(icon) = command.icon
                {
                    crate::icons::button(ui, icon, metrics.button_height, &label, false)
                } else {
                    ui.add_sized(
                        if matches!(style, ButtonStyle::Expanded) {
                            vec2(ui.available_width().min(MAX_TEXT_WIDTH), metrics.button_height)
                        } else if matches!(style, ButtonStyle::Compact) {
                            vec2(COMPACT_TEXT_WIDTH, metrics.button_height)
                        } else {
                            vec2(140.0, metrics.button_height)
                        },
                        egui::Button::new(label.as_str()).truncate(),
                    )
                    .on_hover_text(label.as_str())
                }
            })
            .inner;
        let response = if command.known {
            response.on_disabled_hover_text(label.as_str())
        } else {
            response.on_disabled_hover_text(crate::toolbars::placeholder_reason(&command))
        };
        crate::control::record_widget(ui, &response, &label, "button", None);
        if response.has_focus() {
            response.scroll_to_me(None);
        }
        if response.clicked() {
            crate::menus::activate(app, id);
            egui::Popup::close_id(ui.ctx(), Id::new("cc_ribbon_flyout"));
        }
        let keyboard = response.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::F10));
        let open = (keyboard || response.secondary_clicked()).then_some(egui::SetOpenCommand::Bool(true));
        egui::Popup::menu(&response).id(response.id.with("ribbon_menu")).open_memory(open).show(|ui| {
            let pin = ui.button(crate::tl!("Add to Quick Access"));
            if keyboard {
                pin.request_focus();
            }
            if pin.clicked() {
                run(app, "ui.quickaccess.pin", json!({"id": id}));
                ui.close();
            }
            if ui.button(crate::tl!("Remove from Ribbon")).clicked() {
                run(app, "ui.ribbon.button.remove", json!({"tab": tab.id, "group": group.id, "id": id}));
                ui.close();
            }
        });
    });
}
