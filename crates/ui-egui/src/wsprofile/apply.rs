//! Puts a workspace layout on the running interface and reads it back. Only interface state is
//! touched: the drawing, its layers, UCS and views are never read or written here.

use std::collections::{BTreeSet, HashSet};

use craft_ui::docking::{Floating, Layout, Location, Node, Placement, StackEntry, Zone};
use craft_ui::layout::{SplitAxis, SplitSize};
use serde_json::Value;

use super::merge::{Level, strip_nulls};
use super::schema::{Dock, PanelMode, PanelState, ToolbarState, ViewState, WorkspaceLayout};
use super::screen::{from_ratio, to_ratio};
use crate::CadApp;
use crate::docking::{Panel, Workspace};

/// Points of a floating panel that stay on screen whatever the screen size.
pub const MIN_VISIBLE: f32 = 48.0;
/// Mark a dock tree whose floating rectangles, and split sizes and accordion heights, are
/// fractions of the content area; older trees carry points.
const RECTS_KEY: &str = "rects";
const SIZES_KEY: &str = "sizes";
const RATIO_RECTS: &str = "ratio";
const STANDARD_TOOLBAR: &str = "standard";
const MIN_FLOAT: egui::Vec2 = egui::vec2(160.0, 120.0);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyReport {
    /// Ids this build does not run; they stay in the profile (placeholders, spec §7).
    pub unknown_commands: Vec<String>,
    pub unknown_panels: Vec<String>,
}

/// The interface state a layout drives, as last applied or read, so a later read can tell the
/// user's own changes from values the layout put there.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fingerprint {
    docking: Layout<Panel>,
    hidden: Vec<(Panel, Location<Panel>)>,
    show_toolbar: bool,
    show_status_bar: bool,
    show_command_line: bool,
    show_toolsets: bool,
    history_lines: usize,
    toolset_tab: String,
    collapsed_groups: Vec<String>,
    hidden_tools: Vec<String>,
    view: ViewState,
    pub(crate) theme: &'static str,
    pub(crate) language: &'static str,
}

impl Fingerprint {
    pub(crate) fn of(app: &CadApp) -> Self {
        let docking = synced_docking(app);
        Self {
            docking: docking.layout,
            hidden: docking.hidden,
            show_toolbar: app.ui.show_toolbar,
            show_status_bar: app.ui.show_status_bar,
            show_command_line: app.ui.show_command_line,
            show_toolsets: app.ui.show_toolsets,
            history_lines: app.ui.history_lines,
            toolset_tab: app.ui.toolset_tab.clone(),
            collapsed_groups: app.ui.collapsed_groups.clone(),
            hidden_tools: app.ui.hidden_tools.clone(),
            view: ViewState {
                file_tabs: app.ui.show_file_tabs,
                viewcube: app.ui.show_viewcube,
                ucs_icon: app.ui.show_ucs_icon,
                layer_list: app.ui.show_layer_list,
                properties_all: app.ui.properties_all,
                in_window_menu: app.ui.in_window_menu,
            },
            theme: app.ui.theme.as_str(),
            language: app.ui.interface_language.code(),
        }
    }

    fn workspace(&self) -> Workspace {
        Workspace { layout: self.docking.clone(), hidden: self.hidden.clone() }
    }
}

/// The docking the next frame will show: the show-palette switches folded in as `docking::show` does.
fn synced_docking(app: &CadApp) -> Workspace {
    let mut workspace = app.ui.docking.clone();
    // Runs every frame; `docking::show` reports the same failure to the user when it draws.
    let _ = crate::docking::sync(&mut workspace, app);
    workspace
}

