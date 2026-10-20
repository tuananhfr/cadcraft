//! Workspace layers, merge patches and policy locks; autosave timing; layout history; screen ratios.

use cadcraft_ui_egui::wsprofile::autosave::Autosave;
use cadcraft_ui_egui::wsprofile::history::{HISTORY_CAP, LayoutHistory};
use cadcraft_ui_egui::wsprofile::merge::{Layer, Level, PolicyBlock, apply_patch, changed_pointers, check, diff_patch, effective};
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::{Dock, PanelMode, PanelState, ToolbarState, WorkspaceLayout};
use cadcraft_ui_egui::wsprofile::screen::{from_ratio, to_ratio};
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MemoryStore, ProfileStore, expired_snapshots};
use serde_json::{Value, json};

fn layer(level: Level, patch: Value) -> Layer {
    Layer { level, patch, locks: Vec::new() }
}

fn draw_visible(layers: &[Layer]) -> Option<bool> {
    effective(layers).layout.toolbars.iter().find(|toolbar| toolbar.id == "draw").map(|toolbar| toolbar.visible)
}

#[test]
fn merge_precedence() {
    let platform = layer(Level::Platform, json!({"toolbars": [{"id": "draw", "visible": true}]}));
    let mut company = layer(Level::Company, json!({"toolbars": [{"id": "draw", "visible": false}]}));
    let user = layer(Level::User, json!({"toolbars": [{"id": "draw", "visible": true}]}));
    // Layers are applied by level, not by their order in the slice.
    assert_eq!(draw_visible(&[user.clone(), company.clone(), platform.clone()]), Some(true));

    company.locks = vec!["/toolbars/draw".into()];
    let layers = [user, company, platform];
    assert_eq!(draw_visible(&layers), Some(false));
    let eff = effective(&layers);
    assert_eq!(check(&eff, "/toolbars/draw/visible"), Err(PolicyBlock { pointer: "/toolbars/draw".into(), level: Level::Company }));
    assert_eq!(check(&eff, "/toolbars/draw"), Err(PolicyBlock { pointer: "/toolbars/draw".into(), level: Level::Company }));
    // Replacing the parent would replace the locked child too.
    assert!(check(&eff, "/toolbars").is_err());
    assert_eq!(check(&eff, "/toolbars/standard/visible"), Ok(()));
    assert_eq!(check(&eff, "/toolbars/drawing"), Ok(()), "a lock covers its own subtree only");
}

#[test]
fn keyed_arrays_merge_by_id_not_index() {
    let platform = layer(Level::Platform, json!({"toolbars": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]}));
    let user = layer(Level::User, json!({"toolbars": [{"id": "b", "visible": false}, {"id": "c", "label": "C"}]}));
    let toolbars = effective(&[platform, user]).layout.toolbars;
    let ids: Vec<_> = toolbars.iter().map(|t| (t.id.as_str(), t.label.as_str(), t.visible)).collect();
    assert_eq!(ids, vec![("a", "A", true), ("b", "B", false), ("c", "C", true)]);
}

#[test]
fn a_lock_survives_deletes_and_replacements_from_later_levels() {
    let mut company = layer(Level::Company, json!({"toolbars": [{"id": "draw", "visible": true}], "shortcuts": {"line": "L"}}));
    company.locks = vec!["/toolbars/draw".into(), "/shortcuts/line".into()];
    let user = layer(Level::User, json!({"toolbars": [{"id": "draw", "$delete": true}, {"id": "mine"}], "shortcuts": null, "favorites": ["line"]}));
    let eff = effective(&[company, user]);
    assert!(eff.layout.toolbars.iter().any(|t| t.id == "draw" && t.visible));
    assert!(eff.layout.toolbars.iter().any(|t| t.id == "mine"), "unlocked parts of the patch still apply");
    assert_eq!(eff.layout.shortcuts.get("line").map(String::as_str), Some("L"));
    assert_eq!(eff.layout.favorites, vec!["line".to_string()]);
    assert_eq!(eff.locked.get("/shortcuts/line"), Some(&Level::Company));
}

#[test]
fn a_layer_that_breaks_the_layout_is_skipped() {
    let platform = layer(Level::Platform, json!({"favorites": ["line"]}));
    let user = layer(Level::User, json!({"favorites": "not a list"}));
    assert_eq!(effective(&[platform, user]).layout.favorites, vec!["line".to_string()]);
}

fn edited(base: &WorkspaceLayout) -> WorkspaceLayout {
    let mut now = base.clone();
    now.toolbars[0].visible = false;
    now.panels.retain(|panel| panel.id != "layers");
    now.panels.push(PanelState { id: "fonts".into(), mode: PanelMode::Float, side: Some(Dock::Floating), ..PanelState::default() });
    now.panels.push(PanelState { id: "notYetBuilt".into(), ..PanelState::default() });
    now.toolbars.push(ToolbarState { id: "draw".into(), commands: vec!["line".into()], ..ToolbarState::default() });
    now.shortcuts.insert("line".into(), "L".into());
    now.dock_tree = Some(json!({"layout": {"floating": []}}));
    now.extra.insert("future".into(), json!({"kept": true}));
    now
}

