//! Data-driven tool bars: the profile's `toolbars[]` drawn on the four window edges or floating,
//! and the `ui.toolbar.*` commands that change them.

use serde_json::{Value, json};

use crate::CadApp;
use crate::icons::Icon;
use crate::palettes;
use crate::wsprofile::schema::{Dock, ToolbarState};

pub mod drag;
mod menu;
pub mod render;

pub use drag::{DragPhase, DropTarget};
pub use render::{floating_rect, show_docked, show_floating};

pub(crate) use menu::{claim_menu_key, menu_check_state, menu_entries};

/// The tool bars this build ships: (id, label).
pub const BUILTIN: &[(&str, &str)] =
    &[("standard", "Standard"), ("draw", "Draw"), ("modify", "Modify"), ("dimension", "Dimension"), ("snap", "Object Snap")];

pub(crate) const COMMAND_PREFIX: &str = "ui.toolbar.";
/// Window ▸ Toolbars items are `ui.toolbar.hide.<id>` while the bar shows and `ui.toolbar.show.<id>` while it is hidden.
pub(crate) const HIDE_ITEM: &str = "ui.toolbar.hide.";
pub(crate) const SHOW_ITEM: &str = "ui.toolbar.show.";

/// (icon, command, label, shortcut)
pub type Tool = (Icon, &'static str, &'static str, Option<&'static str>);

const STANDARD: &[&[Tool]] = &[
    &[
        (Icon::New, "new", "New Drawing", Some("Cmd+N")),
        (Icon::Open, "open", "Open", Some("Cmd+O")),
        (Icon::Save, "qsave", "Save", Some("Cmd+S")),
        (Icon::SaveAs, "saveas", "Save As", None),
    ],
    &[(Icon::Undo, "u", "Undo", Some("Cmd+Z")), (Icon::Redo, "redo", "Redo", Some("Cmd+Shift+Z"))],
    &[
        (Icon::Plot, "plot", "Print", None),
        (Icon::Publish, "publish", "Batch Publish", None),
        (Icon::PageSetup, "pagesetup", "Page Setup Manager", None),
        (Icon::Preview, "preview", "Plot Preview", None),
    ],
    &[
        (Icon::Import, "import", "Import", None),
        (Icon::Export, "export", "Export", None),
        (Icon::Attach, "attach", "Attach", None),
        (Icon::Share, "share", "Share", None),
    ],
    &[(Icon::ZoomWindow, "zoom.window", "Zoom Window", None), (Icon::Pan, "pan", "Pan", None), (Icon::Orbit, "3dorbit", "Orbit", None)],
    &[
        (Icon::ZoomExtents, "zoom.extents", "Zoom Extents", None),
        (Icon::Properties, "ui.toggle.palettes", "Properties", None),
        (Icon::Layers, "ui.dialog.layers", "Layer Properties Manager", None),
        (Icon::Blocks, "ui.dialog.blocks", "Blocks", None),
    ],
    &[(Icon::Measure, "dist", "Measure Distance", None), (Icon::List, "list", "List", None), (Icon::Area, "area", "Area", None)],
    &[(Icon::Help, "ui.dialog.about", "Help", None)],
];

const SNAP: &[Tool] = &[
    (Icon::Osnap, "osnap", "Object Snap (F3)", None),
    (Icon::OTrack, "otrack", "Object Snap Tracking (F11)", None),
    (Icon::Grid, "grid", "Grid (F7)", None),
    (Icon::Snap, "snap", "Snap Mode (F9)", None),
    (Icon::Ortho, "ortho", "Ortho Mode (F8)", None),
    (Icon::Polar, "polar", "Polar Tracking (F10)", None),
];

/// The command groups of a shipped tool bar; the groups are the separators between runs of buttons.
pub fn builtin_groups(id: &str) -> Option<Vec<Vec<Tool>>> {
    let from_tool_sets = |name: &str| {
        palettes::DRAFTING
            .iter()
            .find(|(group, ..)| *group == name)
            .map(|(_, large, small)| vec![large.iter().chain(small.iter()).map(|(icon, command, label)| (*icon, *command, *label, None)).collect()])
    };
    match id {
        "standard" => Some(STANDARD.iter().map(|group| group.to_vec()).collect()),
        "draw" => from_tool_sets("Draw"),
        "modify" => from_tool_sets("Modify"),
        "dimension" => from_tool_sets("Dimension"),
        "snap" => Some(vec![SNAP.to_vec()]),
        _ => None,
    }
}

/// Command ids of a shipped tool bar in display order; empty for an id that is not shipped.
pub fn builtin_toolbar_commands(id: &str) -> Vec<&'static str> {
    builtin_groups(id).map(|groups| groups.iter().flatten().map(|tool| tool.1).collect()).unwrap_or_default()
}