/// The layout the interface shows now. Fields the interface has no control for yet (ribbon,
/// shortcuts, other tool bars, unknown ids) come from the layout last applied.
pub fn capture(app: &CadApp, area: egui::Rect) -> WorkspaceLayout {
    let mut layout = app.ws.applied.clone();
    let now = Fingerprint::of(app);
    let base = app.ws.synced.as_ref();
    let moved = |same: fn(&Fingerprint, &Fingerprint) -> bool| base.is_none_or(|base| !same(base, &now));
    if moved(|a, b| a.show_toolbar == b.show_toolbar) {
        set_standard_toolbar(&mut layout, now.show_toolbar);
    }
    if moved(|a, b| a.show_status_bar == b.show_status_bar) {
        layout.status_bar.visible = now.show_status_bar;
    }
    if moved(|a, b| a.show_command_line == b.show_command_line) {
        layout.command_line.visible = now.show_command_line;
    }
    if moved(|a, b| a.history_lines == b.history_lines) {
        layout.command_line.lines = u32::try_from(now.history_lines).unwrap_or(u32::MAX);
    }
    if moved(|a, b| a.show_toolsets == b.show_toolsets) {
        layout.toolsets.visible = now.show_toolsets;
    }
    if moved(|a, b| a.toolset_tab == b.toolset_tab) {
        layout.toolsets.tab = now.toolset_tab.clone();
    }
    if moved(|a, b| a.collapsed_groups == b.collapsed_groups) {
        layout.toolsets.collapsed_groups = now.collapsed_groups.clone();
    }
    if moved(|a, b| a.hidden_tools == b.hidden_tools) {
        layout.toolsets.hidden_tools = now.hidden_tools.clone();
    }
    let view: [(fn(&ViewState) -> bool, fn(&mut ViewState) -> &mut bool); 6] = [
        (|view| view.file_tabs, |view| &mut view.file_tabs),
        (|view| view.viewcube, |view| &mut view.viewcube),
        (|view| view.ucs_icon, |view| &mut view.ucs_icon),
        (|view| view.layer_list, |view| &mut view.layer_list),
        (|view| view.properties_all, |view| &mut view.properties_all),
        (|view| view.in_window_menu, |view| &mut view.in_window_menu),
    ];
    for (read, write) in view {
        if base.is_none_or(|base| read(&base.view) != read(&now.view)) {
            *write(&mut layout.view) = read(&now.view);
        }
    }
    if moved(|a, b| a.docking == b.docking && a.hidden == b.hidden) {
        let workspace = now.workspace();
        let previous = layout.panels.clone();
        layout.panels = panels_from_docking(&workspace, area, &previous);
        for state in &mut layout.panels {
            let Some(before) = previous.iter().find(|panel| panel.id == state.id) else { continue };
            for (name, value, prior) in [
                ("width_ratio", &mut state.width_ratio, before.width_ratio),
                ("height_ratio", &mut state.height_ratio, before.height_ratio),
                ("x_ratio", &mut state.x_ratio, before.x_ratio),
                ("y_ratio", &mut state.y_ratio, before.y_ratio),
            ] {
                if (*value - prior).abs() <= 0.000001 && super::merge::check(&app.ws.effective, &format!("/panels/{}/{name}", state.id)).is_err() {
                    *value = prior;
                }
            }
        }
        layout.dock_tree = Some(dock_tree(&workspace, area));
    }
    layout
}

/// Sets the interface to `layout`. Unknown panel and command ids are reported and left in the
/// layout; an unusable dock tree falls back to the flat panel list.
pub fn apply(app: &mut CadApp, layout: &WorkspaceLayout, area: egui::Rect) -> ApplyReport {
    app.ui.show_toolbar = layout.toolbars.iter().any(|toolbar| toolbar.id == STANDARD_TOOLBAR && toolbar.visible);
    app.ui.show_status_bar = layout.status_bar.visible;
    app.ui.show_command_line = layout.command_line.visible;
    app.ui.history_lines = usize::try_from(layout.command_line.lines).unwrap_or(crate::cmdline::MAX_LINES).min(crate::cmdline::MAX_LINES);
    // The Tool Sets palette knows these two tabs only; another value stays in the profile.
    if matches!(layout.toolsets.tab.as_str(), "Drafting" | "Modeling") {
        app.ui.toolset_tab = layout.toolsets.tab.clone();
    }
    app.ui.collapsed_groups = layout.toolsets.collapsed_groups.clone();
    app.ui.hidden_tools = layout.toolsets.hidden_tools.clone();
    let view = &layout.view;
    app.ui.show_file_tabs = view.file_tabs;
    app.ui.show_viewcube = view.viewcube;
    app.ui.show_ucs_icon = view.ucs_icon;
    app.ui.show_layer_list = view.layer_list;
    app.ui.properties_all = view.properties_all;
    app.ui.in_window_menu = view.in_window_menu;

    let pinned = pinned_panels(app, layout);
    let mut docking = docking_from_tree(layout, area)
        .filter(|tree| tree_agrees(app, tree, layout, &pinned, area))
        .unwrap_or_else(|| docking_from_panels(&layout.panels, area));
    if !layout.toolsets.visible {
        let _ = docking.set_visible(Panel::ToolSets, false);
    }
    for state in layout.panels.iter().filter(|state| state.mode == PanelMode::AutoHide) {
        if let Some(panel) = panel_of(&state.id)
            && panel != Panel::Canvas
        {
            let _ = docking.set_visible(panel, false);
        }
    }
    app.ui.show_toolsets = docking.layout.contains(&Panel::ToolSets);
    app.ui.show_palettes = docking.layout.contains(&Panel::Layers) || docking.layout.contains(&Panel::Properties);
    app.ui.docking = docking;

    app.ws.applied = layout.clone();
    app.ws.synced = Some(Fingerprint::of(app));
    report(layout)
}

/// Applies profile zoom once, then observes egui keyboard zoom through the debounced profile path.
pub(crate) fn sync_scale(app: &mut CadApp, ctx: &egui::Context) {
    let wanted = app.ws.device.ui_scale.clamp(0.75, 2.0);
    if !wanted.is_finite() {
        return;
    }
    if app.ws.shown_scale != Some(wanted) {
        ctx.set_zoom_factor(wanted);
        app.ws.shown_scale = Some(wanted);
        return;
    }
    let actual = ctx.zoom_factor();
    if !actual.is_finite() || actual == wanted {
        return;
    }
    let scale = actual.clamp(0.75, 2.0);
    if scale != wanted {
        if let Err(error) = app.run("ui.workspace.set", serde_json::json!({"pointer": "/ui_scale", "value": scale})) {
            app.set_status(error);
            ctx.set_zoom_factor(wanted);
            return;
        }
        app.ws.shown_scale = Some(scale);
    }
    if actual != scale {
        ctx.set_zoom_factor(scale);
    }
}

