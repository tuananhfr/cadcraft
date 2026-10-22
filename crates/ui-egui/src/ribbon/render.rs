//! Responsive ribbon groups collapse before the remaining groups need scrolling.

use egui::{Align, Id, Key, Layout, Modifiers, vec2};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;
use crate::wsprofile::schema::{RibbonGroup, RibbonMode, RibbonTab};

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
        reserved += crate::chrome::FILE_TABS_HEIGHT;
    }
    if app.ui.show_command_line && app.ws.effective.layout.command_line.dock != crate::wsprofile::schema::Dock::Floating {
        reserved += crate::cmdline::docked_height(app, available_height);
    }
    if preferred == RibbonMode::Expanded && available_height >= EXPANDED_HEIGHT + reserved {
        RibbonMode::Expanded
    } else if available_height >= COMPACT_HEIGHT + reserved {
        RibbonMode::Compact
    } else {
        RibbonMode::TabsOnly
    }
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let mode = rendered_mode(app, ui.available_height());
    let popup_id = Id::new("cc_ribbon_flyout");
    if mode != RibbonMode::TabsOnly {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    let tabs = super::visible_tabs(app);
    let state_id = Id::new("cc_ribbon_active");
    let mut active: String = ui.ctx().data(|data| data.get_temp(state_id)).unwrap_or_default();
    if !tabs.iter().any(|tab| tab.id == active) {
        egui::Popup::close_id(ui.ctx(), popup_id);
        active = tabs.first().map(|tab| tab.id.clone()).unwrap_or_default();
    }
    let height = match mode {
        RibbonMode::Expanded => EXPANDED_HEIGHT,
        RibbonMode::Compact => COMPACT_HEIGHT,
        RibbonMode::TabsOnly => TAB_HEIGHT,
        RibbonMode::Hidden => 6.0,
    };
    egui::Panel::top("cc_ribbon").exact_size(height).frame(egui::Frame::NONE.fill(Tokens::get().chrome)).show(ui, |ui| {
        if mode == RibbonMode::Hidden {
            return;
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut responses = Vec::new();
        let mut open = None;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), TAB_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
            egui::ScrollArea::horizontal().id_salt("ribbon_tabs").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tab in &tabs {
                        let response = ui.push_id(&tab.id, |ui| ui.add(egui::Button::selectable(active == tab.id, tab_label(tab)))).inner;
                        crate::icons::describe_control(ui, &response, tab_label(tab), Some(active == tab.id), true);
                        if response.has_focus() {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            open = Some(egui::SetOpenCommand::Bool(active != tab.id || !egui::Popup::is_id_open(ui.ctx(), popup_id)));
                            active = tab.id.clone();
                            response.request_focus();
                        }
                        if response.double_clicked() {
                            run(app, "ui.ribbon.collapse", json!({"collapsed": mode != RibbonMode::TabsOnly}));
                            open = Some(egui::SetOpenCommand::Bool(false));
                        }
                        responses.push(response);
                    }
                });
            });
        });
        if !app.cmd.ime_composing() && !ui.ctx().text_edit_focused() {
            let focused = responses.iter().position(|response| response.has_focus());
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
                    if let Some(response) = responses.get(next) {
                        response.request_focus();
                        response.scroll_to_me(Some(Align::Center));
                    }
                }
                if focused.is_some()
                    && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter) || input.consume_key(Modifiers::NONE, Key::Space))
                {
                    open = Some(egui::SetOpenCommand::Bool(true));
                }
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
                    if let Some(response) = responses.get(index) {
                        let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
                        let screen = ui.ctx().content_rect();
                        let below = (screen.bottom() - response.rect.bottom()).max(1.0);
                        let width = screen.width().min(1000.0);
                        let frame = egui::Frame::popup(ui.style());
                        let margin = frame.total_margin().sum();
                        let close_behavior = if matches!(open, Some(egui::SetOpenCommand::Bool(true))) {
                            egui::PopupCloseBehavior::IgnoreClicks
                        } else {
                            egui::PopupCloseBehavior::CloseOnClickOutside
                        };
                        egui::Popup::from_response(response)
                            .id(popup_id)
                            .open_memory(open)
                            .align(egui::RectAlign::BOTTOM_START)
                            .align_alternatives(&[])
                            .width(width)
                            .frame(frame)
                            .close_behavior(close_behavior)
                            .show(|ui| {
                                ui.set_max_size(vec2((width - margin.x).max(1.0), (below - margin.y).max(1.0)));
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
    });
    ui.ctx().data_mut(|data| data.insert_temp(state_id, active));
}

pub(crate) fn flyout(ui: &mut egui::Ui, app: &mut CadApp, tab: usize) {
    let Some(tab) = super::visible_tabs(app).get(tab).cloned() else { return };
    egui::ScrollArea::both()
        .id_salt((&tab.id, "flyout"))
        .min_scrolled_width(1.0)
        .min_scrolled_height(1.0)
        .max_width(ui.available_width())
        .max_height(ui.available_height())
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GROUP_GAP;
                for group in &tab.groups {
                    let columns = column_widths(ui, group);
                    expanded_group(app, ui, &tab, group, &columns, group_width(&columns));
                }
            });
        });
}

