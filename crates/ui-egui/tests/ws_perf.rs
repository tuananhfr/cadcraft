//! Manual release SLO checks; elapsed times include workspace application and an egui frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cadcraft_engine::{Session, command_specs};
use cadcraft_ui_egui::wsprofile::presets::{PRESET_IDS, preset};
use cadcraft_ui_egui::wsprofile::schema::{RibbonGroup, RibbonTab, ToolbarState, WorkspaceLayout, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::store::{MemoryStore, ProfileSet};
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::json;

static LOCK: Mutex<()> = Mutex::new(());

fn controls(layout: &WorkspaceLayout) -> usize {
    let ribbon: usize = layout.ribbon.tabs.iter().flat_map(|tab| &tab.groups).map(|group| group.commands.len()).sum();
    ribbon + layout.toolbars.iter().map(|toolbar| toolbar.commands.len()).sum::<usize>()
}

fn large_profile() -> WorkspaceProfile {
    let mut commands: Vec<String> = command_specs().iter().map(|spec| spec.id.to_string()).collect();
    commands.sort_unstable();
    commands.dedup();
    let mut commands = commands.into_iter();
    let mut profile = preset("AUTOCAD_FAMILIAR").unwrap();
    profile.base_id = Some("AUTOCAD_FAMILIAR".into());
    profile.layout.quick_access.clear();
    profile.layout.ribbon.tabs = vec![RibbonTab {
        id: "perf".into(),
        label: "Performance fixture".into(),
        groups: (0..10)
            .map(|index| RibbonGroup { id: format!("perf-{index}"), label: format!("Group {index}"), commands: commands.by_ref().take(10).collect() })
            .collect(),
        ..Default::default()
    }];
    profile.layout.toolbars = (0..4)
        .map(|index| ToolbarState {
            id: format!("perf-{index}"),
            label: format!("Toolbar {index}"),
            order: index,
            commands: commands.by_ref().take(25).collect(),
            ..Default::default()
        })
        .collect();
    assert_eq!(controls(&profile.layout), 200);
    profile
}

fn app() -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app
}

fn frame(app: &mut CadApp, ctx: &egui::Context, tick: usize) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3840.0, 2160.0))),
        time: Some(tick as f64 / 60.0),
        ..Default::default()
    };
    let _output = ctx.run_ui(raw, |ui| {
        app.logic(&ui.ctx().clone());
        app.ui(ui);
    });
}

fn assert_p95(name: &str, mut samples: Vec<Duration>, limit: Duration) {
    assert!(!samples.is_empty());
    samples.sort_unstable();
    let rank = (samples.len() * 95).div_ceil(100);
    let p95 = *samples.get(rank - 1).unwrap();
    println!("{name}: samples={}, P95={:.3} ms, SLO={:.3} ms", samples.len(), p95.as_secs_f64() * 1000.0, limit.as_secs_f64() * 1000.0);
    assert!(p95 <= limit, "{name}: P95 {p95:?} exceeds {limit:?}");
}

#[test]
#[ignore = "manual release performance measurement"]
fn restore_p95() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let profile = large_profile();
    let set = ProfileSet { active: "AUTOCAD_FAMILIAR".into(), user: Some(profile), ..Default::default() };
    let text = serde_json::to_string(&set).unwrap();
    let mut samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let store = MemoryStore { current: Some(text.clone()), ..Default::default() };
        let started = Instant::now();
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach_profile_store(Box::new(store));
        frame(&mut app, &ctx, 0);
        samples.push(started.elapsed());
        assert!(app.ws.fallback.is_none(), "{:?}", app.ws.fallback);
        assert_eq!(controls(&app.ws.effective.layout), 200);
    }
    assert_p95("restore_p95", samples, Duration::from_millis(1500));
}

#[test]
#[ignore = "manual release performance measurement"]
fn switch_p95() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app();
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("ui.sample", json!({})).unwrap();
    app.canvas.zoom_pending = false;
    let ctx = egui::Context::default();
    let mut tick = 0;
    // Warm every preset so lazy font/texture creation does not masquerade as a switch leak.
    for id in PRESET_IDS.iter().chain(std::iter::once(&"SIMPLE")) {
        app.run("ui.workspace.select", json!({"id": id})).unwrap();
        for _ in 0..3 {
            frame(&mut app, &ctx, tick);
            tick += 1;
        }
    }
    let drawing = app.session.state().unwrap().doc.clone();
    let view = app.session.state().unwrap().view();
    assert!(!drawing.model.is_empty());
    assert!(!app.session.state().unwrap().is_dirty());
    let textures_before = ctx.tex_manager().read().num_allocated();
    let mut samples = Vec::with_capacity(100);
    for id in PRESET_IDS.iter().cycle().skip(1).take(100) {
        let started = Instant::now();
        app.run("ui.workspace.select", json!({"id": id})).unwrap();
        frame(&mut app, &ctx, tick);
        samples.push(started.elapsed());
        tick += 1;
        let state = app.session.state().unwrap();
        assert!(Arc::ptr_eq(&drawing, &state.doc), "{id}: drawing was reloaded");
        assert_eq!(state.view(), view, "{id}");
        assert!(!state.is_dirty(), "{id}");
        assert!(app.ws.history.undo_len() + app.ws.history.redo_len() <= 100);
        assert!(app.ws.store.as_deref().unwrap().snapshots().len() <= 20);
    }
    app.run("ui.workspace.select", json!({"id": "SIMPLE"})).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx, tick);
        tick += 1;
    }
    let textures_after = ctx.tex_manager().read().num_allocated();
    println!(
        "switch proxies: history={}, snapshots={}, egui textures={textures_before}->{textures_after}; profiler evidence unavailable",
        app.ws.history.undo_len() + app.ws.history.redo_len(),
        app.ws.store.as_deref().unwrap().snapshots().len(),
    );
    assert_eq!(textures_after, textures_before, "egui texture count grew after returning to SIMPLE");
    assert_p95("switch_p95", samples, Duration::from_millis(500));
}