/// Remaps live floating panels when the window or zoom changes, including after startup.
pub(crate) fn remap_screen(app: &mut CadApp, ctx: &egui::Context) {
    let area = ctx.content_rect();
    let previous = app.ws.area;
    if !app.ws.area_known || area == previous || !area.is_finite() || !area.is_positive() {
        return;
    }
    for group in &mut app.ui.docking.layout.floating {
        group.rect = floating_rect(to_ratio(rect_points(group.rect), previous), area);
    }
    for (_, location) in &mut app.ui.docking.hidden {
        if let Some(rect) = &mut location.floating {
            *rect = floating_rect(to_ratio(rect_points(*rect), previous), area);
        }
    }
}

/// Panels whose `panels[]` entry outranks the user's dock tree: locked by policy or placed by
/// the session layer.
fn pinned_panels(app: &CadApp, layout: &WorkspaceLayout) -> Vec<String> {
    let mut ids = Vec::new();
    for lock in app.ws.effective.locked.keys() {
        if lock.is_empty() || lock == "/panels" {
            return layout.panels.iter().map(|state| state.id.clone()).collect();
        }
        if let Some(token) = lock.strip_prefix("/panels/").and_then(|rest| rest.split('/').next()) {
            ids.push(token.replace("~1", "/").replace("~0", "~"));
        }
    }
    let session = app.ws.layers.iter().filter(|layer| layer.level == Level::Session);
    for items in session.filter_map(|layer| layer.patch.get("panels").and_then(Value::as_array)) {
        ids.extend(items.iter().filter_map(|item| item.get("id").and_then(Value::as_str)).map(str::to_string));
    }
    ids
}

/// Shown, floating, and the side it docks to.
fn placement(state: &PanelState) -> (bool, bool, Option<Dock>) {
    let shown = state.visible && !matches!(state.mode, PanelMode::Hidden | PanelMode::AutoHide);
    let floats = state.mode == PanelMode::Float || state.side == Some(Dock::Floating);
    let side = if shown && !floats { state.side } else { None };
    (shown, shown && floats, side)
}

/// Whether `tree` places each pinned panel where `layout.panels` says.
fn tree_agrees(app: &CadApp, tree: &Workspace, layout: &WorkspaceLayout, pinned: &[String], area: egui::Rect) -> bool {
    if pinned.is_empty() {
        return true;
    }
    let shown = panels_from_docking(tree, area, &layout.panels);
    let flat = docking_from_panels(&layout.panels, area);
    let expected = panels_from_docking(&flat, area, &layout.panels);
    pinned.iter().all(|id| {
        let Some(wanted) = expected.iter().find(|state| state.id == *id) else { return true };
        let locked = |field| pinned_field(app, id, field);
        let wanted_side = |side: Option<Dock>| panel_of(id).map(|panel| zone_of(side, panel));
        shown.iter().find(|state| state.id == *id).is_some_and(|state| {
            let (now, want) = (placement(state), placement(wanted));
            if (locked("visible") && now.0 != want.0)
                || (locked("mode") && (now.0 != want.0 || now.1 != want.1))
                || (locked("side") && (now.1 != want.1 || wanted_side(state.side) != wanted_side(wanted.side)))
                || (locked("tab_group") && !same_tab_members(tree, &flat, id))
            {
                return false;
            }
            let Some([x, y, width, height]) = panel_ratios(tree, state, area, &layout.panels) else { return false };
            let Some([want_x, want_y, want_width, want_height]) = panel_ratios(&flat, wanted, area, &layout.panels) else { return false };
            // Compare normalized geometry so split rounding and minimum floating sizes do not reject a valid tree.
            [("width_ratio", width, want_width), ("height_ratio", height, want_height), ("x_ratio", x, want_x), ("y_ratio", y, want_y)]
                .iter()
                .all(|(field, actual, expected)| !locked(field) || (actual - expected).abs() <= 0.000001)
        })
    })
}

fn panel_ratios(workspace: &Workspace, state: &PanelState, area: egui::Rect, previous: &[PanelState]) -> Option<[f32; 4]> {
    let mut state = state.clone();
    if let Some(panel) = panel_of(&state.id)
        && workspace.hidden.iter().any(|(hidden, _)| *hidden == panel)
    {
        // Hidden locations also carry geometry; copying the prior flat entry would conceal a changed restore size.
        let mut opened = workspace.clone();
        opened.set_visible(panel, true).ok()?;
        state = panels_from_docking(&opened, area, previous).into_iter().find(|placed| placed.id == state.id)?;
    }
    Some([state.x_ratio, state.y_ratio, state.width_ratio, state.height_ratio])
}

