//! The command line: history, the active prompt with clickable keywords, typed input
//! (keyboard focus stays on the drawing — typing anywhere goes here) and AutoComplete.

use std::{collections::HashSet, sync::Arc};

use cadcraft_engine::Input;
use egui::{Key, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::CadApp;
use crate::theme::Tokens;

#[derive(Default)]
pub struct CmdLine {
    pub buffer: String,
    ime_composing: bool,
    /// The selection as of the last frame, to notice when it changes.
    seen_selection: Vec<cadcraft_doc::Handle>,
    /// Something was typed at the command line since the selection was made.
    typed_since_selection: bool,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    /// What was typed at command prompts (not command names), oldest first: the in-command
    /// shortcut menu's Recent Input.
    pub recent_input: Vec<String>,
    /// The expanded history window is open (F2, the chevron, or a click on the history lines).
    pub expanded: bool,
    /// Dragging the history's top edge: (pointer y, line count) when the drag started.
    resize_from: Option<(f32, usize)>,
    resize_preview: Option<usize>,
    move_preview: Option<Pos2>,
    /// The Dynamic Input options menu at the cursor (Down Arrow at a prompt with keywords).
    pub options: Option<crate::dyninput::OptionsMenu>,
    /// The text whose second Dynamic Input value Tab locked; the lock holds while it is unchanged.
    locked_second: Option<String>,
    /// Where the options menu's rows were drawn last frame (screen), for clicks on them.
    pub option_rows: Vec<(Rect, String)>,
    /// Where the Dynamic Input tooltip stays while the options menu is open.
    pub options_anchor: Option<Pos2>,
}

/// The most history lines the command line shows above its bar.
pub const MAX_LINES: usize = 12;
const LINE_H: f32 = 16.0;
const INPUT_HEIGHT: f32 = 24.0;
const ROW_PADDING: f32 = 14.0;
pub(crate) const INPUT_ROW_HEIGHT: f32 = INPUT_HEIGHT + ROW_PADDING;
pub(crate) const MIN_CANVAS_HEIGHT: f32 = 150.0;

/// The Dynamic Input frame for the prompt on screen, or `None` when the value boxes are not live
/// (Dynamic Input off, no point prompt or a pending selection window).
pub fn dyn_frame(app: &CadApp) -> Option<crate::dyninput::Frame> {
    let st = &app.session.settings;
    if !st.dynmode || app.session.pending_window.is_some() {
        return None;
    }
    let p = app.session.current_prompt().filter(cadcraft_engine::pointmod::accepts)?;
    Some(crate::dyninput::Frame { base: p.base, cartesian: st.dynpi_cartesian, absolute: st.dynpi_absolute })
}

/// Take Tab away from egui before it moves keyboard focus to the next widget (which would leave
/// the command line deaf to typing). While no widget has focus, a pressed Tab becomes a `"\t"`
/// text event, which keeps its place among the other keystrokes; [`keyboard`] handles it.
pub fn capture_tab(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    raw.events.retain_mut(|e| {
        let tab = match e {
            egui::Event::Key { key: Key::Tab, pressed, .. } => Some(*pressed),
            _ => None,
        };
        match tab {
            Some(true) => {
                *e = egui::Event::Text("\t".into());
                true
            }
            Some(false) => false,
            None => true,
        }
    });
}

/// Tab on the command line: cycle the object snaps under the cursor, type the Dynamic Input
/// separator (`,` or `<`) after a value, or complete a command name.
fn tab(app: &mut CadApp) {
    // Grip dimensional input: Tab locks the typed value and moves to the next field.
    if crate::grip_dyn::tab(app) {
        return;
    }
    // Over a snap marker with nothing typed, Tab cycles the candidate snaps.
    if app.cmd.buffer.is_empty() && app.canvas.snap.is_some() {
        app.canvas.snap_tab = true;
    } else if let Some(f) = dyn_frame(app)
        && let Some(sep) = crate::dyninput::tab_separator(&app.cmd.buffer, &f)
    {
        app.cmd.buffer.push(sep);
    } else if let Some(f) = dyn_frame(app)
        && crate::dyninput::can_lock_second(&app.cmd.buffer, &f)
    {
        // Both values typed: Tab locks the second one as well.
        app.cmd.locked_second = Some(app.cmd.buffer.clone());
    } else if let Some(s) = crate::quick_access::search(app, &app.cmd.buffer).first() {
        app.cmd.buffer = s.id.clone();
    }
}

/// An AutoComplete suggestion for the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// Command identifier (uppercase, e.g. "RECTANG"), used for buffer completion and execution.
    pub id: String,
    /// Display text shown in the AutoComplete list (e.g. "REC (RECTANG)" or "RECOVER").
    pub display: String,
    /// Human-readable command label (e.g. "Rectangle").
    pub label: &'static str,
}

/// Matching command names/aliases for AutoComplete.
pub(crate) fn all_suggestions(prefix: &str) -> Vec<Suggestion> {
    if prefix.is_empty() || prefix.contains(' ') {
        return Vec::new();
    }
    let p = prefix.to_ascii_lowercase();

    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen_ids: HashSet<&'static str> = HashSet::new();

    // 1. Exact match: alias resolution or exact command name match.
    let resolved = cadcraft_engine::cmd::resolve_alias(&p);
    if let Some(spec) = cadcraft_engine::cmd::find_command(&resolved)
        && !spec.id.contains('.')
    {
        let display =
            if spec.id == p { spec.id.to_ascii_uppercase() } else { format!("{} ({})", p.to_ascii_uppercase(), spec.id.to_ascii_uppercase()) };
        out.push(Suggestion { id: spec.id.to_ascii_uppercase(), display, label: spec.label });
        seen_ids.insert(spec.id);
    }

    // 2. Command ID prefix matches (excluding already matched exact command).
    let mut id_matches: Vec<&'static cadcraft_engine::CommandSpec> =
        cadcraft_engine::command_specs().iter().filter(|c| !c.id.contains('.') && !seen_ids.contains(c.id) && c.id.starts_with(&p)).collect();
    id_matches.sort_by_key(|c| (c.id.len(), c.id));
    for c in id_matches {
        out.push(Suggestion { id: c.id.to_ascii_uppercase(), display: c.id.to_ascii_uppercase(), label: c.label });
        seen_ids.insert(c.id);
    }

    // 3. Command alias prefix matches (for commands not yet added).
    let mut alias_matches: Vec<(&'static str, &'static cadcraft_engine::CommandSpec)> = Vec::new();
    for c in cadcraft_engine::command_specs() {
        if c.id.contains('.') || seen_ids.contains(c.id) {
            continue;
        }
        let matching_alias = c.aliases.iter().copied().filter(|a| a.starts_with(&p)).min_by_key(|a| (a.len(), *a));
        if let Some(alias) = matching_alias {
            alias_matches.push((alias, c));
        }
    }
    alias_matches.sort_by_key(|(alias, c)| (alias.len(), *alias, c.id));
    for (alias, c) in alias_matches {
        if seen_ids.insert(c.id) {
            out.push(Suggestion {
                id: c.id.to_ascii_uppercase(),
                display: format!("{} ({})", alias.to_ascii_uppercase(), c.id.to_ascii_uppercase()),
                label: c.label,
            });
        }
    }

    out
}

