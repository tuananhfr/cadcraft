//! Canvas split limits and floating bounds through the shared desktop/web shell.

use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::dock_clamp::{MAX_FLOAT_SHARE, MAX_RATIO, MIN_RATIO, clamp_layout};
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use craft_ui::docking::{Floating, Layout, Node};
use craft_ui::layout::{SplitAxis, SplitSize};
use egui::{Event, Modifiers, PointerButton, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn content() -> Rect {
    Rect::from_min_size(egui::Pos2::ZERO, vec2(1366.0, 768.0))
}

fn tabs(panel: Panel) -> Node<Panel> {
    Node::Tabs { panels: vec![panel], active: 0 }
}

fn split(axis: SplitAxis, size: SplitSize, first: Node<Panel>, second: Node<Panel>) -> Node<Panel> {
    Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }
}

fn layout(root: Node<Panel>) -> Layout<Panel> {
    Layout { root: Some(root), floating: Vec::new() }
}

#[test]
fn ratios_on_the_canvas_path_are_clamped() {
    for (ratio, expected) in [(0.02, 0.15), (0.99, 0.85), (0.5, 0.5)] {
        for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
            for canvas_first in [false, true] {
                let (first, second) = if canvas_first { (Panel::Canvas, Panel::Properties) } else { (Panel::Properties, Panel::Canvas) };
                let mut tree = layout(split(axis, SplitSize::Ratio(ratio), tabs(first), tabs(second)));
                assert_eq!(clamp_layout(&mut tree, content()), ratio != expected);
                let Node::Split { size, .. } = tree.root.as_ref().unwrap() else { panic!("expected split") };
                assert_eq!(*size, SplitSize::Ratio(expected));
                assert!(!clamp_layout(&mut tree, content()));
            }
        }
    }
}

#[test]
fn fixed_sizes_are_clamped_against_content() {
    for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
        let (minimum, maximum) = if axis == SplitAxis::Horizontal { (204.9, 1161.1) } else { (115.2, 652.8) };
        for fixed_first in [false, true] {
            for (points, expected) in [(1300.0, maximum), (1.0, minimum), (300.0, 300.0)] {
                let size = if fixed_first { SplitSize::FixedFirst(points) } else { SplitSize::FixedSecond(points) };
                let mut tree = layout(split(axis, size, tabs(Panel::Properties), tabs(Panel::Canvas)));
                assert_eq!(clamp_layout(&mut tree, content()), points != expected);
                let Node::Split { size, .. } = tree.root.as_ref().unwrap() else { panic!("expected split") };
                let actual = match size {
                    SplitSize::FixedFirst(value) if fixed_first => *value,
                    SplitSize::FixedSecond(value) if !fixed_first => *value,
                    _ => panic!("fixed size variant changed"),
                };
                assert!((actual - expected).abs() < 0.01);
                assert!(!clamp_layout(&mut tree, content()));
            }
        }
    }
}

#[test]
fn splits_without_canvas_are_left_alone() {
    let side = split(SplitAxis::Vertical, SplitSize::Ratio(0.02), tabs(Panel::Layers), tabs(Panel::Properties));
    let mut without = layout(side.clone());
    assert!(!clamp_layout(&mut without, content()));
    assert_eq!(without.root.as_ref(), Some(&side));
    let mut tree = layout(split(SplitAxis::Horizontal, SplitSize::Ratio(0.99), side.clone(), tabs(Panel::Canvas)));
    assert!(clamp_layout(&mut tree, content()));
    let Node::Split { first, size, .. } = tree.root.as_ref().unwrap() else { panic!("expected split") };
    assert_eq!(first.as_ref(), &side);
    assert_eq!(*size, SplitSize::Ratio(MAX_RATIO));
}

#[test]
fn floating_panel_never_covers_the_canvas() {
    let mut tree = layout(tabs(Panel::Canvas));
    tree.floating.push(Floating { panels: vec![Panel::Properties], active: 0, rect: [0.0, 0.0, 5000.0, 5000.0] });
    let area = content().translate(vec2(30.0, 40.0));
    assert!(clamp_layout(&mut tree, area));
    let [x, y, width, height] = tree.floating.first().unwrap().rect;
    let floating = Rect::from_min_size(pos2(x, y), vec2(width, height));
    assert!(area.contains_rect(floating));
    assert!(width <= MAX_FLOAT_SHARE * area.width());
    assert!(height <= MAX_FLOAT_SHARE * area.height());
    assert!((width - 1161.1).abs() < 0.01 && (height - 652.8).abs() < 0.01);
    assert!(area.area() - area.intersect(floating).area() > 0.0);
    assert!(!clamp_layout(&mut tree, area));
}

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn harness() -> Harness<'static, CadApp> {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.run("ui.workspace.select", json!({"id":"AUTOCAD_FAMILIAR"})).unwrap();
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

