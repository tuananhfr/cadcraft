use std::cell::Cell;
use std::rc::Rc;

use cadcraft_engine::{Input, Outcome, Session, View};
use cadcraft_geom::Vec2;
use cadcraft_ui_egui::fullscreen::{self, FullscreenHost, Phase};
use cadcraft_ui_egui::wsprofile::store::{MemoryStore, ProfileStore};
use cadcraft_ui_egui::{CadApp, Services, focus, menus, shortcuts};
use egui::{Event, ImeEvent, Key, Modifiers, ViewportCommand, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

#[derive(Clone, Default)]
struct FakeHost {
    requests: Rc<Cell<usize>>,
    exits: Rc<Cell<usize>>,
    active: Rc<Cell<bool>>,
    reject: bool,
}

impl FullscreenHost for FakeHost {
    fn request(&self) -> Result<(), String> {
        self.requests.set(self.requests.get() + 1);
        if self.reject { Err("rejected".into()) } else { Ok(()) }
    }

    fn exit(&self) {
        self.exits.set(self.exits.get() + 1);
        self.active.set(false);
    }

    fn is_active(&self) -> bool {
        self.active.get()
    }
}

fn app() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.ui.start_tab = false;
    app
}

fn with_host(host: &FakeHost) -> CadApp {
    let mut app = app();
    app.services.fullscreen = Some(Box::new(host.clone()));
    app
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn running(app: &CadApp) -> Option<&str> {
    app.session.running.as_ref().map(|command| command.id.as_str())
}

fn viewport_commands(mut output: egui::FullOutput) -> Vec<bool> {
    output.textures_delta.clear(); // no renderer here
    output
        .viewport_output
        .values()
        .flat_map(|viewport| &viewport.commands)
        .filter_map(|command| match command {
            ViewportCommand::Fullscreen(on) => Some(*on),
            _ => None,
        })
        .collect()
}

#[test]
fn fullscreen_keeps_camera_selection_and_dirty_flag_tc03_h04() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.run("ui.sample", json!({})).unwrap();
    app.canvas.zoom_pending = false;
    app.run("line", json!({"points": [[0, 0], [10, 10]]})).unwrap();
    app.session.state_mut().unwrap().set_view_quiet(View { center: Vec2::new(12.0, -7.0), height: 42.0 });
    app.start("line");
    let handles = app.session.doc().unwrap().model.handles().into_iter().take(2).collect();
    app.session.set_selection(handles);
    let view = app.session.state().unwrap().view();
    let selection = app.session.selection();
    assert_eq!(selection.len(), 2);
    assert!(app.session.state().unwrap().is_dirty());
    let drawing = app.session.state().unwrap().doc.clone();
    let undo = app.session.state().unwrap().undo.len();
    let on = ctx.run_ui(egui::RawInput::default(), |_| {
        assert_eq!(fullscreen::toggle(&mut app, &ctx, false), Outcome::Success);
    });
    assert_eq!(viewport_commands(on), [true]);
    let off = ctx.run_ui(egui::RawInput::default(), |_| {
        assert_eq!(fullscreen::toggle(&mut app, &ctx, false), Outcome::Success);
    });
    assert_eq!(viewport_commands(off), [false]);
    assert_eq!(app.fullscreen.phase, Phase::Off);
    assert_eq!(app.session.state().unwrap().view(), view);
    assert_eq!(app.session.selection(), selection);
    assert!(app.session.state().unwrap().is_dirty());
    assert!(std::sync::Arc::ptr_eq(&drawing, &app.session.state().unwrap().doc));
    assert_eq!(app.session.state().unwrap().undo.len(), undo);
    assert_eq!(running(&app), Some("line"));
}