impl CmdLine {
    pub(crate) fn ime_composing(&self) -> bool {
        self.ime_composing
    }

    /// Track the selection: a new selection starts with nothing typed since.
    pub fn watch_selection(&mut self, sel: &[cadcraft_doc::Handle]) {
        if self.seen_selection != sel {
            self.seen_selection = sel.to_vec();
            self.typed_since_selection = false;
        }
    }

    /// Note that the command line holds typed text.
    pub fn note_typing(&mut self) {
        if !self.buffer.is_empty() {
            self.typed_since_selection = true;
        }
    }

    /// Tab locked the second Dynamic Input value and nothing was typed since.
    pub fn second_locked(&self) -> bool {
        self.locked_second.as_deref() == Some(self.buffer.as_str())
    }

    /// The options menu row under `at` (screen), if the menu is open.
    pub fn option_at(&self, at: Pos2) -> Option<String> {
        self.options.as_ref()?;
        self.option_rows.iter().find(|(r, _)| r.contains(at)).map(|(_, k)| k.clone())
    }
}

/// Keep the options menu only while its prompt is on screen with Dynamic Input and DYNPROMPT on.
fn options_still_fit(app: &mut CadApp) {
    let st = &app.session.settings;
    let on = st.dynmode && st.dynprompt && app.canvas.hot_grips.is_empty();
    let kws = app.session.current_prompt().map(|p| p.keywords).unwrap_or_default();
    if app.cmd.options.as_ref().is_some_and(|m| !on || !m.fits(&kws)) {
        app.cmd.options = None;
    }
}

/// Down Arrow at a prompt with keywords (Dynamic Input on, DYNPROMPT on, not browsing the
/// history): open the options menu at the cursor, or move its highlight. `false` when the key is
/// left for the history.
fn options_key(app: &mut CadApp, step: isize) -> bool {
    if let Some(m) = app.cmd.options.as_mut() {
        m.step(step);
        return true;
    }
    let st = &app.session.settings;
    if step < 0 || !st.dynmode || !st.dynprompt || app.cmd.history_pos.is_some() || !app.canvas.hot_grips.is_empty() {
        return false;
    }
    let kws = app.session.current_prompt().map(|p| p.keywords).unwrap_or_default();
    app.cmd.options = crate::dyninput::OptionsMenu::open(&kws);
    app.cmd.options.is_some()
}

/// Send a keyword picked from the options menu, as a click on it at the command line does.
pub fn pick_option(app: &mut CadApp, keyword: String) {
    app.cmd.options = None;
    app.cmd.buffer.clear();
    if let Err(e) = app.session.input(Input::Keyword(keyword)) {
        app.session.echo(e.to_string());
    }
}

/// Whether Backspace should erase the selection instead of editing the command line. The key
/// labelled "delete" on a Mac sends Backspace, so on macOS it erases selected objects, but only
/// when it can't be meant for the command line: a fresh, plain press (never an auto-repeat, so
/// holding it to clear typing can't run on into the drawing, and not Option/Cmd+delete, which
/// delete a word or line of text on macOS), no command running, the command line empty, and
/// nothing typed there since the selection was made.
pub fn backspace_erases(mac: bool, press: KeyPress, buffer: &str, command_running: bool, has_selection: bool, typed_since_selection: bool) -> bool {
    mac && press.plain && !press.repeat && buffer.is_empty() && !command_running && has_selection && !typed_since_selection
}

/// How a key was pressed.
#[derive(Clone, Copy, Debug)]
pub struct KeyPress {
    /// The operating system's auto-repeat while the key is held.
    pub repeat: bool,
    /// No Option, Control or Command held (Shift doesn't matter).
    pub plain: bool,
}