fn compact_groups(app: &mut CadApp, ui: &mut egui::Ui, tab: &RibbonTab) {
    ui.add_space(GROUP_PADDING);
    egui::ScrollArea::horizontal().id_salt((&tab.id, "compact")).max_height(BUTTON_HEIGHT).show(ui, |ui| {
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
    });
}

fn run(app: &mut CadApp, command: &str, params: serde_json::Value) {
    if let Err(error) = app.run(command, params) {
        app.set_status(error);
    }
}

fn scroll_button(ui: &mut egui::Ui, glyph: &str, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled_ui(enabled, |ui| ui.add_sized(vec2(SCROLL_WIDTH, BUTTON_HEIGHT), egui::Button::new(glyph))).inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.on_hover_text(label)
}

const TAB_HEIGHT: f32 = 28.0;
const BUTTON_HEIGHT: f32 = 24.0;
const COLUMN_WIDTH: f32 = 28.0;
const MAX_TEXT_WIDTH: f32 = 160.0;
const COLUMN_GAP: f32 = 6.0;
const ROW_GAP: f32 = 4.0;
const ROWS: usize = 3;
const COLUMNS_HEIGHT: f32 = ROWS as f32 * BUTTON_HEIGHT + (ROWS - 1) as f32 * ROW_GAP;
const LABEL_HEIGHT: f32 = 18.0;
const GROUP_HEIGHT: f32 = COLUMNS_HEIGHT + ROW_GAP + LABEL_HEIGHT;
const GROUP_PADDING: f32 = 6.0;
pub const COMPACT_HEIGHT: f32 = TAB_HEIGHT + BUTTON_HEIGHT + 2.0 * GROUP_PADDING;
const EXPANDED_HEIGHT: f32 = TAB_HEIGHT + GROUP_HEIGHT + 2.0 * GROUP_PADDING;
const GROUP_GAP: f32 = 8.0;
const MIN_GROUP_WIDTH: f32 = 90.0;
const COMPACT_WIDTH: f32 = 76.0;
const SCROLL_WIDTH: f32 = 24.0;

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
    let font = egui::TextStyle::Button.resolve(ui.style());
    group
        .commands
        .chunks(ROWS)
        .map(|commands| {
            commands.iter().fold(COLUMN_WIDTH, |width, id| {
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
        let mut remaining = (available - if scroll { 2.0 * (SCROLL_WIDTH + GROUP_GAP) } else { 0.0 }).max(0.0);
        let mut end = start;
        for (index, group) in tab.groups.iter().enumerate().skip(start) {
            let compact = collapsed.get(index).copied().unwrap_or(true);
            let width = if compact { COMPACT_WIDTH } else { widths.get(index).copied().unwrap_or(MIN_GROUP_WIDTH) };
            if remaining < width {
                break;
            }
            ui.push_id((&tab.id, &group.id), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(width, GROUP_HEIGHT), egui::Sense::hover());
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.set_min_size(vec2(width, GROUP_HEIGHT));
                    ui.spacing_mut().item_spacing.y = ROW_GAP;
                    if compact {
                        egui::containers::menu::MenuButton::from_button(
                            egui::Button::new(group_label(tab, group)).truncate().min_size(vec2(COMPACT_WIDTH, BUTTON_HEIGHT)),
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
    ui.push_id((&tab.id, &group.id), |ui| {
        ui.allocate_ui_with_layout(vec2(width, GROUP_HEIGHT), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            let (rect, _) = ui.allocate_exact_size(vec2(width, COLUMNS_HEIGHT), egui::Sense::hover());
            let mut left = rect.left() + GROUP_PADDING;
            for (column, commands) in group.commands.chunks(ROWS).enumerate() {
                let column_width = columns.get(column).copied().unwrap_or(COLUMN_WIDTH);
                let rect = egui::Rect::from_min_size(egui::pos2(left, rect.top()), vec2(column_width, COLUMNS_HEIGHT));
                left += column_width + COLUMN_GAP;
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.set_min_size(vec2(column_width, COLUMNS_HEIGHT));
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
    ui.push_id(id, |ui| {
        let command = resolve(id);
        let label = label(&command);
        let enabled = command.known && cadcraft_engine::find_command(id).is_none_or(|spec| (spec.enabled)(&app.session).is_ok());
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                if matches!(style, ButtonStyle::Compact) {
                    crate::icons::button(ui, command.icon.unwrap_or(crate::icons::Icon::Help), BUTTON_HEIGHT, &label, false)
                } else if matches!(style, ButtonStyle::Expanded)
                    && let Some(icon) = command.icon
                {
                    crate::icons::button(ui, icon, BUTTON_HEIGHT, &label, false)
                } else {
                    ui.add_sized(
                        if matches!(style, ButtonStyle::Expanded) {
                            vec2(ui.available_width().min(MAX_TEXT_WIDTH), BUTTON_HEIGHT)
                        } else {
                            vec2(140.0, BUTTON_HEIGHT)
                        },
                        egui::Button::new(label.as_str()).truncate(),
                    )
                    .on_hover_text(label.as_str())
                }
            })
            .inner
            .on_disabled_hover_text(label.as_str());
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