/// What a button shows for a command id.
#[derive(Clone, Debug)]
pub(crate) struct Command {
    pub label: String,
    pub icon: Option<Icon>,
    pub shortcut: Option<&'static str>,
    /// False for an id this build does not run: the profile keeps it, the button is a placeholder (D17).
    pub known: bool,
}

/// Ids that name features a later version brings; they get a readable title in the placeholder.
const PLANNED: &[(&str, &str)] = &[("sheetset", "Sheet Set")];

/// Every tool of the shipped bars, then of the Tool Sets: where an icon and a label can be found for an id.
fn known_tools() -> &'static [Tool] {
    static TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        let shipped = BUILTIN.iter().filter_map(|(bar, _)| builtin_groups(bar)).flatten().flatten();
        let palette = palettes::DRAFTING.iter().chain(palettes::MODELING.iter()).flat_map(|(_, large, small)| large.iter().chain(small.iter()));
        shipped.chain(palette.map(|(icon, command, label)| (*icon, *command, *label, None))).collect()
    })
}

pub(crate) fn resolve(id: &str) -> Command {
    if let Some((icon, _, label, shortcut)) = known_tools().iter().find(|tool| tool.1 == id) {
        return Command { label: (*label).to_string(), icon: Some(*icon), shortcut: *shortcut, known: true };
    }
    if crate::wsprofile::apply::command_known(id) {
        let label = cadcraft_engine::find_command(id)
            .map(|spec| spec.label.to_string())
            .or_else(|| crate::menus::UI_COMMANDS.iter().find(|entry| entry.0 == id).map(|entry| entry.1.to_string()))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| title_of(id));
        return Command { label, icon: None, shortcut: None, known: true };
    }
    Command { label: title_of(id), icon: None, shortcut: None, known: false }
}

/// "sheetset" is "Sheet Set"; any other id is split at `.`, `_`, `-` and lower-to-upper case changes.
fn title_of(id: &str) -> String {
    if let Some((_, title)) = PLANNED.iter().find(|(planned, _)| *planned == id) {
        return (*title).to_string();
    }
    let mut words: Vec<String> = Vec::new();
    let mut previous_lower = false;
    for ch in id.chars() {
        if matches!(ch, '.' | '_' | '-') || ch.is_whitespace() {
            words.push(String::new());
            previous_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower {
            words.push(String::new());
        }
        if words.is_empty() {
            words.push(String::new());
        }
        if let Some(word) = words.last_mut() {
            word.push(ch);
        }
        previous_lower = ch.is_lowercase();
    }
    let words: Vec<String> = words
        .into_iter()
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        })
        .collect();
    if words.is_empty() { id.to_string() } else { words.join(" ") }
}

/// The text of a placeholder button and its tool tip.
pub(crate) fn placeholder_label(command: &Command) -> String {
    crate::tf!("{title} — not available in this version", title = command.label)
}

/// The window's tool bars in drawing order for `dock`: by row, then by position in the row.
pub(crate) fn docked(toolbars: &[ToolbarState], dock: Dock) -> Vec<&ToolbarState> {
    let mut bars: Vec<&ToolbarState> = toolbars.iter().filter(|bar| bar.dock == dock).collect();
    bars.sort_by_key(|bar| (bar.row, bar.order));
    bars
}

/// A tool bar this build ships, as the profile would hold it when first shown.
fn shipped_bar(id: &str, toolbars: &[ToolbarState]) -> Option<ToolbarState> {
    let (_, label) = BUILTIN.iter().find(|(shipped, _)| *shipped == id)?;
    let next_row = docked(toolbars, Dock::Top).last().map_or(0, |bar| bar.row.saturating_add(1));
    Some(ToolbarState {
        id: id.to_string(),
        label: (*label).to_string(),
        dock: Dock::Top,
        row: next_row,
        commands: builtin_toolbar_commands(id).iter().map(|command| (*command).to_string()).collect(),
        ..ToolbarState::default()
    })
}

/// Ids a command may name: the profile's tool bars, then the shipped ones it does not hold yet.
pub(crate) fn known_ids(app: &CadApp) -> Vec<String> {
    let mut ids: Vec<String> = app.ws.applied.toolbars.iter().map(|bar| bar.id.clone()).collect();
    let missing: Vec<String> = BUILTIN.iter().filter(|(id, _)| !ids.iter().any(|held| held == id)).map(|(id, _)| (*id).to_string()).collect();
    ids.extend(missing);
    ids
}

fn ratio(params: &Value, key: &str, default: f32) -> Result<f32, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value.as_f64().filter(|v| v.is_finite()).map(|v| (v as f32).clamp(0.0, 1.0)).ok_or_else(|| format!("{key} must be a number")),
    }
}

fn index(params: &Value, key: &str) -> Result<Option<u32>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().and_then(|v| u32::try_from(v).ok()).map(Some).ok_or_else(|| format!("{key} must be a whole number")),
    }
}