/// Handle keyboard input destined for the command line (when no text field has focus).
pub fn keyboard(app: &mut CadApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let selection = app.session.selection();
    app.cmd.watch_selection(&selection);
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    let text_prompt = app.session.current_prompt().is_some_and(|p| p.accept.text && !p.accept.point && !p.accept.number);
    options_still_fit(app);
    for ev in events {
        match &ev {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                app.cmd.ime_composing = !text.is_empty();
                continue;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                app.cmd.buffer.push_str(&crate::i18n::fold::nfc(text));
                app.cmd.ime_composing = false;
                app.cmd.note_typing();
                continue;
            }
            _ => {}
        }
        // capture_tab turns Tab into text before egui can move focus away from the canvas.
        if app.cmd.ime_composing()
            && (matches!(&ev, egui::Event::Key { key: Key::Enter | Key::Space | Key::Escape | Key::Tab, .. })
                || matches!(&ev, egui::Event::Text(text) if text == " " || text == "\t"))
        {
            continue;
        }
        // The options menu takes the arrows, Enter and Escape; typing closes it.
        if app.cmd.options.is_some() {
            let space = matches!(&ev, egui::Event::Text(t) if t == " ") && !text_prompt;
            match &ev {
                _ if space => {
                    if let Some(k) = app.cmd.options.as_ref().and_then(|m| m.current()).map(str::to_owned) {
                        pick_option(app, k);
                    }
                    continue;
                }
                egui::Event::Key { key: Key::Enter, pressed: true, .. } => {
                    if let Some(k) = app.cmd.options.as_ref().and_then(|m| m.current()).map(str::to_owned) {
                        pick_option(app, k);
                    }
                    continue;
                }
                egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                    app.cmd.options = None;
                    continue;
                }
                egui::Event::Key { key: Key::ArrowUp, pressed: true, modifiers, .. } if !modifiers.any() => {
                    options_key(app, -1);
                    continue;
                }
                egui::Event::Key { key: Key::ArrowDown | Key::ArrowUp, pressed: true, .. } => {}
                // Space arrives as a key and as text; the text picks.
                egui::Event::Key { key: Key::Space, .. } => {}
                egui::Event::Text(_) | egui::Event::Key { pressed: true, .. } => app.cmd.options = None,
                _ => {}
            }
        }
        match ev {
            egui::Event::Text(t) => {
                if t == "\t" {
                    tab(app);
                } else if t == " " && !text_prompt {
                    submit(app);
                } else {
                    app.cmd.buffer.push_str(&t);
                }
            }
            egui::Event::Key { key, pressed: true, repeat, modifiers, .. } => match key {
                Key::Enter => submit(app),
                Key::Escape => crate::focus::escape(app, ctx),
                Key::Backspace => {
                    let press = KeyPress { repeat, plain: !(modifiers.alt || modifiers.ctrl || modifiers.command) };
                    let erase = backspace_erases(
                        mac,
                        press,
                        &app.cmd.buffer,
                        app.session.prompting(),
                        !app.session.selection().is_empty(),
                        app.cmd.typed_since_selection,
                    );
                    if erase {
                        let _ = app.run("erase.selection", serde_json::json!({}));
                    } else {
                        app.cmd.buffer.pop();
                    }
                }
                Key::ArrowUp if !modifiers.any() => {
                    if !app.cmd.history.is_empty() {
                        let n = app.cmd.history.len();
                        let pos = app.cmd.history_pos.map(|p| p.saturating_sub(1)).unwrap_or(n - 1);
                        app.cmd.history_pos = Some(pos);
                        app.cmd.buffer = app.cmd.history.get(pos).cloned().unwrap_or_default();
                    }
                }
                Key::ArrowDown if !modifiers.any() => {
                    if !options_key(app, 1)
                        && let Some(p) = app.cmd.history_pos
                    {
                        let np = p + 1;
                        if np >= app.cmd.history.len() {
                            app.cmd.history_pos = None;
                            app.cmd.buffer.clear();
                        } else {
                            app.cmd.history_pos = Some(np);
                            app.cmd.buffer = app.cmd.history.get(np).cloned().unwrap_or_default();
                        }
                    }
                }
                Key::Tab => tab(app),
                Key::Delete if app.cmd.buffer.is_empty() && !app.session.prompting() => {
                    let _ = app.run("erase.selection", serde_json::json!({}));
                }
                _ => {}
            },
            _ => {}
        }
        app.cmd.note_typing();
    }
}

pub fn submit(app: &mut CadApp) {
    // Grip dimensional input: the typed lengths and angles place the grip.
    if crate::grip_dyn::submit(app) {
        return;
    }
    // Dynamic Input: a split entry (`40,30`, `40<90`, `40,`) is completed from the cursor and
    // measured from the last point.
    if let Some(f) = dyn_frame(app) {
        let cursor = app.canvas.cursor.unwrap_or(app.session.cursor);
        let last = app.session.last_point;
        let done = crate::dyninput::parse(&app.cmd.buffer, &f).and_then(|e| crate::dyninput::complete(&e, &f, cursor, last));
        if let Some(text) = done {
            app.cmd.buffer = text;
        }
    }
    let text = std::mem::take(&mut app.cmd.buffer);
    let text = crate::i18n::fold::nfc(&text).into_owned();
    app.cmd.locked_second = None;
    app.cmd.history_pos = None;
    if !text.trim().is_empty() && !app.session.prompting() {
        app.cmd.history.push(text.clone());
        if app.cmd.history.len() > 200 {
            app.cmd.history.remove(0);
        }
    }
    crate::context_menu::note_input(app, &text);
    if text.is_empty() {
        if let Err(e) = app.session.input(Input::Enter) {
            app.session.echo(e.to_string());
        }
        return;
    }
    app.cmdline(&text);
}

fn visible_lines(app: &CadApp, canvas: Rect, bar: Rect) -> usize {
    let available = ((bar.top() - canvas.top() - 8.0).max(0.0) / LINE_H) as usize;
    history_lines(app).min(MAX_LINES).min(available)
}

fn history_lines(app: &CadApp) -> usize {
    app.cmd.resize_preview.unwrap_or(app.ui.history_lines)
}

pub(crate) fn docked_height(app: &CadApp, screen_height: f32) -> f32 {
    let ratio = app.ws.effective.layout.command_line.height_ratio;
    let minimum = history_lines(app).min(MAX_LINES) as f32 * LINE_H + INPUT_ROW_HEIGHT;
    let requested = screen_height * ratio;
    let requested = if let Some(lines) = app.cmd.resize_preview {
        let original = app.ui.history_lines.min(MAX_LINES) as f32;
        let original_height = requested.max(original * LINE_H + INPUT_ROW_HEIGHT);
        original_height + (lines as f32 - original) * LINE_H
    } else {
        requested
    };
    if requested.is_finite() { requested.max(minimum) } else { minimum }
}

/// Docked command lines reserve a row before the drawing is laid out.
pub fn show_docked(app: &mut CadApp, ui: &mut egui::Ui) {
    use crate::wsprofile::schema::Dock;
    let dock = app.ws.effective.layout.command_line.dock;
    let panel = match dock {
        Dock::Top => egui::Panel::top("cc_cmdline_top"),
        Dock::Bottom => egui::Panel::bottom("cc_cmdline_bottom"),
        _ => return,
    };
    let free = ui.available_rect_before_wrap();
    let height = docked_height(app, ui.ctx().content_rect().height());
    let height = height.clamp(INPUT_ROW_HEIGHT, (free.height() - MIN_CANVAS_HEIGHT).max(INPUT_ROW_HEIGHT));
    panel.resizable(false).exact_size(height).frame(egui::Frame::NONE.fill(Tokens::get().cmd_history)).show(ui, |ui| {
        let area = ui.max_rect();
        let bar = Rect::from_min_size(pos2(area.left() + 4.0, area.bottom() - INPUT_HEIGHT - 4.0), vec2((area.width() - 8.0).max(0.0), INPUT_HEIGHT));
        show_at(app, ui, area, bar);
    });
}

