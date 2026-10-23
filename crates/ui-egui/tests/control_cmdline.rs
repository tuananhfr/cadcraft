//! The app's control channel answers `cmdline.input` and `app.open` with the same reply shape as
//! the headless MCP backend (`cadcraft_mcp::Headless`, tested in `crates/mcp/tests/cmdline_parity.rs`).

use cadcraft_engine::Session;
use cadcraft_ui_egui::control::{ControlRequest, Outcome, handle};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Event, Modifiers, PointerButton, Shape, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const STATE_KEYS: [&str; 7] = ["prompt", "running", "keywords", "accept", "buffer", "history", "historyExpanded"];

fn call(app: &mut CadApp, method: &str, params: Value) -> Value {
    let (req, _reply) = ControlRequest::new(method, params);
    match handle(app, &egui::Context::default(), &req) {
        Outcome::Done(v) => v,
        Outcome::Screenshot { .. } => Value::Null,
    }
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    k.sort();
    k
}

fn want(extra: &[&str]) -> Vec<String> {
    let mut k: Vec<String> = STATE_KEYS.iter().chain(extra).map(|s| s.to_string()).collect();
    k.sort();
    k
}

fn app() -> CadApp {
    CadApp::new(Session::new(), Services::default())
}

#[test]
fn refused_input_is_echoed_to_output_and_reported_in_error() {
    let mut app = app();
    let v = call(&mut app, "cmdline.input", json!({"text": "nosuchcommand"}));
    assert_eq!(v["ok"], true, "{v}");
    let r = &v["result"];
    assert_eq!(keys(r), want(&["output", "error"]), "{r}");
    let e = r["error"].as_str().unwrap();
    assert!(!e.is_empty());
    assert!(r["output"].as_array().unwrap().iter().any(|l| l.as_str() == Some(e)), "{r}");
    assert!(r["history"].as_array().unwrap().iter().any(|l| l.as_str() == Some(e)), "{r}");
}

#[test]
fn accepted_input_has_no_error_and_reports_the_prompt() {
    let mut app = app();
    let v = call(&mut app, "cmdline.input", json!({"text": "circle"}));
    let r = &v["result"];
    assert_eq!(keys(r), want(&["output"]), "{r}");
    assert_eq!(r["running"], "circle");
    assert!(!r["accept"].is_null(), "{r}");
    assert_eq!(keys(&call(&mut app, "cmdline.state", json!({}))["result"]), want(&[]));
}

#[test]
fn open_without_a_path_is_refused_before_the_engine() {
    let mut app = app();
    assert_eq!(call(&mut app, "app.open", json!({})), json!({"ok": false, "error": "missing path", "outcome": "validation_error"}));
    assert_eq!(call(&mut app, "app.open", json!({"path": 3})), json!({"ok": false, "error": "missing path", "outcome": "validation_error"}));
}

fn long_prompt_harness(dock: &str, raw: bool) -> (Harness<'static, CadApp>, String) {
    let mut app = app();
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.ui.start_tab = false;
    app.run("ui.workspace.set", json!({"pointer": "/command_line/dock", "value": dock})).unwrap();
    let name = format!("C:/Projects/{}review.lsp", "Bản vẽ/Довгий шлях/".repeat(16));
    if raw {
        app.session.cmdline(&format!("(getstring \"Load unreviewed LISP file {name}? [Yes/No/Always] <No>: \")")).unwrap();
    } else {
        app.session.lisp_load_interactive(&name, "(princ \"prompt-overflow-regression\")").unwrap();
        assert!(app.session.lisp_review_pending());
    }
    let full = app.session.current_prompt().unwrap().display();
    app.session.echo(full.clone());
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    (h, full)
}