fn pinned_field(app: &CadApp, id: &str, field: &str) -> bool {
    let pointer = format!("/panels/{}/{field}", super::merge::escape_token(id));
    super::merge::check(&app.ws.effective, &pointer).is_err()
        || app.ws.layers.iter().filter(|layer| layer.level == Level::Session).any(|layer| {
            layer
                .patch
                .get("panels")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item.get("id").and_then(Value::as_str) == Some(id) && item.get(field).is_some()))
        })
}

fn same_tab_members(tree: &Workspace, flat: &Workspace, id: &str) -> bool {
    fn members(workspace: &Workspace, id: &str) -> Option<Vec<Panel>> {
        let mut workspace = workspace.clone();
        if let Some(panel) = panel_of(id) {
            workspace.set_visible(panel, true).ok()?;
        }
        for group in &workspace.layout.floating {
            if group.panels.iter().any(|panel| panel.id() == id) {
                return Some(group.panels.clone());
            }
        }
        let mut visit: Vec<_> = workspace.layout.root.iter().collect();
        while let Some(node) = visit.pop() {
            match node {
                Node::Split { first, second, .. } => {
                    visit.push(first);
                    visit.push(second);
                }
                Node::Tabs { panels, .. } if panels.iter().any(|panel| panel.id() == id) => return Some(panels.clone()),
                Node::Stack { entries } if entries.iter().any(|entry| entry.panel.id() == id) => {
                    return Some(entries.iter().filter(|entry| entry.panel.id() == id).map(|entry| entry.panel).collect());
                }
                Node::Tabs { .. } | Node::Stack { .. } => {}
            }
        }
        Some(Vec::new())
    }
    let (actual, expected) = (members(tree, id), members(flat, id));
    actual.zip(expected).is_some_and(|(actual, expected)| actual.len() == expected.len() && actual.iter().all(|panel| expected.contains(panel)))
}

fn set_standard_toolbar(layout: &mut WorkspaceLayout, visible: bool) {
    match layout.toolbars.iter_mut().find(|toolbar| toolbar.id == STANDARD_TOOLBAR) {
        Some(toolbar) => toolbar.visible = visible,
        None if visible => {
            let standard = WorkspaceLayout::default().toolbars.into_iter().find(|toolbar| toolbar.id == STANDARD_TOOLBAR);
            layout.toolbars.push(standard.unwrap_or_else(|| ToolbarState { id: STANDARD_TOOLBAR.into(), ..ToolbarState::default() }));
        }
        None => {}
    }
}

fn panel_of(id: &str) -> Option<Panel> {
    Panel::ALL.iter().chain(Panel::ON_DEMAND).copied().find(|panel| panel.id() == id)
}

/// Whether this build runs the command `id` (an id it does not know stays in the profile as a placeholder).
pub(crate) fn command_known(id: &str) -> bool {
    crate::menus::is_ui_command(id)
        || cadcraft_engine::command_specs().iter().any(|spec| spec.id == id || spec.aliases.contains(&id))
        || cadcraft_engine::cmd::registry::resolve_command(id).is_some()
}

fn report(layout: &WorkspaceLayout) -> ApplyReport {
    let known = command_known;
    let ribbon = layout.ribbon.tabs.iter().flat_map(|tab| tab.groups.iter()).flat_map(|group| group.commands.iter());
    let toolbars = layout.toolbars.iter().flat_map(|toolbar| toolbar.commands.iter());
    let named = ribbon
        .chain(toolbars)
        .chain(layout.quick_access.iter())
        .chain(layout.favorites.iter())
        .chain(layout.toolsets.hidden_tools.iter())
        .chain(layout.shortcuts.keys())
        .chain(layout.aliases.values());
    let unknown_commands: BTreeSet<String> = named.filter(|id| !known(id)).cloned().collect();
    let mut unknown_panels: Vec<String> = Vec::new();
    for panel in &layout.panels {
        if panel_of(&panel.id).is_none() && !unknown_panels.contains(&panel.id) {
            unknown_panels.push(panel.id.clone());
        }
    }
    ApplyReport { unknown_commands: unknown_commands.into_iter().collect(), unknown_panels }
}

fn rect_points(rect: [f32; 4]) -> egui::Rect {
    let [x, y, width, height] = rect;
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
}

fn rect_array(rect: egui::Rect) -> [f32; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()]
}

fn floating_rect(ratio: [f32; 4], area: egui::Rect) -> [f32; 4] {
    let [x, y, width, height] = ratio;
    let min = MIN_FLOAT / area.size().max(egui::vec2(1.0, 1.0));
    rect_array(from_ratio([x, y, width.max(min.x), height.max(min.y)], area, MIN_VISIBLE))
}

// Capture before committing so a policy rejection leaves the live tree untouched.
pub(crate) fn capture_docking(workspace: &Workspace, area: egui::Rect, previous: &[PanelState]) -> (Vec<PanelState>, Value) {
    (panels_from_docking(workspace, area, previous), dock_tree(workspace, area))
}