fn dock_menu(app: &mut CadApp, response: &egui::Response) {
    egui::Popup::menu(response).show(|ui| {
        for (dock, label) in [("top", "Dock command line at top"), ("bottom", "Dock command line at bottom"), ("floating", "Float command line")] {
            if ui.button(crate::i18n::t(label)).clicked() {
                if let Err(error) = app.run("ui.workspace.set", serde_json::json!({"pointer": "/command_line/dock", "value": dock})) {
                    app.set_status(error);
                }
                ui.close();
            }
        }
    });
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui, canvas: Rect) {
    if app.focus.active() {
        if app.focus.command_line == crate::focus::CommandLineInFocus::Hidden {
            return;
        }
        let width = (canvas.width() - 16.0).clamp(0.0, 760.0);
        let bar = Rect::from_min_size(pos2(canvas.center().x - width / 2.0, canvas.bottom() - INPUT_HEIGHT - 8.0), vec2(width, INPUT_HEIGHT));
        show_at(app, ui, canvas, bar);
        return;
    }
    if app.ws.effective.layout.command_line.dock != crate::wsprofile::schema::Dock::Floating {
        return;
    }
    let w = (canvas.width() * 0.48).clamp(360.0, 760.0).min((canvas.width() - 8.0).max(0.0));
    let h = INPUT_HEIGHT;
    let bar = Rect::from_min_size(pos2(canvas.center().x - w / 2.0, canvas.bottom() - h - 10.0), vec2(w, h));
    let state = &app.ws.effective.layout.command_line;
    let bar = if state.x_ratio == 0.0 && state.y_ratio == 0.0 {
        bar
    } else {
        let top = canvas.top() + (history_lines(app).min(MAX_LINES) as f32 * LINE_H + 8.0).min((canvas.height() - h).max(0.0));
        let left = canvas.left() + state.x_ratio * (canvas.width() - w).max(0.0);
        let y = top + state.y_ratio * (canvas.bottom() - h - top).max(0.0);
        Rect::from_min_size(pos2(left, y), vec2(w, h))
    };
    let bar = app.cmd.move_preview.map_or(bar, |position| Rect::from_min_size(position, bar.size()));
    show_at(app, ui, canvas, bar);
}

fn prompt_galley(painter: &egui::Painter, text: &str, font: &egui::FontId, color: egui::Color32, width: f32) -> Arc<egui::Galley> {
    let layout = |text| painter.layout_no_wrap(text, font.clone(), color);
    let full = layout(text.to_string());
    if full.size().x <= width {
        return full;
    }
    let mut best = layout(String::new());
    let ellipsis = layout("…".into());
    if ellipsis.size().x > width {
        return best;
    }
    best = ellipsis;
    // Bound candidate allocations even when a prompt contains a hostile file name.
    let head: String = text.chars().take(86).collect();
    let tail: Vec<char> = text.chars().rev().take(171).collect();
    let mut low = 0;
    let mut high = text.chars().take(257).count().saturating_sub(1).min(256);
    while low < high {
        let keep = low + (high - low).div_ceil(2);
        // The end of a prompt carries what the user must answer, so it gets two thirds of the room.
        let mut candidate: String = head.chars().take(keep / 3).collect();
        candidate.push('…');
        candidate.extend(tail.iter().take(keep - keep / 3).rev().copied());
        let galley = layout(candidate);
        if galley.size().x <= width {
            best = galley;
            low = keep;
        } else {
            high = keep - 1;
        }
    }
    best
}