fn drag(h: &mut Harness<'_, CadApp>, from: egui::Pos2, to: egui::Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from.lerp(to, step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
}

fn canvas_parent(node: &Node<Panel>, path: Vec<bool>) -> Vec<bool> {
    let Node::Split { first, second, .. } = node else { panic!("expected canvas ancestor") };
    for (branch, child) in [(false, first), (true, second)] {
        if let Node::Tabs { panels, .. } = child.as_ref()
            && panels.contains(&Panel::Canvas)
        {
            return path;
        }
        let probe = Layout { root: Some(child.as_ref().clone()), floating: Vec::new() };
        if probe.contains(&Panel::Canvas) {
            let mut next = path.clone();
            next.push(branch);
            return canvas_parent(child, next);
        }
    }
    panic!("canvas missing")
}

#[test]
fn fifty_dock_cycles_keep_canvas_usable_h02_tc02() {
    let _guard = test_lock();
    let mut h = harness();
    let preset = h.state().ui.docking.layout.clone();
    let drawing = h.state().session.state().unwrap().doc.clone();
    let dirty = h.state().session.state().unwrap().is_dirty();
    let camera = h.state().session.state().unwrap().view();
    let selection = h.state().session.selection();
    for cycle in 0..50 {
        h.state_mut().run("ui.dock", json!({"operation":"float","panel":"properties","rect":[450,220,300,240]})).unwrap();
        h.run_steps(3);
        let from = h.get_by_role_and_label(egui::accesskit::Role::Tab, "Properties").rect().center();
        let canvas = h.state().canvas.rect.unwrap();
        let x = if cycle % 2 == 0 { canvas.left() + 5.0 } else { canvas.right() - 5.0 };
        drag(&mut h, from, pos2(x, canvas.center().y));
        assert!(!h.state().ui.docking.layout.floating.iter().any(|group| group.panels.contains(&Panel::Properties)));
        let path = canvas_parent(h.state().ui.docking.layout.root.as_ref().unwrap(), Vec::new());
        for x in [5.0, 1360.0] {
            let id = egui::Id::new("cadcraft-docking").with(("split", &path));
            let from = h.ctx.read_response(id).expect("canvas divider").rect.center();
            drag(&mut h, from, pos2(x, from.y));
            let canvas = h.state().canvas.rect.unwrap();
            assert!(canvas.width() >= MIN_RATIO * content().width(), "cycle {cycle}: {canvas:?}");
            assert!(canvas.height() >= MIN_RATIO * content().height(), "cycle {cycle}: {canvas:?}");
            let mut probe = h.state().ui.docking.layout.clone();
            assert!(!clamp_layout(&mut probe, h.ctx.content_rect()));
            h.state().ui.docking.validate().unwrap();
        }
    }
    h.state_mut().run("ui.workspace.reset", json!({})).unwrap();
    h.run_steps(5);
    assert_eq!(h.state().ui.docking.layout, preset);
    assert!(std::sync::Arc::ptr_eq(&drawing, &h.state().session.state().unwrap().doc));
    assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
    assert_eq!(h.state().session.state().unwrap().view(), camera);
    assert_eq!(h.state().session.selection(), selection);
}

#[test]
fn dpi_change_reclamps_floating_panels() {
    let _guard = test_lock();
    let mut h = harness();
    h.state_mut().run("ui.dock", json!({"operation":"float","panel":"properties","rect":[1000,600,5000,5000]})).unwrap();
    h.run_steps(4);
    h.set_pixels_per_point(2.0);
    h.set_size(vec2(683.0, 384.0));
    h.run_steps(6);
    let area = h.ctx.content_rect();
    assert!((h.ctx.pixels_per_point() - 2.0).abs() < 0.01);
    assert!(!h.state().ui.docking.layout.floating.is_empty());
    for group in &h.state().ui.docking.layout.floating {
        let [x, y, width, height] = group.rect;
        let rect = Rect::from_min_size(pos2(x, y), vec2(width, height));
        assert!(area.expand(0.01).contains_rect(rect));
        assert!(width <= MAX_FLOAT_SHARE * area.width());
        assert!(height <= MAX_FLOAT_SHARE * area.height());
    }
}