fn edge(params: &Value) -> Result<Dock, String> {
    match params.get("dock").and_then(Value::as_str) {
        Some("top") => Ok(Dock::Top),
        Some("bottom") => Ok(Dock::Bottom),
        Some("left") => Ok(Dock::Left),
        Some("right") => Ok(Dock::Right),
        _ => Err("dock must be top, bottom, left or right".into()),
    }
}

/// Where a bar goes when a command names an edge but not the row or position.
fn append_to(toolbars: &[ToolbarState], moving: &str, dock: Dock) -> (u32, u32) {
    let others: Vec<&ToolbarState> = docked(toolbars, dock).into_iter().filter(|bar| bar.id != moving).collect();
    let Some(last) = others.last() else { return (0, 0) };
    let in_row = others.iter().filter(|bar| bar.row == last.row).count();
    (last.row, u32::try_from(in_row).unwrap_or(u32::MAX))
}

fn edit(toolbars: &mut Vec<ToolbarState>, op: &str, id: &str, params: &Value, frozen: &[String]) -> Result<(), String> {
    let mut at = toolbars.iter().position(|bar| bar.id == id);
    if at.is_none()
        && op != "hide"
        && let Some(bar) = shipped_bar(id, toolbars)
    {
        toolbars.push(bar);
        at = toolbars.len().checked_sub(1);
    }
    let Some(i) = at else {
        if op == "hide" && BUILTIN.iter().any(|(shipped, _)| *shipped == id) {
            return Ok(());
        }
        return Err(format!("Unknown toolbar: {}", id.chars().take(64).collect::<String>()));
    };
    let locked = toolbars.get(i).is_some_and(|bar| bar.locked);
    if locked && matches!(op, "dock" | "float" | "move") {
        return Err("The toolbar is locked; unlock it first".into());
    }
    match op {
        "hide" | "show" | "lock" | "unlock" => {
            let Some(bar) = toolbars.get_mut(i) else { return Ok(()) };
            match op {
                "hide" => bar.visible = false,
                "show" => bar.visible = true,
                "lock" => bar.locked = true,
                _ => bar.locked = false,
            }
        }
        "dock" => {
            let dock = edge(params)?;
            let (row, order) = append_to(toolbars, id, dock);
            let (row, order) = (index(params, "row")?.unwrap_or(row), index(params, "order")?.unwrap_or(order));
            drag::apply_drop(toolbars, id, &DropTarget::Edge { dock, row, order }, frozen)?;
            show(toolbars, i);
        }
        "float" => {
            let (x, y) = toolbars.get(i).map_or((0.1, 0.12), |bar| if bar.dock == Dock::Floating { (bar.x_ratio, bar.y_ratio) } else { (0.1, 0.12) });
            let pos_ratio = [ratio(params, "x_ratio", x)?, ratio(params, "y_ratio", y)?];
            drag::apply_drop(toolbars, id, &DropTarget::Float { pos_ratio }, frozen)?;
            show(toolbars, i);
        }
        _ => {
            if params.get("dock").and_then(Value::as_str) == Some("floating") {
                let pos_ratio = [ratio(params, "x_ratio", 0.1)?, ratio(params, "y_ratio", 0.12)?];
                drag::apply_drop(toolbars, id, &DropTarget::Float { pos_ratio }, frozen)?;
            } else {
                let dock = edge(params)?;
                let (row, order) = append_to(toolbars, id, dock);
                let (row, order) = (index(params, "row")?.unwrap_or(row), index(params, "order")?.unwrap_or(order));
                drag::apply_drop(toolbars, id, &DropTarget::Edge { dock, row, order }, frozen)?;
            }
            show(toolbars, i);
        }
    }
    Ok(())
}

/// A bar that is docked or floated by a command is meant to be seen.
fn show(toolbars: &mut [ToolbarState], at: usize) {
    if let Some(bar) = toolbars.get_mut(at) {
        bar.visible = true;
    }
}

/// `ui.toolbar.hide|show|lock|unlock|dock|float|move`; the Window ▸ Toolbars items carry the id in the command.
pub fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let rest = id.strip_prefix(COMMAND_PREFIX)?;
    let (op, inline) = match rest.split_once('.') {
        Some((op, target)) => (op, Some(target)),
        None => (rest, None),
    };
    if !matches!(op, "hide" | "show" | "lock" | "unlock" | "dock" | "float" | "move") {
        return None;
    }
    let target = params.get("id").and_then(Value::as_str).or(inline).map(str::trim).filter(|target| !target.is_empty());
    let Some(target) = target else { return Some(Err(format!("id is required: {}", known_ids(app).join(", ")))) };
    let target = target.to_string();
    let frozen = drag::frozen_orders(app);
    let changed = crate::wsprofile::edit_toolbars(app, |toolbars| edit(toolbars, op, &target, params, &frozen));
    Some(changed.map(|changed| json!({"id": target, "changed": changed})))
}