fn show_at(app: &mut CadApp, ui: &mut egui::Ui, canvas: Rect, bar: Rect) {
    let t = Tokens::get();
    let (w, h) = (bar.width(), bar.height());
    let p = ui.painter_at(canvas);
    if !app.focus.active() {
        history(app, ui, canvas, bar);
    }
    let input = ui.interact(bar, ui.id().with("cmdline_input"), Sense::hover());
    input.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, crate::tl!("Command line input")));
    crate::control::record_widget(ui, &input, "Command line input", "label", None);
    // History lines above the bar (the expanded window replaces them).
    let n = if app.cmd.expanded || app.focus.active() { 0 } else { visible_lines(app, canvas, bar) };
    let lines: Vec<String> = app.session.log.iter().rev().take(n).rev().cloned().collect();
    if !lines.is_empty() {
        let lh = 16.0;
        let hr = Rect::from_min_size(pos2(bar.left(), bar.top() - lh * lines.len() as f32 - 4.0), vec2(w, lh * lines.len() as f32 + 2.0));
        p.rect_filled(hr, 3.0, t.cmd_history);
        for (i, l) in lines.iter().enumerate() {
            p.text(pos2(hr.left() + 8.0, hr.top() + 1.0 + lh * i as f32), egui::Align2::LEFT_TOP, l, crate::theme::small(), t.text_dim);
        }
    }
    p.rect_filled(bar, 3.0, t.cmd_bg);
    p.rect_stroke(bar, 3.0, Stroke::new(1.0, t.cmd_border), egui::StrokeKind::Inside);
    if !app.focus.active() {
        let grip_rect = Rect::from_min_size(pos2(bar.left(), bar.top() + 4.0), vec2(7.0, 16.0));
        let grip = ui.interact(grip_rect, ui.id().with("cmdline_move"), Sense::click_and_drag());
        crate::icons::describe_control(ui, &grip, "Command line grip", None, false);
        for y in [5.0, 10.0, 15.0] {
            p.circle_filled(pos2(bar.left() + 3.0, bar.top() + y), 0.8, t.text_dim);
        }
        dock_menu(app, &grip);
        if app.ws.effective.layout.command_line.dock == crate::wsprofile::schema::Dock::Floating {
            if grip.drag_started() {
                app.cmd.move_preview = Some(bar.min);
            }
            if grip.dragged() {
                let position = app.cmd.move_preview.unwrap_or(bar.min) + grip.drag_delta();
                let top = canvas.top() + (history_lines(app).min(MAX_LINES) as f32 * LINE_H + 8.0).min((canvas.height() - h).max(0.0));
                app.cmd.move_preview = Some(pos2(
                    position.x.clamp(canvas.left(), (canvas.right() - w).max(canvas.left())),
                    position.y.clamp(top, (canvas.bottom() - h).max(top)),
                ));
            }
            if grip.drag_stopped()
                && let Some(position) = app.cmd.move_preview.take()
            {
                let top = canvas.top() + (history_lines(app).min(MAX_LINES) as f32 * LINE_H + 8.0).min((canvas.height() - h).max(0.0));
                let x_room = (canvas.width() - w).max(0.0);
                let y_room = (canvas.bottom() - h - top).max(0.0);
                let x = if x_room > 0.0 { ((position.x - canvas.left()) / x_room).clamp(0.0, 1.0) } else { 0.5 };
                let y = if y_room > 0.0 { ((position.y - top) / y_room).clamp(0.0, 1.0) } else { 1.0 };
                let mut state = app.ws.effective.layout.command_line.clone();
                // Zero coordinates retain the legacy default placement.
                state.x_ratio = if x == 0.0 && y == 0.0 { f32::EPSILON } else { x };
                state.y_ratio = y;
                if let Err(error) = app.run("ui.workspace.set", serde_json::json!({"pointer": "/command_line", "value": state})) {
                    app.set_status(error);
                }
                app.ws.autosave.interaction_ended();
            }
        }
    }
    let p = p.with_clip_rect(bar.intersect(ui.clip_rect()));
    // Prompt glyph.
    p.text(pos2(bar.left() + 8.0, bar.center().y), egui::Align2::LEFT_CENTER, ">_", crate::theme::mono(), t.text_dim);
    let mut x = (bar.left() + 30.0).min(bar.right());
    let input_right = (bar.right() - if app.focus.active() { 6.0 } else { 28.0 }).max(x);
    let font = crate::theme::body();
    // The input grows from a caret-sized slot to 120 px as the user types, so an empty answer leaves the prompt more room.
    let typed_width = p.layout_no_wrap(app.cmd.buffer.clone(), font.clone(), t.text).size().x;
    let input_room = (typed_width + 24.0).clamp(60.0, 120.0);
    let prompt_right = (input_right - input_room.min((input_right - x) * 0.5)).max(x);
    let prompt_rect = Rect::from_min_max(pos2(x, bar.top()), pos2(prompt_right, bar.bottom()));
    let prompt = app.session.current_prompt();
    match &prompt {
        Some(pr) => {
            // The grip modes name themselves ("** STRETCH **…").
            let name = app.session.running.as_ref().filter(|r| r.id != "grip").map(|r| r.id.to_ascii_uppercase() + " ").unwrap_or_default();
            // A LISP `get…` prompt shows as the routine wrote it (no keywords, default or colon added).
            let mut message = if pr.raw { pr.display() } else { crate::i18n::prompt_message(pr) };
            let mut tail = Vec::new();
            if pr.raw {
                // A routine that follows the "… [Yes/No] <No>: " convention keeps its answers visible like a native prompt.
                if let Some(at) = message.rfind(" [").filter(|&at| message[at..].chars().count() <= 64) {
                    let answers = message.split_off(at);
                    tail.push((p.layout_no_wrap(answers, font.clone(), t.text), None));
                }
            } else {
                if !pr.keywords.is_empty() {
                    let opening = if pr.message.is_empty() { " [".into() } else { crate::tl!(" or [").into() };
                    tail.push((p.layout_no_wrap(opening, font.clone(), t.text), None));
                    for (i, keyword) in pr.keywords.iter().enumerate() {
                        if i > 0 {
                            tail.push((p.layout_no_wrap("/".into(), font.clone(), t.text), None));
                        }
                        tail.push((p.layout_no_wrap(keyword.clone(), font.clone(), t.cmd_keyword), Some(keyword)));
                    }
                    tail.push((p.layout_no_wrap("]".into(), font.clone(), t.text), None));
                }
                if let Some(default) = &pr.default {
                    tail.push((p.layout_no_wrap(format!(" <{default}>"), font.clone(), t.text), None));
                }
                tail.push((p.layout_no_wrap(": ".into(), font.clone(), t.text), None));
            }
            let tail_width: f32 = tail.iter().map(|(galley, _)| galley.size().x).sum();
            let prefix_width = (prompt_right - x - tail_width).max(0.0);
            // The command name is context only; it gives way before the message is cut to nothing.
            let combined_width = p.layout_no_wrap(format!("{name}{message}"), font.clone(), t.text).size().x;
            let shown_name = if combined_width > prefix_width && p.layout_no_wrap(name.clone(), font.clone(), t.text).size().x > prefix_width / 3.0 {
                ""
            } else {
                name.as_str()
            };
            let name_galley = prompt_galley(&p, shown_name, &font, t.text_faint, prefix_width);
            let message_galley = prompt_galley(&p, &message, &font, t.text, (prefix_width - name_galley.size().x).max(0.0));
            let full = format!("{name}{message}{}", tail.iter().map(|(galley, _)| galley.text()).collect::<String>());
            let visible =
                format!("{}{}{}", name_galley.text(), message_galley.text(), tail.iter().map(|(galley, _)| galley.text()).collect::<String>());
            let pp = p.with_clip_rect(prompt_rect.intersect(p.clip_rect()));
            for galley in [name_galley, message_galley] {
                let width = galley.size().x;
                pp.galley(pos2(x, bar.center().y - galley.size().y / 2.0), galley, t.text);
                x += width;
            }
            // An unusually wide keyword list still keeps its default at the right edge.
            x = x.min(prompt_right - tail_width);
            for (i, (galley, keyword)) in tail.into_iter().enumerate() {
                let width = galley.size().x;
                if let Some(keyword) = keyword {
                    let rect = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(width, h - 6.0)).intersect(prompt_rect);
                    if rect.is_positive() {
                        let response = ui.interact(rect, ui.id().with(("kw", i)), Sense::click());
                        if response.hovered() {
                            pp.rect_filled(rect, 2.0, t.cmd_keyword_hover);
                        }
                        if response.clicked() {
                            let _ = app.session.input(Input::Keyword(keyword.clone()));
                        }
                    }
                }
                pp.galley(pos2(x, bar.center().y - galley.size().y / 2.0), galley, t.text);
                x += width;
            }
            x = x.clamp(prompt_rect.left(), prompt_right);
            let rect = Rect::from_min_max(prompt_rect.min, pos2(x, bar.bottom()));
            let response = ui.interact(rect, ui.id().with("cmdline_prompt"), Sense::hover());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &visible));
            response.on_hover_text(full);
            if pr.raw && !pr.message.ends_with(' ') {
                x = (x + 4.0).min(prompt_right);
            }
        }
        None => {
            if app.cmd.buffer.is_empty() {
                p.text(
                    pos2(x, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    crate::tl!("Type a command"),
                    egui::FontId::proportional(12.5),
                    t.text_faint,
                );
            }
        }
    }
    let g = p.layout_no_wrap(app.cmd.buffer.clone(), font, t.text);
    let gw = g.size().x;
    let input_rect = Rect::from_min_max(pos2(x, bar.top()), pos2(input_right, bar.bottom()));
    let input_painter = p.with_clip_rect(input_rect.intersect(p.clip_rect()));
    let input_x = x - (gw + 2.0 - input_rect.width()).max(0.0);
    input_painter.galley(pos2(input_x, bar.center().y - g.size().y / 2.0), g, t.text);
    // Caret.
    let blink = (ui.input(|i| i.time) * 2.0).floor() as i64 % 2 == 0;
    if blink {
        input_painter
            .line_segment([pos2(input_x + gw + 1.0, bar.top() + 5.0), pos2(input_x + gw + 1.0, bar.bottom() - 5.0)], Stroke::new(1.0, t.text));
    }
    if !app.focus.active() {
        // History chevron.
        crate::icons::paint(
            &p,
            Rect::from_center_size(pos2(bar.right() - 14.0, bar.center().y), vec2(14.0, 14.0)),
            if app.cmd.expanded { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronUp },
            false,
        );
    }
    autocomplete(app, ui, canvas, bar);
}

