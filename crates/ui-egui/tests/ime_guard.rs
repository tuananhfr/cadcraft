//! IME composition and focused editors keep drafting aliases on the command line.

use cadcraft_engine::{Input, Session};
use cadcraft_geom::Vec2;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, cmdline};
use egui::accesskit::Role;
use egui::{Event, ImeEvent, Key, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness() -> Harness<'static, CadApp> {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    let at = h.state().canvas.rect.unwrap().center();
    h.event(Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
    // A blank-canvas click can begin a selection window, which must not absorb the test's Enter.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.ctx.egui_wants_keyboard_input());
    assert!(h.state().session.pending_window.is_none());
    h
}

fn running(app: &CadApp) -> Option<&str> {
    app.session.running.as_ref().map(|command| command.id.as_str())
}

fn preedit(text: &str) -> Event {
    Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: None })
}

#[test]
fn ime_composition_never_dispatches_alias() {
    let mut h = harness();
    h.event(preedit("l"));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
    assert!(h.state().cmd.buffer.is_empty());

    h.event(preedit("la"));
    h.event(Event::Text(" ".into()));
    h.key_press(Key::Space);
    h.key_press(Key::Escape);
    h.run_steps(2);
    let mut raw = egui::RawInput::default();
    for pressed in [true, false] {
        raw.events.push(Event::Key { key: Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    cmdline::capture_tab(&h.ctx, &mut raw);
    h.input_mut().events.extend(raw.events);
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
    assert!(h.state().cmd.buffer.is_empty());

    h.event(Event::Ime(ImeEvent::Commit("lá".into())));
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
    assert_eq!(h.state().cmd.buffer, "lá");

    h.event(preedit("lo"));
    h.event(Event::Ime(ImeEvent::Commit("lo\u{031b}\u{0301}p".into())));
    h.run_steps(2);
    assert_eq!(h.state().cmd.buffer, "lálớp");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().cmd.buffer.is_empty());
    assert_eq!(h.state().cmd.history.last().map(String::as_str), Some("lálớp"));

    for end in [Event::Ime(ImeEvent::Commit(String::new())), preedit("")] {
        h.event(preedit("l"));
        h.event(end);
        h.event(Event::Text("L".into()));
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert_eq!(running(h.state()), Some("line"));
        h.key_press(Key::Escape);
        h.run_steps(2);
    }
}

#[test]
fn ime_enter_during_composition_never_repeats_last_command() {
    let mut h = harness();
    h.state_mut().start("line");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().session.last_command.as_deref(), Some("line"));
    assert_eq!(running(h.state()), None);

    h.input_mut().events.extend([
        preedit("o"),
        Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
    assert!(h.state().cmd.buffer.is_empty());

    h.event(preedit(""));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(running(h.state()), Some("line"));
    h.event(preedit("o"));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(running(h.state()), Some("line"));
    h.event(Event::Ime(ImeEvent::Commit(String::new())));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(running(h.state()), None);
}

#[test]
fn typed_aliases_dispatch_after_commit_h07() {
    let mut h = harness();
    for (alias, id) in [("L", "line"), ("PL", "pline"), ("CO", "copy")] {
        h.event(Event::Text(alias.into()));
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert_eq!(running(h.state()), Some(id), "{alias}");
        assert!(h.state().cmd.buffer.is_empty());
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(running(h.state()), None);
    }
}

#[test]
fn typing_in_text_fields_never_dispatches_alias_tc06() {
    let mut dispatched = Vec::new();
    // The Quick Access "Search commands..." box feeds the command line by design, so it is not a text field here.
    for label in ["Find command", "Filter"] {
        let mut h = harness();
        match label {
            "Find command" => {
                h.state_mut().run("ui.customize.open", json!({})).unwrap();
                h.run_steps(3);
                h.get_by_role_and_label(Role::Button, "Keyboard").click();
                h.run_steps(3);
                h.get_by_role_and_label(Role::TextInput, label).click();
            }
            "Filter" => {
                h.key_press(Key::F1);
                h.run_steps(3);
                // F1 has a visible Filter label but no accessible label on its text input.
                let row_y = h.get_by_label(label).rect().center().y;
                h.query_all_by_role(Role::TextInput).find(|node| node.rect().y_range().contains(row_y)).unwrap().click();
            }
            _ => unreachable!("{label}"),
        }
        h.run_steps(2);
        assert!(h.ctx.text_edit_focused(), "{label}");
        h.event(Event::Text("L".into()));
        h.key_press(Key::Enter);
        h.run_steps(3);
        if let Some(id) = running(h.state()) {
            dispatched.push((label, id.to_string()));
        }
    }
    assert!(dispatched.is_empty(), "text fields dispatched aliases: {dispatched:?}");
}

#[test]
fn vietnamese_in_mtext_editor_does_not_run_alias_h08() {
    let mut h = harness();
    h.state_mut().start("mtext");
    h.state_mut().session.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    h.state_mut().session.input(Input::Point(Vec2::new(10.0, 5.0))).unwrap();
    h.run_steps(3);
    assert!(h.ctx.text_edit_focused());
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    h.event(Event::Text("l".into()));
    h.event(Event::Ime(ImeEvent::Commit("ừ".into())));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(running(h.state()), Some("mtext"));
    assert!(h.state().cmd.buffer.is_empty());
    assert!(h.get_by_role(Role::MultilineTextInput).value().is_some_and(|text| text.contains("lừ")));
}
