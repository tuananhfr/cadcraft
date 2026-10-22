//! Command palette search, platform shortcuts and keyboard interaction.

use cadcraft_engine::Session;
use cadcraft_ui_egui::palette::index::{Hit, MatchKind, SearchIndex};
use cadcraft_ui_egui::shortcuts::{self, KeyPlatform};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, palette, quick_access};
use egui::accesskit::Role;
use egui::{Event, ImeEvent, Key, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn first_id(index: &SearchIndex, query: &str) -> &'static str {
    let hits = index.search(query, &[], &[], 50);
    index.entry(hits.first().expect("a matching command")).expect("a valid hit").id
}

fn harness() -> Harness<'static, CadApp> {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
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

#[test]
fn lop_layer_and_la_all_find_layer() {
    for lang in ["en", "vi"] {
        let index = SearchIndex::build(lang, &shortcuts::defaults());
        for q in ["lop", "layer", "LA", "Lớp", "lớp bản vẽ"] {
            assert_eq!(first_id(&index, q), "layer", "{lang} {q}");
        }
    }
}

#[test]
fn decomposed_query_finds_layer() {
    let index = SearchIndex::build("en", &shortcuts::defaults());
    assert_eq!(first_id(&index, "Lo\u{031B}\u{0301}p"), "layer");
}

#[test]
fn duong_thang_finds_line() {
    let index = SearchIndex::build("en", &shortcuts::defaults());
    assert_eq!(first_id(&index, "duong thang"), "line");
    let hits = index.search("L", &[], &[], 50);
    let hit = hits.first().unwrap();
    assert_eq!(index.entry(hit).unwrap().id, "line");
    assert_eq!(hit.matched, MatchKind::ExactAlias);
}

#[test]
fn fuzzy_and_recent_ranking() {
    let index = SearchIndex::build("vi", &shortcuts::defaults());
    let hits = index.search("lyr", &[], &[], 3);
    let layer = hits.iter().find(|hit| index.entry(hit).is_some_and(|entry| entry.id == "layer")).unwrap();
    assert_eq!(layer.matched, MatchKind::Fuzzy);
    assert!(layer.score > 0);
    let mut session = Session::default();
    session.execute("new", &json!({})).unwrap();
    session.start("circle").unwrap();
    let recent = session.recent_commands(8);
    assert_eq!(recent.first().map(String::as_str), Some("circle"));
    let hits = index.search("c", &recent, &[], 50);
    let ids: Vec<_> = hits.iter().filter_map(|hit| index.entry(hit).map(|entry| entry.id)).collect();
    assert!(ids.iter().position(|id| *id == "circle").unwrap() < ids.iter().position(|id| *id == "copy").unwrap());
    let hits = index.search("", &recent, &[], 50);
    assert_eq!(index.entry(hits.first().unwrap()).unwrap().id, "circle");
    let hits = index.search("", &recent, &["line".into()], 50);
    assert_eq!(index.entry(hits.first().unwrap()).unwrap().id, "line");
}

#[test]
fn hits_carry_icon_shortcut_and_ids() {
    let index = SearchIndex::build("vi", &shortcuts::defaults());
    let hits = index.search("qsave", &[], &[], 1);
    let entry = index.entry(hits.first().unwrap()).unwrap();
    assert_eq!(entry.id, "qsave");
    assert_eq!(entry.shortcut.as_deref(), Some("Ctrl+S"));
    assert_eq!(entry.namespaced_id, "cad.file.qsave");
    assert!(entry.icon.is_some());
    assert_eq!(hits.first().unwrap().matched, MatchKind::ExactId);
    assert_eq!(first_id(&index, "cad.draw.line"), "line");
    assert!(index.entry(&Hit { entry: usize::MAX, score: 0, matched: MatchKind::ExactId }).is_none());
}

#[test]
fn palette_runs_by_keyboard_and_escape_closes() {
    let mut h = harness();
    h.state_mut().run("ui.palette", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.get_by_role_and_label(Role::TextInput, "Search commands...").is_focused());
    h.event(Event::Text("lop".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("layers"));
    h.state_mut().run("ui.palette", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialog.is_none());
}

#[test]
fn hostile_queries_are_bounded() {
    let index = SearchIndex::build("vi", &shortcuts::defaults());
    for query in ["x".repeat(100_000), "\u{0301}".repeat(1000), String::new()] {
        for limit in [0, 1, 12, 50, usize::MAX] {
            let hits = index.search(&query, &[], &[], limit);
            assert!(hits.len() <= limit.min(50));
            assert!(hits.iter().all(|hit| index.entry(hit).is_some()));
        }
    }
    let query = format!("{}layer", " ".repeat(64));
    let truncated: Vec<_> = index.search(&query, &[], &[], 50).iter().map(|hit| hit.entry).collect();
    let empty: Vec<_> = index.search("", &[], &[], 50).iter().map(|hit| hit.entry).collect();
    assert_eq!(truncated, empty);
}

#[test]
fn palette_shortcut_is_desktop_only() {
    assert_eq!(shortcuts::defaults_for(KeyPlatform::Desktop).get("ui.palette").map(String::as_str), Some("Ctrl+Shift+P"));
    assert!(!shortcuts::defaults_for(KeyPlatform::Web).contains_key("ui.palette"));
    assert!(shortcuts::is_browser_reserved("Ctrl+Shift+P"));
}

#[test]
fn quick_access_opens_prefilled_palette() {
    let mut h = harness();
    h.state_mut().run("ui.quickaccess.pin", json!({"id": "line"})).unwrap();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Search commands...").focus();
    h.run_steps(2);
    h.event(Event::Text("lop".into()));
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(palette::DIALOG));
    assert_eq!(h.state().palette.query, "lop");
    assert!(h.state().cmd.buffer.is_empty());
    assert!(quick_access::search(h.state(), "lop").iter().any(|entry| entry.id == "LAYER"));
}

#[test]
fn palette_ime_enter_never_runs_a_command() {
    let mut h = harness();
    h.state_mut().run("ui.palette", json!({"query": "lop"})).unwrap();
    h.run_steps(3);
    h.event(Event::Ime(ImeEvent::Preedit { text: "lớp".into(), active_range_chars: None }));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(palette::DIALOG));
    assert!(h.state().session.running.is_none());
    h.input_mut().events.extend([
        Event::Ime(ImeEvent::Commit("lớp".into())),
        Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(palette::DIALOG));
    assert!(h.state().session.running.is_none());
}