fn overlay_rect(app: &CadApp, screen: Rect, bar: Rect, size: egui::Vec2, above: f32) -> Rect {
    let size = size.min(screen.size()).max(vec2(1.0, 1.0));
    let top = if !app.focus.active() && app.ws.effective.layout.command_line.dock == crate::wsprofile::schema::Dock::Top {
        bar.bottom() + 4.0
    } else {
        bar.top() - above - size.y - 4.0
    };
    let left = bar.left().clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let top = top.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
    Rect::from_min_size(pos2(left, top), size)
}

fn autocomplete(app: &mut CadApp, ui: &egui::Ui, canvas: Rect, bar: Rect) {
    if app.session.prompting() {
        return;
    }
    let suggestions = crate::quick_access::search(app, &app.cmd.buffer);
    if suggestions.is_empty() {
        return;
    }
    let n = if app.cmd.expanded || app.focus.active() { 0 } else { visible_lines(app, canvas, bar).min(app.session.log.len()) };
    let above = if n == 0 { 0.0 } else { LINE_H * n as f32 + 6.0 };
    let anchor = bar.translate(vec2(24.0, 0.0));
    let rect = overlay_rect(app, ui.ctx().content_rect(), anchor, vec2((bar.width() - 24.0).max(1.0), 20.0 * suggestions.len() as f32), above);
    egui::Area::new(egui::Id::new("cmdline_autocomplete")).order(egui::Order::Foreground).fixed_pos(rect.min).movable(false).show(ui.ctx(), |ui| {
        let (list, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
        let t = Tokens::get();
        let p = ui.painter_at(list);
        p.rect_filled(list, 3.0, t.list_bg);
        p.rect_stroke(list, 3.0, Stroke::new(1.0, t.cmd_border), egui::StrokeKind::Inside);
        for (i, suggestion) in suggestions.iter().enumerate() {
            let row = Rect::from_min_size(pos2(list.left(), list.top() + 20.0 * i as f32), vec2(list.width(), 20.0));
            let response = ui.interact(row, ui.id().with(&suggestion.id), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &suggestion.display));
            if response.hovered() || i == 0 {
                p.rect_filled(row.shrink(1.0), 2.0, if response.hovered() { t.cmd_keyword_hover } else { t.list_row });
            }
            p.text(pos2(row.left() + 8.0, row.center().y), egui::Align2::LEFT_CENTER, &suggestion.display, crate::theme::body(), t.text);
            p.text(
                pos2(row.right() - 8.0, row.center().y),
                egui::Align2::RIGHT_CENTER,
                crate::i18n::t(suggestion.label),
                crate::theme::small(),
                t.text_faint,
            );
            if response.clicked() {
                app.cmd.buffer.clear();
                app.start(&suggestion.id.to_ascii_lowercase());
            }
        }
    });
}

/// The command line's areas, laid out like [`show`] draws them.
struct Zones {
    /// The inline history lines (when there are any and the window is closed).
    lines: Option<Rect>,
    /// The expanded history window.
    window: Option<Rect>,
    chevron: Rect,
    /// The top edge that resizes the inline history.
    grip: Rect,
    /// The inline controls keep pointer input away from the drawing underneath.
    all: Rect,
}

fn zones(app: &CadApp, canvas: Rect, bar: Rect) -> Zones {
    let n = if app.cmd.expanded { 0 } else { visible_lines(app, canvas, bar).min(app.session.log.len()) };
    let lines =
        (n > 0).then(|| Rect::from_min_size(pos2(bar.left(), bar.top() - LINE_H * n as f32 - 4.0), vec2(bar.width(), LINE_H * n as f32 + 2.0)));
    let window = app.cmd.expanded.then(|| {
        // As tall as the log (at least three lines), at most half the drawing area.
        let full = LINE_H * app.session.log.len().max(3) as f32 + 4.0;
        let h = full.min((canvas.height() * 0.5).max(3.0 * LINE_H));
        Rect::from_min_size(pos2(bar.left(), bar.top() - h - 4.0), vec2(bar.width(), h + 2.0))
    });
    let top = window.or(lines).map_or(bar.top(), |r| r.top());
    let grip = Rect::from_min_max(pos2(bar.left(), top - 3.0), pos2(bar.right(), top + 3.0));
    let all = lines.map_or(bar.union(grip), |lines| bar.union(grip).union(lines));
    Zones { lines, window, chevron: Rect::from_center_size(pos2(bar.right() - 14.0, bar.center().y), vec2(24.0, 22.0)), grip, all }
}