/// The tree as stored: floating rectangles as fractions of `area`, no null members.
fn dock_tree(workspace: &Workspace, area: egui::Rect) -> Value {
    let mut copy = workspace.clone();
    for group in &mut copy.layout.floating {
        group.rect = to_ratio(rect_points(group.rect), area);
    }
    for (_, location) in &mut copy.hidden {
        if let Some(rect) = &mut location.floating {
            *rect = to_ratio(rect_points(*rect), area);
        }
    }
    scale_sizes(&mut copy, area, true);
    let mut value = serde_json::to_value(&copy).unwrap_or(Value::Null);
    strip_nulls(&mut value);
    if let Some(fields) = value.as_object_mut() {
        fields.insert(RECTS_KEY.into(), Value::String(RATIO_RECTS.into()));
        fields.insert(SIZES_KEY.into(), Value::String(RATIO_RECTS.into()));
    }
    value
}

fn scale_split(size: SplitSize, along: f32, into_fraction: bool) -> SplitSize {
    let scale = |value: f32| if into_fraction { value / along } else { value * along };
    match size {
        SplitSize::FixedFirst(value) => SplitSize::FixedFirst(scale(value)),
        SplitSize::FixedSecond(value) => SplitSize::FixedSecond(scale(value)),
        ratio @ SplitSize::Ratio(_) => ratio,
    }
}

fn scale_node(node: &mut Node<Panel>, extent: egui::Vec2, into_fraction: bool, depth: usize) {
    if depth > craft_ui::docking::MAX_DEPTH {
        return;
    }
    match node {
        Node::Split { axis, size, first, second } => {
            let along = if *axis == SplitAxis::Horizontal { extent.x } else { extent.y };
            *size = scale_split(*size, along, into_fraction);
            scale_node(first, extent, into_fraction, depth + 1);
            scale_node(second, extent, into_fraction, depth + 1);
        }
        Node::Stack { entries } => {
            for entry in entries {
                entry.height = entry.height.map(|height| if into_fraction { height / extent.y } else { height * extent.y });
            }
        }
        Node::Tabs { .. } => {}
    }
}

/// Converts split sizes and accordion heights between points and fractions of `area`, so a
/// layout keeps its proportions on another screen.
fn scale_sizes(workspace: &mut Workspace, area: egui::Rect, into_fraction: bool) {
    let extent = area.size().max(egui::vec2(1.0, 1.0));
    if let Some(root) = &mut workspace.layout.root {
        scale_node(root, extent, into_fraction, 0);
    }
    for (_, location) in &mut workspace.hidden {
        let along = if matches!(location.placement, Placement::Split(Zone::Top | Zone::Bottom)) { extent.y } else { extent.x };
        location.size = location.size.map(|size| scale_split(size, along, into_fraction));
        if let Some((_, Some(height))) = &mut location.stack {
            *height = if into_fraction { *height / extent.y } else { *height * extent.y };
        }
    }
}

fn docking_from_tree(layout: &WorkspaceLayout, area: egui::Rect) -> Option<Workspace> {
    let tree = layout.dock_tree.as_ref()?;
    let ratio = tree.get(RECTS_KEY).and_then(Value::as_str) == Some(RATIO_RECTS);
    let mut workspace: Workspace = serde_json::from_value(tree.clone()).ok()?;
    if tree.get(SIZES_KEY).and_then(Value::as_str) == Some(RATIO_RECTS) {
        scale_sizes(&mut workspace, area, false);
    }
    let place = |rect: [f32; 4]| if ratio { floating_rect(rect, area) } else { floating_rect(to_ratio(rect_points(rect), area), area) };
    for group in &mut workspace.layout.floating {
        group.rect = place(group.rect);
    }
    for (_, location) in &mut workspace.hidden {
        if let Some(rect) = &mut location.floating {
            *rect = place(*rect);
        }
    }
    workspace.validate().ok()?;
    Some(workspace)
}

struct Group {
    tag: Option<String>,
    panels: Vec<Panel>,
    thickness: f32,
    share: f32,
}

fn unit_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 { value.min(1.0) } else { fallback }
}

fn zone_of(side: Option<Dock>, panel: Panel) -> Zone {
    match side {
        Some(Dock::Left) => Zone::Left,
        Some(Dock::Right) => Zone::Right,
        Some(Dock::Top) => Zone::Top,
        Some(Dock::Bottom) => Zone::Bottom,
        Some(Dock::Floating) | None if panel == Panel::ToolSets => Zone::Left,
        Some(Dock::Floating) | None => Zone::Right,
    }
}

fn side_index(zone: Zone) -> usize {
    match zone {
        Zone::Left => 0,
        Zone::Right | Zone::Center => 1,
        Zone::Top => 2,
        Zone::Bottom => 3,
    }
}

fn tabs(panels: Vec<Panel>) -> Node<Panel> {
    Node::Tabs { panels, active: 0 }
}