#[test]
fn web_fullscreen_requires_a_user_gesture() {
    let host = FakeHost::default();
    let mut app = with_host(&host);
    let ctx = egui::Context::default();
    assert_eq!(fullscreen::toggle(&mut app, &ctx, false), Outcome::Unsupported);
    assert_eq!(app.dispatch("ui.fullscreen", json!({})).outcome, Outcome::Unsupported);
    assert_eq!(host.requests.get(), 0);
    assert_eq!(host.exits.get(), 0);
    assert_eq!(app.fullscreen.phase, Phase::Off);
    assert!(!app.focus.active());
    assert_eq!(fullscreen::toggle(&mut app, &ctx, true), Outcome::Success);
    assert_eq!(host.requests.get(), 1);
    assert_eq!(app.fullscreen.phase, Phase::Requested);
    assert_eq!(fullscreen::toggle(&mut app, &ctx, false), Outcome::Unsupported);
    assert_eq!(app.fullscreen.phase, Phase::Requested);
    assert_eq!(host.exits.get(), 0);
}

#[test]
fn rejected_fullscreen_falls_back_to_focus_mode() {
    let host = FakeHost { reject: true, ..Default::default() };
    let mut app = with_host(&host);
    let view = app.session.state().unwrap().view();
    assert_eq!(fullscreen::toggle(&mut app, &egui::Context::default(), true), Outcome::Success);
    assert_eq!(host.requests.get(), 1);
    assert_eq!(app.fullscreen.phase, Phase::Off);
    assert!(app.fullscreen.fallback_focus);
    assert!(app.focus.active());
    assert!(app.status.as_ref().unwrap().0.contains("Focus mode"));
    assert_eq!(app.session.state().unwrap().view(), view);
}

#[test]
fn browser_exit_resets_the_phase() {
    let host = FakeHost::default();
    let mut app = with_host(&host);
    let ctx = egui::Context::default();
    fullscreen::toggle(&mut app, &ctx, true);
    fullscreen::sync(&mut app, &ctx);
    assert_eq!(app.fullscreen.phase, Phase::Requested, "an asynchronous request must wait for confirmation");
    host.active.set(true);
    fullscreen::sync(&mut app, &ctx);
    assert_eq!(app.fullscreen.phase, Phase::On);
    host.active.set(false);
    fullscreen::sync(&mut app, &ctx);
    fullscreen::sync(&mut app, &ctx);
    assert_eq!(app.fullscreen.phase, Phase::Off);
    assert_eq!(host.exits.get(), 0);
    assert!(!app.focus.active());
}

#[test]
fn escape_exits_desktop_fullscreen_only_when_idle() {
    let mut h = harness(app());
    h.state_mut().fullscreen.phase = Phase::On;
    h.state_mut().start("line");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
    assert_eq!(h.state().fullscreen.phase, Phase::On);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().fullscreen.phase, Phase::Off);
}

#[test]
fn focus_mode_gives_the_canvas_85_percent_of_the_height() {
    for size in [vec2(1366.0, 768.0), vec2(683.0, 384.0)] {
        let mut app = app();
        app.attach_profile_store(Box::new(MemoryStore::default()));
        app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
        for command in ["ui.toggle.toolsets", "ui.toggle.palettes", "ui.toggle.toolbar"] {
            app.run(command, json!({"on": true})).unwrap();
        }
        assert!(app.ui.show_toolsets && app.ui.show_palettes && app.ui.show_toolbar);
        let mut h = harness(app);
        h.set_size(size);
        h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
        h.run_steps(4);
        let canvas = h.state().canvas.rect.unwrap();
        assert!(canvas.height() >= 0.85 * size.y, "{canvas:?} at {size:?}");
        assert!(canvas.width() >= 0.85 * size.x, "{canvas:?} at {size:?}");
        let exit = h.get_by_label("Exit Focus Mode (Esc)");
        assert!(h.ctx.content_rect().contains_rect(exit.rect()));
        assert!(h.query_by_label("Command line input").is_some());
        assert!(h.query_by_label("Command line grip").is_none());
        for part in [
            focus::Chrome::Ribbon,
            focus::Chrome::Panels,
            focus::Chrome::Toolbars,
            focus::Chrome::FileTabs,
            focus::Chrome::StatusBar,
            focus::Chrome::MenuBar,
            focus::Chrome::QuickAccess,
        ] {
            assert!(focus::hides(h.state(), part));
        }
        h.get_by_label("Exit Focus Mode (Esc)").click();
        h.run_steps(3);
        assert!(!h.state().focus.active());
    }
}