/// The pointer on the command line: one widget under all of it keeps clicks, drags and the wheel
/// off the drawing (the autocomplete overlay handles its own input). Clicking the history
/// lines or the chevron opens/closes the history window (`ui.toggle.history`, also F2); dragging
/// the top edge commits the line count and height together (`ui.workspace.set`).
fn history(app: &mut CadApp, ui: &mut egui::Ui, canvas: Rect, bar: Rect) {
    let z = zones(app, canvas, bar);
    let resp = ui.interact(z.all, ui.id().with("cmdline"), Sense::click_and_drag());
    let at = resp.hover_pos().or(resp.interact_pointer_pos());
    let on_grip = |p: Pos2| z.grip.contains(p) && z.window.is_none();
    let on_toggle = |p: Pos2| z.chevron.contains(p) || z.lines.is_some_and(|r| r.contains(p));
    if app.cmd.resize_from.is_some() || at.is_some_and(on_grip) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    } else if at.is_some_and(on_toggle) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
        && on_grip(o)
    {
        app.cmd.resize_from = Some((o.y, visible_lines(app, canvas, bar)));
    }
    if let Some((y0, n0)) = app.cmd.resize_from {
        if let (true, Some(position)) = (resp.dragged(), resp.interact_pointer_pos()) {
            let direction = if app.ws.effective.layout.command_line.dock == crate::wsprofile::schema::Dock::Top { 1.0 } else { -1.0 };
            let lines = (n0 as f32 + direction * (position.y - y0) / LINE_H).round().clamp(0.0, MAX_LINES as f32) as usize;
            app.cmd.resize_preview = Some(lines);
        }
        if resp.drag_stopped() {
            app.cmd.resize_from = None;
            let resized_height = docked_height(app, ui.ctx().content_rect().height());
            if let Some(lines) = app.cmd.resize_preview.take() {
                let mut state = app.ws.effective.layout.command_line.clone();
                state.lines = lines as u32;
                if state.dock != crate::wsprofile::schema::Dock::Floating {
                    let height = ui.ctx().content_rect().height();
                    if height.is_finite() && height > 0.0 {
                        state.height_ratio = (resized_height / height).clamp(0.0, 1.0);
                    }
                }
                if let Err(error) = app.run("ui.workspace.set", serde_json::json!({"pointer": "/command_line", "value": state})) {
                    app.set_status(error);
                }
                app.ws.autosave.interaction_ended();
            }
        }
    }
    if resp.clicked() && resp.interact_pointer_pos().is_some_and(on_toggle) {
        let _ = app.run("ui.toggle.history", serde_json::json!({}));
    }
    crate::shortcut_menus::attach(app, &resp, crate::shortcut_menus::command_line_entries);
    if let Some(r) = zones(app, canvas, bar).window {
        let rect = overlay_rect(app, ui.ctx().content_rect(), bar, r.size(), 0.0);
        egui::Area::new(egui::Id::new("cmdline_history_window")).order(egui::Order::Foreground).fixed_pos(rect.min).movable(false).show(
            ui.ctx(),
            |ui| {
                let (rect, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
                history_window(app, ui, rect);
            },
        );
    }
}

/// The expanded history: the whole command log, scrollable, newest at the bottom.
fn history_window(app: &CadApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get();
    ui.painter().rect_filled(r, 3.0, t.cmd_bg);
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let inner = r.shrink2(vec2(8.0, 3.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    let _ = history_rows(&mut child, app, false);
}

pub(crate) fn history_body(ui: &mut egui::Ui, app: &mut CadApp) {
    if let Some(command) = history_rows(ui, app, true) {
        app.start(&command);
    }
}

fn history_rows(ui: &mut egui::Ui, app: &CadApp, replay: bool) -> Option<String> {
    let t = Tokens::get();
    let log = &app.session.log;
    let mut repeat = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("cmd_history").stick_to_bottom(true).auto_shrink([false, false]).show_rows(
            ui,
            LINE_H,
            log.len(),
            |ui, rows| {
                for line in log.get(rows).into_iter().flatten() {
                    let command = line.strip_prefix("Command: ").filter(|name| replay && !name.trim().is_empty());
                    let sense = if command.is_some() { Sense::click() } else { Sense::hover() };
                    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), LINE_H), sense);
                    let p = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
                    p.text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, line, crate::theme::small(), t.text_dim);
                    if let Some(command) = command {
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), line));
                        if response.double_clicked() {
                            repeat = Some(command.to_string());
                        }
                        response.on_hover_text(crate::tl!("Double-click to repeat command"));
                    }
                }
            },
        );
    });
    repeat
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_engine::Session;
    use egui::{Event, PointerButton};

    #[test]
    fn submit_normalizes_typed_text_to_nfc() {
        let mut app = CadApp::new(Session::new(), crate::Services::default());
        app.cmd.buffer = "lo\u{031b}\u{0301}p".into();
        submit(&mut app);
        assert_eq!(app.cmd.history.last().map(String::as_str), Some("lớp"));
        assert!(app.cmd.buffer.is_empty());
    }

    #[test]
    fn compact_canvas_contains_history_controls_without_changing_line_preference() {
        let mut app = CadApp::new(Session::empty(), crate::Services::default());
        app.ui.history_lines = 12;
        for i in 0..20 {
            app.session.echo(format!("line {i}"));
        }
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 180.0));
        let w = (canvas.width() * 0.48).clamp(360.0, 760.0).min((canvas.width() - 8.0).max(0.0));
        let bar = Rect::from_min_size(pos2(canvas.center().x - w / 2.0, canvas.bottom() - 34.0), vec2(w, 24.0));
        let z = zones(&app, canvas, bar);
        assert!(canvas.contains_rect(bar));
        assert!(canvas.contains_rect(z.chevron));
        assert!(z.lines.is_some_and(|r| canvas.contains_rect(r)));
        assert!(canvas.contains_rect(z.grip));
        assert_eq!(app.ui.history_lines, 12);
    }

    fn frame(app: &mut CadApp, ctx: &egui::Context, events: Vec<Event>) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }

    fn press(app: &mut CadApp, ctx: &egui::Context, at: Pos2, pressed: bool) {
        frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() }]);
    }

    #[test]
    fn history_expands_by_click_chevron_f2_and_resizes_by_drag_without_touching_the_drawing() {
        let mut app = CadApp::new(Session::empty(), crate::Services::default());
        app.start("ui.sample");
        for i in 0..20 {
            app.session.echo(format!("line {i}"));
        }
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, Vec::new());
        frame(&mut app, &ctx, Vec::new());
        let canvas = app.canvas.rect.unwrap_or(Rect::NOTHING);
        let w = (canvas.width() * 0.48).clamp(360.0, 760.0).min((canvas.width() - 8.0).max(0.0));
        let bar = Rect::from_min_size(pos2(canvas.center().x - w / 2.0, canvas.bottom() - 34.0), vec2(w, 24.0));
        let z = zones(&app, canvas, bar);
        let Some(lines) = z.lines else { panic!("three history lines show by default") };
        // A click on the history lines opens the history window, and doesn't start a selection.
        for (at, pressed) in [(lines.center(), true), (lines.center(), false)] {
            frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
            press(&mut app, &ctx, at, pressed);
        }
        frame(&mut app, &ctx, Vec::new());
        assert!(app.cmd.expanded);
        assert!(app.session.pending_window.is_none(), "the click stayed on the command line");
        // The chevron closes it; F2 and TEXTSCR open it.
        let chevron = z.chevron.center();
        press(&mut app, &ctx, chevron, true);
        press(&mut app, &ctx, chevron, false);
        frame(&mut app, &ctx, Vec::new());
        assert!(!app.cmd.expanded);
        let f2 = |pressed| Event::Key { key: Key::F2, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, vec![f2(true), f2(false)]);
        assert!(app.cmd.expanded);
        assert_eq!(app.run("ui.toggle.history", serde_json::json!({"on": false})), Ok(serde_json::json!({"on": false})));
        app.cmdline("textscr");
        assert!(app.cmd.expanded);
        let _ = app.run("ui.toggle.history", serde_json::json!({"on": false}));
        frame(&mut app, &ctx, Vec::new());
        // Dragging the top edge up by two lines shows two more lines (capped).
        let top = pos2(lines.center().x, lines.top() - 1.0);
        frame(&mut app, &ctx, vec![Event::PointerMoved(top)]);
        press(&mut app, &ctx, top, true);
        for dy in [8.0, 20.0, 2.0 * LINE_H] {
            frame(&mut app, &ctx, vec![Event::PointerMoved(top - vec2(0.0, dy))]);
        }
        press(&mut app, &ctx, top - vec2(0.0, 2.0 * LINE_H), false);
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(app.ui.history_lines, 5);
        assert!(!app.cmd.expanded, "a drag isn't a click");
        assert!(app.session.pending_window.is_none());
        assert_eq!(app.run("ui.cmdline.lines", serde_json::json!({"lines": 99})), Ok(serde_json::json!({"lines": MAX_LINES})));
    }
}