/// Builds the docking from the flat `panels[]` list (presets carry no dock tree).
fn docking_from_panels(panels: &[PanelState], area: egui::Rect) -> Workspace {
    let mut seen: HashSet<Panel> = HashSet::new();
    let mut sides: [Vec<Group>; 4] = Default::default();
    let mut floating: Vec<(Option<String>, Floating<Panel>)> = Vec::new();
    let mut hidden: Vec<(Panel, Location<Panel>)> = Vec::new();
    for state in panels {
        let Some(panel) = panel_of(&state.id) else { continue };
        if panel == Panel::Canvas || !seen.insert(panel) {
            continue;
        }
        let ratio = [state.x_ratio, state.y_ratio, state.width_ratio, state.height_ratio];
        let floats = state.mode == PanelMode::Float || state.side == Some(Dock::Floating);
        if !state.visible || matches!(state.mode, PanelMode::Hidden | PanelMode::AutoHide) {
            let zone = zone_of(state.side, panel);
            let location = if floats {
                Location {
                    floating: Some(floating_rect(ratio, area)),
                    anchor: None,
                    placement: Placement::Split(zone),
                    size: None,
                    stack: None,
                    sibling: Vec::new(),
                }
            } else {
                Location {
                    floating: None,
                    anchor: Some(Panel::Canvas),
                    placement: Placement::Split(zone),
                    size: None,
                    stack: None,
                    sibling: Vec::new(),
                }
            };
            hidden.push((panel, location));
            continue;
        }
        if floats {
            match floating.iter_mut().find(|(tag, _)| tag.is_some() && *tag == state.tab_group) {
                Some((_, group)) => group.panels.push(panel),
                None => floating.push((state.tab_group.clone(), Floating { panels: vec![panel], active: 0, rect: floating_rect(ratio, area) })),
            }
            continue;
        }
        let zone = zone_of(state.side, panel);
        let across = matches!(zone, Zone::Left | Zone::Right);
        let (thickness, share) = if across {
            (unit_or(state.width_ratio, 0.2), unit_or(state.height_ratio, 1.0))
        } else {
            (unit_or(state.height_ratio, 0.2), unit_or(state.width_ratio, 1.0))
        };
        let Some(side) = sides.get_mut(side_index(zone)) else { continue };
        match side.iter_mut().find(|group| group.tag.is_some() && group.tag == state.tab_group) {
            Some(group) => group.panels.push(panel),
            None => side.push(Group { tag: state.tab_group.clone(), panels: vec![panel], thickness, share }),
        }
    }

    let thickness = |groups: &[Group]| groups.iter().map(|group| group.thickness).fold(0.0f32, f32::max).clamp(0.05, 0.9);
    let [left, right, top, bottom] = sides;
    let mut root = tabs(vec![Panel::Canvas]);
    // Top and bottom wrap the drawing first, so the side columns run the full height as they do today.
    if let Some(node) = side_node(&top, false, area) {
        root = Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::FixedFirst(thickness(&top) * area.height()),
            first: Box::new(node),
            second: Box::new(root),
        };
    }
    if let Some(node) = side_node(&bottom, false, area) {
        root = Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::FixedSecond(thickness(&bottom) * area.height()),
            first: Box::new(root),
            second: Box::new(node),
        };
    }
    if let Some(node) = side_node(&right, true, area) {
        root = Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(thickness(&right) * area.width()),
            first: Box::new(root),
            second: Box::new(node),
        };
    }
    if let Some(node) = side_node(&left, true, area) {
        root = Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(thickness(&left) * area.width()),
            first: Box::new(node),
            second: Box::new(root),
        };
    }
    let workspace = Workspace { layout: Layout { root: Some(root), floating: floating.into_iter().map(|(_, group)| group).collect() }, hidden };
    if workspace.validate().is_ok() {
        return workspace;
    }
    // A hidden panel that cannot come back where it was still opens at its default side.
    let without_hidden = Workspace { hidden: Vec::new(), ..workspace };
    if without_hidden.validate().is_ok() { without_hidden } else { Workspace::default() }
}

/// One side's groups: an accordion for single panels in a column, else splits along the side.
fn side_node(groups: &[Group], column: bool, area: egui::Rect) -> Option<Node<Panel>> {
    let (last, rest) = groups.split_last()?;
    if rest.is_empty() {
        return Some(tabs(last.panels.clone()));
    }
    if column && groups.iter().all(|group| group.panels.len() == 1) {
        let entries = groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                let panel = *group.panels.first()?;
                let height = (index + 1 < groups.len()).then(|| group.share * area.height());
                Some(StackEntry { panel, open: true, height })
            })
            .collect();
        return Some(Node::Stack { entries });
    }
    let axis = if column { SplitAxis::Vertical } else { SplitAxis::Horizontal };
    let mut node = tabs(last.panels.clone());
    let mut remaining = last.share;
    for group in rest.iter().rev() {
        remaining += group.share;
        let ratio = if remaining > 0.0 { (group.share / remaining).clamp(0.05, 0.95) } else { 0.5 };
        node = Node::Split { axis, size: SplitSize::Ratio(ratio), first: Box::new(tabs(group.panels.clone())), second: Box::new(node) };
    }
    Some(node)
}