#[test]
fn diff_patch_round_trips_edits() {
    let base = preset("SIMPLE").unwrap().layout;
    let now = edited(&base);
    let patch = diff_patch(&base, &now);
    assert_eq!(apply_patch(&base, &patch), now);
    assert_eq!(diff_patch(&base, &base), json!({}), "no edits, empty user layer");
    // Undoing the edits is a patch too.
    assert_eq!(apply_patch(&now, &diff_patch(&now, &base)), base);
    let pointers = changed_pointers(&patch);
    for expected in ["/toolbars/standard/visible", "/panels/layers", "/panels/fonts", "/shortcuts/line", "/dock_tree", "/future"] {
        let under = format!("{expected}/");
        assert!(pointers.iter().any(|p| p == expected || p.starts_with(&under)), "{expected} missing from {pointers:?}");
    }
}

#[test]
fn diff_patch_keeps_a_new_order() {
    let base = preset("SIMPLE").unwrap().layout;
    let mut now = base.clone();
    now.panels.reverse();
    let patch = diff_patch(&base, &now);
    assert_eq!(apply_patch(&base, &patch), now);
}

#[test]
fn every_preset_round_trips_through_a_patch_from_simple() {
    let simple = preset("SIMPLE").unwrap().layout;
    for id in PRESET_IDS {
        let target = preset(id).unwrap().layout;
        assert_eq!(apply_patch(&simple, &diff_patch(&simple, &target)), target, "{id}");
    }
}

#[test]
fn debounce_and_drag_end() {
    let mut autosave = Autosave::default();
    assert_eq!(autosave.debounce_secs, 0.75);
    assert!(!autosave.due(0.0), "nothing changed, nothing to save");
    autosave.changed(0.0);
    assert!(!autosave.due(0.5));
    assert!(autosave.due(0.8));
    assert!(!autosave.due(0.9), "a due save is handed out once");
    autosave.changed(1.0);
    autosave.interaction_ended();
    assert!(autosave.due(1.0));
    // A later change restarts the wait.
    autosave.changed(2.0);
    autosave.changed(2.5);
    assert!(!autosave.due(3.0));
    assert!(autosave.due(3.3));
}

#[test]
fn layout_history_is_capped_and_a_new_change_drops_redo() {
    let mut history = LayoutHistory::default();
    let mut layout = WorkspaceLayout::default();
    assert!(history.undo(layout.clone()).is_none());
    for lines in 0..(HISTORY_CAP as u32 + 10) {
        let before = layout.clone();
        layout.command_line.lines = lines + 1;
        history.record(before);
    }
    assert_eq!(history.undo_len(), HISTORY_CAP);
    let previous = history.undo(layout.clone()).unwrap();
    assert_eq!(previous.command_line.lines, HISTORY_CAP as u32 + 9);
    assert_eq!(history.redo(previous.clone()).unwrap(), layout);
    history.undo(layout.clone());
    history.record(previous);
    assert!(history.redo(WorkspaceLayout::default()).is_none());
}

#[test]
fn ratios_clamp_back_into_a_smaller_screen() {
    let big = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(3840.0, 2160.0));
    let small = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1366.0, 768.0));
    let near_corner = egui::Rect::from_min_size(egui::pos2(3700.0, 2100.0), egui::vec2(400.0, 300.0));
    let ratio = to_ratio(near_corner, big);
    assert!(ratio.iter().all(|r| (0.0..=1.0).contains(r)));
    let restored = from_ratio(ratio, small, 48.0);
    let visible = small.intersect(restored);
    assert!(visible.width() >= 48.0 && visible.height() >= 48.0, "{restored:?}");
    let same = from_ratio(to_ratio(egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(300.0, 200.0)), big), big, 48.0);
    assert!((same.min.x - 100.0).abs() < 0.01 && (same.width() - 300.0).abs() < 0.01, "{same:?}");
    for hostile in [[f32::NAN, f32::INFINITY, -1.0, 9.0], [0.0; 4]] {
        let rect = from_ratio(hostile, small, 48.0);
        assert!(rect.is_finite() && small.intersect(rect).width() > 0.0, "{rect:?}");
    }
    assert_eq!(to_ratio(near_corner, egui::Rect::NOTHING), [0.0; 4]);
}

#[test]
fn memory_store_keeps_the_newest_snapshots_and_the_damaged_copy() {
    let mut store = MemoryStore::default();
    store.push_snapshot(CORRUPT_SNAPSHOT, "{").unwrap();
    for revision in 1..=25 {
        store.push_snapshot(revision, &format!("{revision}")).unwrap();
    }
    let kept = store.snapshots();
    assert_eq!(kept.len(), KEEP_SNAPSHOTS + 1);
    assert_eq!(kept.first(), Some(&CORRUPT_SNAPSHOT));
    assert_eq!(kept.get(1), Some(&6));
    assert_eq!(store.read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
    assert_eq!(expired_snapshots(&[0, 1, 2, 3]), Vec::<u64>::new());
    assert_eq!(expired_snapshots(&(0..=22).collect::<Vec<_>>()), vec![1, 2]);
    let mut failing = MemoryStore { fail_writes: true, ..MemoryStore::default() };
    assert!(failing.write_current("{}").is_err());
    assert!(failing.push_snapshot(1, "{}").is_err());
    assert!(failing.read_current().is_none());
}