#[cfg(test)]
mod delete_key_tests {
    use super::*;
    use cadcraft_doc::Handle;

    const PRESS: KeyPress = KeyPress { repeat: false, plain: true };

    #[test]
    fn mac_delete_erases_a_fresh_selection() {
        assert!(backspace_erases(true, PRESS, "", false, true, false));
        // Not on Windows/Linux, where Backspace never deletes objects.
        assert!(!backspace_erases(false, PRESS, "", false, true, false));
        // Nothing selected, a command running, or text on the command line: just edit text.
        assert!(!backspace_erases(true, PRESS, "", false, false, false));
        assert!(!backspace_erases(true, PRESS, "", true, true, false));
        assert!(!backspace_erases(true, PRESS, "m", false, true, false));
        // Option/Cmd+delete are text-editing gestures on macOS.
        assert!(!backspace_erases(true, KeyPress { plain: false, ..PRESS }, "", false, true, false));
    }

    #[test]
    fn holding_delete_to_clear_typing_never_erases() {
        // Auto-repeat after the last character is gone.
        assert!(!backspace_erases(true, KeyPress { repeat: true, ..PRESS }, "", false, true, false));
        // Typed, then cleared with separate presses: still not erased.
        assert!(!backspace_erases(true, PRESS, "", false, true, true));
    }

    #[test]
    fn typing_is_forgotten_when_the_selection_changes() {
        let mut c = CmdLine::default();
        c.watch_selection(&[Handle(1)]);
        c.buffer.push('m');
        c.note_typing();
        c.buffer.clear();
        c.watch_selection(&[Handle(1)]);
        assert!(c.typed_since_selection);
        c.watch_selection(&[Handle(1), Handle(2)]);
        assert!(!c.typed_since_selection);
    }
}

#[cfg(test)]
mod autocomplete_tests {
    use super::*;

    fn suggestions(prefix: &str) -> Vec<Suggestion> {
        let app = CadApp::new(cadcraft_engine::Session::empty(), crate::Services::default());
        crate::quick_access::search(&app, prefix)
    }

    #[test]
    fn favorite_substring_keeps_exact_alias_first() {
        let mut app = CadApp::new(cadcraft_engine::Session::empty(), crate::Services::default());
        app.ws.effective.layout.favorites.push("hatch".into());
        assert_eq!(crate::quick_access::search(&app, "c").first().map(|entry| entry.id.as_str()), Some("CIRCLE"));
        app.cmd.buffer = "c".into();
        tab(&mut app);
        assert_eq!(app.cmd.buffer, "CIRCLE");
    }

    #[test]
    fn exact_alias_is_first_suggestion_with_alias_display() {
        let rec = suggestions("rec");
        assert!(!rec.is_empty());
        assert_eq!(rec[0].id, "RECTANG");
        assert_eq!(rec[0].display, "REC (RECTANG)");
        assert_eq!(rec[0].label, "Rectangle");
        // RECOVER should follow RECTANG, not precede it
        assert!(rec.iter().any(|s| s.id == "RECOVER"));
        assert_eq!(rec[1].id, "RECOVER");
        assert_eq!(rec[1].display, "RECOVER");

        let l = suggestions("l");
        assert_eq!(l[0].id, "LINE");
        assert_eq!(l[0].display, "L (LINE)");

        let c = suggestions("c");
        assert_eq!(c[0].id, "CIRCLE");
        assert_eq!(c[0].display, "C (CIRCLE)");

        let co = suggestions("co");
        assert_eq!(co[0].id, "COPY");
        assert_eq!(co[0].display, "CO (COPY)");
    }

    #[test]
    fn prefix_without_alias_shows_command_id() {
        let rect = suggestions("rect");
        assert_eq!(rect[0].id, "RECTANG");
        assert_eq!(rect[0].display, "RECTANG");

        let line = suggestions("line");
        assert_eq!(line[0].id, "LINE");
        assert_eq!(line[0].display, "LINE");
    }

    #[test]
    fn tab_autocompletes_alias_to_resolved_command() {
        let mut app = CadApp::new(cadcraft_engine::Session::empty(), crate::Services::default());
        app.cmd.buffer = "rec".to_string();
        tab(&mut app);
        assert_eq!(app.cmd.buffer, "RECTANG");
    }
}