fn split_fraction(size: SplitSize, total: f32) -> f32 {
    let fraction = match size {
        SplitSize::Ratio(ratio) => ratio,
        SplitSize::FixedFirst(points) => points / total,
        SplitSize::FixedSecond(points) => 1.0 - points / total,
    };
    if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.5 }
}

fn holds_canvas(node: &Node<Panel>) -> bool {
    match node {
        Node::Split { first, second, .. } => holds_canvas(first) || holds_canvas(second),
        Node::Tabs { panels, .. } => panels.contains(&Panel::Canvas),
        Node::Stack { entries } => entries.iter().any(|entry| entry.panel == Panel::Canvas),
    }
}

struct Placed {
    panel: Panel,
    side: Option<Dock>,
    group: Option<Vec<Panel>>,
    thickness: f32,
    share: f32,
}

fn collect_docked(node: &Node<Panel>, side: Option<Dock>, thickness: f32, share: f32, area: egui::Rect, out: &mut Vec<Placed>, depth: usize) {
    if depth > craft_ui::docking::MAX_DEPTH {
        return;
    }
    match node {
        Node::Split { axis, size, first, second } => {
            let horizontal = *axis == SplitAxis::Horizontal;
            let total = if horizontal { area.width() } else { area.height() };
            let fraction = split_fraction(*size, total);
            if side.is_some() || (!holds_canvas(first) && !holds_canvas(second)) {
                collect_docked(first, side, thickness, share * fraction, area, out, depth + 1);
                collect_docked(second, side, thickness, share * (1.0 - fraction), area, out, depth + 1);
            } else if holds_canvas(first) {
                collect_docked(first, None, thickness, share, area, out, depth + 1);
                collect_docked(second, Some(if horizontal { Dock::Right } else { Dock::Bottom }), 1.0 - fraction, 1.0, area, out, depth + 1);
            } else {
                collect_docked(first, Some(if horizontal { Dock::Left } else { Dock::Top }), fraction, 1.0, area, out, depth + 1);
                collect_docked(second, None, thickness, share, area, out, depth + 1);
            }
        }
        Node::Tabs { panels, .. } => {
            let group = (panels.len() > 1).then(|| panels.clone());
            for panel in panels.iter().filter(|panel| **panel != Panel::Canvas) {
                out.push(Placed { panel: *panel, side, group: group.clone(), thickness, share });
            }
        }
        Node::Stack { entries } => {
            let height = area.height().max(1.0);
            let fixed: f32 = entries.iter().filter_map(|entry| entry.height).sum();
            let flexible = entries.iter().filter(|entry| entry.height.is_none()).count().max(1) as f32;
            for entry in entries.iter().filter(|entry| entry.panel != Panel::Canvas) {
                let part = entry.height.map_or_else(|| ((height - fixed) / height).max(0.0) / flexible, |points| points / height);
                out.push(Placed { panel: entry.panel, side, group: None, thickness, share: share * unit_or(part, 1.0) });
            }
        }
    }
}

/// The flat `panels[]` description of a docking (D3: exported beside the tree, read on import).
fn panels_from_docking(workspace: &Workspace, area: egui::Rect, previous: &[PanelState]) -> Vec<PanelState> {
    let earlier = |panel: Panel| previous.iter().find(|state| state.id == panel.id());
    let group_name = |members: &[Panel], panel: Panel| {
        earlier(panel).and_then(|state| state.tab_group.clone()).or_else(|| members.first().map(|first| format!("group.{}", first.id())))
    };
    let mut out: Vec<PanelState> = Vec::new();
    let mut placed = Vec::new();
    if let Some(root) = &workspace.layout.root {
        collect_docked(root, None, 1.0, 1.0, area, &mut placed, 0);
    }
    for item in placed {
        let across = matches!(item.side, Some(Dock::Left | Dock::Right) | None);
        let (width_ratio, height_ratio) = if across { (item.thickness, item.share) } else { (item.share, item.thickness) };
        out.push(PanelState {
            id: item.panel.id().into(),
            mode: PanelMode::Dock,
            side: item.side,
            tab_group: item.group.as_deref().and_then(|members| group_name(members, item.panel)),
            visible: true,
            x_ratio: 0.0,
            y_ratio: 0.0,
            width_ratio: unit_or(width_ratio, 0.2),
            height_ratio: unit_or(height_ratio, 1.0),
        });
    }
    for group in &workspace.layout.floating {
        let [x_ratio, y_ratio, width_ratio, height_ratio] = to_ratio(rect_points(group.rect), area);
        for panel in group.panels.iter().filter(|panel| **panel != Panel::Canvas) {
            out.push(PanelState {
                id: panel.id().into(),
                mode: PanelMode::Float,
                side: Some(Dock::Floating),
                tab_group: (group.panels.len() > 1).then(|| group_name(&group.panels, *panel)).flatten(),
                visible: true,
                x_ratio,
                y_ratio,
                width_ratio,
                height_ratio,
            });
        }
    }
    for (panel, location) in &workspace.hidden {
        let mode = if earlier(*panel).is_some_and(|state| state.mode == PanelMode::AutoHide) { PanelMode::AutoHide } else { PanelMode::Hidden };
        let side = match location.placement {
            _ if location.floating.is_some() => Some(Dock::Floating),
            Placement::Split(Zone::Left) => Some(Dock::Left),
            Placement::Split(Zone::Top) => Some(Dock::Top),
            Placement::Split(Zone::Bottom) => Some(Dock::Bottom),
            _ => earlier(*panel).and_then(|state| state.side).or(Some(Dock::Right)),
        };
        let base = earlier(*panel).cloned().unwrap_or_default();
        out.push(PanelState { id: panel.id().into(), mode, side, visible: false, ..base });
    }
    // Panels not on screen and ids this build does not know keep their entries.
    for state in previous {
        if out.iter().any(|placed| placed.id == state.id) {
            continue;
        }
        let mut kept = state.clone();
        if panel_of(&state.id).is_some() {
            kept.visible = false;
            if kept.mode != PanelMode::AutoHide {
                kept.mode = PanelMode::Hidden;
            }
        }
        out.push(kept);
    }
    let rank = |state: &PanelState| previous.iter().position(|earlier| earlier.id == state.id).unwrap_or(usize::MAX);
    out.sort_by_key(rank);
    out
}