fn assert_prompt_fits(h: &Harness<'_, CadApp>, full: &str) {
    let bar = h.get_by_label("Command line input").rect();
    let prompt = h.get_by_label_contains("[Yes/No/Always]");
    let visible = prompt.accesskit_node().value().unwrap();
    assert!(visible.contains('…'), "{visible}");
    assert!(visible.contains("Load"), "{visible}");
    assert!(visible.contains("[Yes/No/Always] <No>:"), "{visible}");
    assert!(!visible.contains(full), "the displayed prompt is shortened");
    assert!(bar.contains_rect(prompt.rect()), "prompt {:?} escapes bar {bar:?}", prompt.rect());
    assert!(bar.right() - 28.0 - prompt.rect().right() >= 59.5, "an empty answer keeps a caret-sized input slot");
    assert!(h.state().session.log.iter().any(|line| line == full), "history keeps the full prompt");
    assert_eq!(h.state().session.current_prompt().unwrap().display(), full);

    let mut painted = String::new();
    for shape in &h.output().shapes {
        if let Shape::Text(text) = &shape.shape {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            // Only the prompt pieces: canvas labels next to a docked bar are not command-line text.
            if bar.contains(rect.center()) && !text.galley.is_empty() && visible.contains(text.galley.text()) {
                assert!(bar.contains_rect(rect), "painted prompt {rect:?} escapes bar {bar:?}");
                assert!(bar.contains_rect(shape.clip_rect), "prompt painting is clipped to the bar");
                painted.push_str(text.galley.text());
            }
        }
    }
    assert!(painted.contains("[Yes/No/Always] <No>:"), "the tail is painted, not only exposed to accessibility: {painted}");
}

#[test]
fn long_prompt_stays_inside_floating_and_docked_command_line_with_keyword_tail() {
    for dock in ["floating", "top", "bottom"] {
        let (h, full) = long_prompt_harness(dock, false);
        assert_prompt_fits(&h, &full);
    }
}

#[test]
fn long_prompt_hover_keeps_full_text_and_keywords_remain_clickable() {
    let (mut h, full) = long_prompt_harness("floating", false);
    h.get_by_label_contains("[Yes/No/Always]").hover();
    h.run_steps(30);
    // The bar words the keyword list " or [", where the engine's history line uses " [" after a question.
    let message = full.split(" [").next().unwrap();
    let revealed = h.query_all_by_label_contains(message).any(|node| {
        let node = node.accesskit_node();
        [node.label(), node.value()].into_iter().flatten().any(|text| text.contains(message) && text.contains("[Yes/No/Always] <No>:"))
    });
    assert!(revealed, "hover reveals the full prompt");
    let bar = h.get_by_label("Command line input").rect();
    let at = h
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == "No" => {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                bar.contains_rect(rect).then_some(rect.center())
            }
            _ => None,
        })
        .expect("the No keyword is visible inside the bar");
    h.hover_at(at);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(!h.state().session.lisp_review_pending(), "clicking No answers the existing prompt");
}

#[test]
fn long_raw_prompt_and_typed_input_are_clipped_inside_command_line() {
    let (mut h, full) = long_prompt_harness("floating", true);
    assert_prompt_fits(&h, &full);
    let buffer = "typed-input-".repeat(100);
    h.state_mut().cmd.buffer = buffer.clone();
    h.run_steps(2);
    let bar = h.get_by_label("Command line input").rect();
    let prompt = h.get_by_label_contains("[Yes/No/Always]").rect();
    let (text, clip) = h
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == buffer.as_str() => Some((text, shape.clip_rect)),
            _ => None,
        })
        .expect("the input buffer is painted");
    assert!(bar.contains_rect(clip));
    assert!(clip.left() >= prompt.right(), "typed input cannot paint over the prompt");
    assert!(clip.width() >= 119.5, "long input still has a usable viewport");
    let caret_x = text.pos.x + text.galley.size().x + 1.0;
    assert!(caret_x >= clip.left() && caret_x <= clip.right(), "long input scrolls to keep the caret visible");
    assert_eq!(h.state().cmd.buffer, buffer);
}
