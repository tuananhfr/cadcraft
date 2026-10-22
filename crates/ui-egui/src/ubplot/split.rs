#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;

#[cfg(not(target_arch = "wasm32"))]
use cadcraft_engine::cmd::plotters::find_device;
use cadcraft_engine::sheetplan::{Sheet, SheetPlan};
#[cfg(not(target_arch = "wasm32"))]
use cadcraft_engine::sheetplan::{ids::sheet_key, split::propose_split};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use super::dialog::Action;

#[derive(Clone, Default)]
pub(super) struct State {
    #[cfg(not(target_arch = "wasm32"))]
    overlaps: BTreeMap<String, f64>,
}

#[cfg(target_arch = "wasm32")]
pub(super) fn show(ui: &mut egui::Ui, _plan: &SheetPlan, _sheet: Option<&Sheet>, _state: &mut State, _action: &mut Option<Action>) {
    ui.label(crate::tl!("Printer limits apply in the desktop app; the web app plots PDF."));
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn show(ui: &mut egui::Ui, plan: &SheetPlan, sheet: Option<&Sheet>, state: &mut State, action: &mut Option<Action>) {
    let Some(sheet) = sheet else { return };
    let Some(split) = &sheet.split else { return };
    let key = sheet_key(sheet);
    let overlap = state.overlaps.entry(key.clone()).or_insert(split.overlap_mm);
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::tl!("Overlap (mm)"));
        ui.add(egui::DragValue::new(&mut *overlap).range(0.0..=200.0));
    });
    let device = plan.device.as_deref().and_then(find_device);
    let limits = device.as_ref().map(|device| {
        device.papers.iter().take(cadcraft_doc::MAX_DEVICE_PAPERS).fold((0.0_f64, 0.0_f64), |(long, short), paper| {
            if paper.width.is_finite() && paper.height.is_finite() && paper.width > 0.0 && paper.height > 0.0 {
                (long.max(paper.width.max(paper.height)), short.max(paper.width.min(paper.height)))
            } else {
                (long, short)
            }
        })
    });
    let proposal = limits.and_then(|(long, short)| propose_split(sheet, long, short, *overlap));
    ui.label(crate::tf!("{n} pieces", n = proposal.as_ref().map_or(0, |proposal| proposal.pieces.len())));
    if ui.add_enabled(proposal.is_some(), egui::Button::new(crate::tl!("Approve split"))).clicked() {
        *action = Some(Action::Approve(json!({"split": [{"sheetKey": key, "overlapMm": *overlap}], "by": "user"})));
    }
}