struct CountingStore {
    inner: MemoryStore,
    writes: Rc<Cell<usize>>,
}

impl ProfileStore for CountingStore {
    fn read_current(&self) -> Option<String> {
        self.inner.read_current()
    }
    fn write_current(&mut self, json: &str) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        self.inner.write_current(json)
    }
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        self.inner.push_snapshot(revision, json)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.inner.snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.inner.read_snapshot(revision)
    }
}

#[test]
fn focus_mode_restores_the_exact_layout() {
    let writes = Rc::new(Cell::new(0));
    let mut app = app();
    app.attach_profile_store(Box::new(CountingStore { inner: MemoryStore::default(), writes: writes.clone() }));
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h = harness(app);
    h.state_mut().commit_workspace_now().unwrap();
    let before = serde_json::to_value(&h.state().ws.effective.layout).unwrap();
    let docking = serde_json::to_value(&h.state().ui.docking).unwrap();
    let profile = h.state().profile_json();
    let count = writes.get();
    let undo = h.state().ws.history.undo_len();
    h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
    h.run_steps(4);
    assert_eq!(serde_json::to_value(&h.state().ws.effective.layout).unwrap(), before);
    assert_eq!(serde_json::to_value(&h.state().ui.docking).unwrap(), docking);
    h.state_mut().run("ui.focusmode", json!({"on": false})).unwrap();
    h.run_steps(4);
    assert_eq!(serde_json::to_value(&h.state().ws.effective.layout).unwrap(), before);
    assert_eq!(serde_json::to_value(&h.state().ui.docking).unwrap(), docking);
    assert_eq!(h.state().profile_json(), profile);
    assert_eq!(writes.get(), count);
    assert_eq!(h.state().ws.history.undo_len(), undo);
    assert!(!h.state().ws.needs_flush());
}

#[test]
fn escape_cancels_command_before_leaving_focus_tc04() {
    let mut drawings = Vec::new();
    for focused in [false, true] {
        let mut h = harness(app());
        if focused {
            h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
        }
        h.state_mut().start("pline");
        for point in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 5.0)] {
            h.state_mut().session.input(Input::Point(point)).unwrap();
        }
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(running(h.state()), None);
        assert_eq!(h.state().focus.active(), focused);
        drawings.push(h.state().session.doc().unwrap().clone());
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(!h.state().focus.active());
    }
    assert_eq!(drawings[0], drawings[1]);
}

#[test]
fn ime_composition_never_leaves_focus_mode() {
    let mut h = harness(app());
    h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
    h.event(Event::Ime(ImeEvent::Preedit { text: "lớp".into(), active_range_chars: None }));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().focus.active());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().focus.active());
    h.event(Event::Ime(ImeEvent::Commit(String::new())));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().focus.active());
}

#[test]
fn resize_in_focus_mode_keeps_the_camera_h05() {
    let mut h = harness(app());
    h.state_mut().session.state_mut().unwrap().set_view_quiet(View { center: Vec2::new(12.0, -7.0), height: 42.0 });
    let view = h.state().session.state().unwrap().view();
    let dirty = h.state().session.state().unwrap().is_dirty();
    h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
    for size in [vec2(1366.0, 768.0), vec2(1920.0, 1080.0), vec2(1366.0, 768.0)] {
        h.set_size(size);
        h.run_steps(3);
        assert_eq!(h.state().session.state().unwrap().view(), view);
        assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
    }
}

#[test]
fn cleanscreen_aliases_toggle_focus_mode() {
    let mut app = app();
    assert_eq!(app.run("cleanscreenon", json!({})).unwrap(), json!({"on": true}));
    assert!(app.cmdline("cleanscreenoff").is_none());
    assert!(!app.focus.active());
    assert!(app.cmdline("ui.focusmode {\"on\":true,\"command_line\":\"hidden\"}").is_none());
    assert!(app.focus.active());
    assert_eq!(app.focus.command_line, focus::CommandLineInFocus::Hidden);
    app.run("ui.focusmode", json!({})).unwrap();
    assert!(!app.focus.active());
    for id in ["ui.fullscreen", "ui.focusmode", "cleanscreenon", "cleanscreenoff"] {
        assert!(menus::is_ui_command(id));
        assert!(include_str!("../../../docs/workspace/command-ids.md").contains(&format!("`{id}`")));
    }
}