/// A removed accordion member gives its space to unlocked siblings; policy sizes keep their
/// prior fractions instead of changing when a formerly flexible member becomes the last one.
pub(crate) fn keep_locked_stack_sizes(app: &CadApp, next: &mut Workspace) {
    let mut old_stacks = Vec::new();
    let mut visit: Vec<_> = app.ui.docking.layout.root.iter().collect();
    while let Some(node) = visit.pop() {
        match node {
            Node::Split { first, second, .. } => {
                visit.push(first);
                visit.push(second);
            }
            Node::Stack { entries } => old_stacks.push(entries.iter().map(|entry| entry.panel).collect::<Vec<_>>()),
            Node::Tabs { .. } => {}
        }
    }
    let mut placed = Vec::new();
    if let Some(root) = &next.layout.root {
        collect_docked(root, None, 1.0, 1.0, app.ws.area, &mut placed, 0);
    }
    let height = app.ws.area.height().max(1.0);
    let mut visit: Vec<_> = next.layout.root.iter_mut().collect();
    while let Some(node) = visit.pop() {
        match node {
            Node::Split { first, second, .. } => {
                visit.push(first);
                visit.push(second);
            }
            Node::Stack { entries } => {
                let changed = old_stacks.iter().any(|old| {
                    old.iter().any(|panel| entries.iter().any(|entry| entry.panel == *panel))
                        && old.iter().any(|panel| !entries.iter().any(|entry| entry.panel == *panel))
                });
                if !changed {
                    continue;
                }
                let fixed: f32 = entries.iter().filter_map(|entry| entry.height).sum();
                let flexible = entries.iter().filter(|entry| entry.height.is_none()).count().max(1) as f32;
                let locked = |panel: Panel| {
                    let pointer = format!("/panels/{}/height_ratio", panel.id());
                    super::merge::check(&app.ws.effective, &pointer).is_err()
                };
                if !entries.iter().any(|entry| locked(entry.panel)) {
                    continue;
                }
                let mut locked_height = 0.0;
                let mut unlocked_weight = 0.0;
                for entry in entries.iter_mut() {
                    let part = entry.height.map_or_else(|| ((height - fixed) / height).max(0.0) / flexible, |points| points / height);
                    if locked(entry.panel) {
                        if let Some(previous) = app.ws.applied.panels.iter().find(|state| state.id == entry.panel.id())
                            && let Some(now) = placed.iter().find(|item| item.panel == entry.panel)
                            && part > 0.0
                            && now.share > 0.0
                        {
                            entry.height = Some(previous.height_ratio * height * part / now.share);
                        }
                        locked_height += entry.height.unwrap_or(0.0);
                    } else {
                        entry.height = Some(part.max(0.001));
                        unlocked_weight += part.max(0.001);
                    }
                }
                if unlocked_weight > 0.0 {
                    for entry in entries.iter_mut().filter(|entry| !locked(entry.panel)) {
                        entry.height = Some((height - locked_height).max(0.0) * entry.height.unwrap_or(0.0) / unlocked_weight);
                    }
                }
            }
            Node::Tabs { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::command_known;

    #[test]
    fn command_known_accepts_namespaced_engine_ids() {
        assert!(command_known("cad.draw.line"));
        assert!(command_known("cad.other.-block"));
        assert!(command_known("line"));
        assert!(command_known("l"));
        assert!(command_known("ui.dock"));
        assert!(!command_known("cad.draw.nope"));
    }
}