#[test]
fn focus_escape_clears_selection_and_dialog_before_exiting() {
    let mut h = harness(app());
    h.state_mut().run("ui.sample", json!({})).unwrap();
    h.run_steps(3);
    let selection = h.state().session.doc().unwrap().model.handles().into_iter().take(2).collect();
    h.state_mut().session.set_selection(selection);
    h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().session.selection().is_empty());
    assert!(h.state().focus.active());
    h.state_mut().run("ui.customize.open", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialog.is_none());
    assert!(h.state().focus.active());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().focus.active());
}

#[test]
fn focus_hidden_command_line_and_desktop_keys() {
    let mut h = harness(app());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    h.run_steps(3);
    assert!(h.state().focus.active());
    h.state_mut().run("ui.focusmode", json!({"on": true, "command_line": "hidden"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Command line input").is_none());
    assert!(h.ctx.content_rect().contains_rect(h.state().canvas.rect.unwrap()));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().focus.active());
    let defaults = shortcuts::defaults_for(shortcuts::KeyPlatform::Desktop);
    assert_eq!(defaults.get("ui.fullscreen").map(String::as_str), Some("Ctrl+Shift+F"));
    assert_eq!(defaults.get("ui.focusmode").map(String::as_str), Some("Ctrl+0"));
    assert_eq!(defaults.get("otrack").map(String::as_str), Some("F11"));
    let web = shortcuts::defaults_for(shortcuts::KeyPlatform::Web);
    assert!(!web.contains_key("ui.fullscreen"));
    assert!(!web.contains_key("ui.focusmode"));
}

#[test]
fn asynchronous_fullscreen_rejection_and_control_requests() {
    let host = FakeHost::default();
    let mut app = with_host(&host);
    let ctx = egui::Context::default();
    fullscreen::toggle(&mut app, &ctx, true);
    assert_eq!(app.fullscreen.phase, Phase::Requested);
    fullscreen::rejected(&mut app);
    assert!(app.focus.active());
    assert_eq!(app.fullscreen.phase, Phase::Off);
    assert!(app.cmdline("ui.fullscreen").is_some());
    for method in ["engine.execute", "ui.menu.invoke", "cmdline.input"] {
        let (request, _) = cadcraft_ui_egui::control::ControlRequest::new(method, json!({"command": "ui.fullscreen", "text": "ui.fullscreen"}));
        match cadcraft_ui_egui::control::handle(&mut app, &ctx, &request) {
            cadcraft_ui_egui::control::Outcome::Done(value) => {
                assert_eq!(value["outcome"], "unsupported");
                assert_eq!(value["ok"], false);
            }
            _ => panic!("unexpected screenshot"),
        }
    }
    assert_eq!(host.requests.get(), 1);
    assert_eq!(host.exits.get(), 0);
}

#[test]
fn web_fullscreen_menu_click_is_a_user_gesture() {
    let host = FakeHost::default();
    let mut app = with_host(&host);
    app.ui.in_window_menu = true;
    let mut h = harness(app);
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Full Screen").click();
    h.run_steps(2);
    assert_eq!(host.requests.get(), 1);
    assert_eq!(h.state().fullscreen.phase, Phase::Requested);
    host.active.set(true);
    h.run_steps(2);
    assert_eq!(h.state().fullscreen.phase, Phase::On);
}

#[test]
fn focus_without_a_drawing_still_exits_on_escape() {
    let mut app = CadApp::new(Session::empty(), Services::default());
    app.run("ui.focusmode", json!({"on": true})).unwrap();
    let mut h = harness(app);
    assert!(h.state().focus.active());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().focus.active());
}
