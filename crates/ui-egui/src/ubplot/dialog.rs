use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use cadcraft_doc::Handle;
use cadcraft_engine::sheetplan::{ScanMode, ScanRequest, Severity, Sheet, SheetPlan, SheetSource, SheetStatus, ids::sheet_key, verdict};
use egui::{TextureHandle, vec2};
use serde_json::{Value, json};

use crate::CadApp;

#[derive(Clone, Default)]
struct State {
    doc_uid: u64,
    job_id: String,
    exceptions: super::exceptions::State,
    split: bool,
    textures: BTreeMap<(String, u32), Result<TextureHandle, String>>,
    failed: Vec<String>,
    error: Option<String>,
}

pub(super) enum Action {
    Rescan,
    Profile,
    Edit(Box<ScanRequest>),
    Pick(String),
    Publish,
    Approve(Value),
}

pub(crate) fn show(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let Some(last) = app.session.ubplot.last.as_ref().filter(|last| app.session.state().is_ok_and(|state| state.uid == last.doc_uid)) else {
        *open = false;
        return;
    };
    let (uid, plan, request) = (last.doc_uid, last.plan.clone(), last.request.clone());
    let id = egui::Id::new("ubplot_dialog_state");
    let mut state = ctx.data_mut(|data| data.remove_temp::<State>(id)).filter(|state| state.doc_uid == uid).unwrap_or_default();
    state.doc_uid = uid;
    if state.job_id != plan.job_id {
        state.job_id = plan.job_id.clone();
        state.textures.clear();
        state.exceptions = super::exceptions::State::default();
        state.failed.clear();
        state.error = None;
    }
    let mut action = None;
    let available = ctx.content_rect().size() - vec2(32.0, 48.0);
    let compact = ctx.content_rect().height() < 480.0;
    egui::Window::new(crate::tl!("Quick Plot"))
        .id(egui::Id::new(super::DIALOG))
        .open(open)
        .collapsible(false)
        .default_width(820.0_f32.min(available.x))
        .default_height(640.0_f32.min(available.y))
        .max_width(available.x.max(260.0))
        .max_height(available.y.max(180.0))
        .show(ctx, |ui| {
            // Reserve the footer before scrolling so publishing stays reachable at high zoom.
            let body_height = (ui.available_height() - 76.0).max(40.0);
            let body = egui::ScrollArea::vertical().id_salt("ubplot_body").max_height(body_height).auto_shrink([false, false]);
            let body = if compact { body.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible) } else { body };
            body.show(ui, |ui| {
                ui.heading(crate::tl!("Scan"));
                let (mut pass, mut info, mut warn, mut block) = (0, 0, 0, 0);
                for sheet in &plan.sheets {
                    match verdict(sheet) {
                        Severity::Pass => pass += 1,
                        Severity::Info => info += 1,
                        Severity::Warn => warn += 1,
                        Severity::Block => block += 1,
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::tf!("{n} sheets", n = plan.sheets.len()));
                    ui.label(crate::tf!("PASS {p} · INFO {i} · WARN {w} · BLOCK {b}", p = pass, i = info, w = warn, b = block));
                    if ui.button(crate::tl!("Rescan")).clicked() {
                        action = Some(Action::Rescan);
                    }
                });
                source(ui, &request, &mut action);
                ui.separator();
                ui.heading(crate::tl!("Preview"));
                thumbnails(app, ui, &plan, &mut state, compact);
                ui.separator();
                ui.heading(crate::tl!("Exceptions"));
                super::exceptions::show(ui, &plan, &request, &mut state.exceptions, &mut action);
                if let Some(error) = &state.error {
                    ui.colored_label(crate::theme::Tokens::get().warn, error);
                }
                if !state.failed.is_empty() {
                    ui.label(crate::tl!("Failed sheets"));
                    for key in &state.failed {
                        ui.label(key);
                    }
                }
            });
            ui.separator();
            ui.heading(crate::tl!("Publish"));
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::tl!("Profile…")).clicked() {
                    action = Some(Action::Profile);
                }
                ui.radio_value(&mut state.split, false, crate::tl!("Merged PDF"));
                ui.radio_value(&mut state.split, true, crate::tl!("One PDF per sheet"));
                let reason = publish_reason(&plan, state.split);
                let button = ui.add_enabled(reason.is_none(), egui::Button::new(crate::tl!("Publish")));
                if button.clicked() {
                    action = Some(Action::Publish);
                }
                if let Some(reason) = reason {
                    button.on_disabled_hover_ui(|ui| {
                        ui.label(reason);
                        if !state.split
                            && let Some(issue) = plan
                                .issues
                                .iter()
                                .chain(plan.sheets.iter().flat_map(|sheet| &sheet.issues))
                                .find(|issue| issue.severity == Severity::Block)
                        {
                            ui.label(super::exceptions::issue_label(&issue.code));
                        }
                    });
                }
            });
        });
    super::exceptions::confirmation(ctx, &plan, &mut state.exceptions, &mut action);
    match action {
        Some(Action::Rescan) => match super::rescan(app) {
            Ok(_) => {
                state.textures.clear();
                state.error = None;
                state.failed.clear();
            }
            Err(error) => state.error = Some(error),
        },
        Some(Action::Edit(request)) => match super::scan(app, &request) {
            Ok(_) => state.error = None,
            Err(error) => state.error = Some(error),
        },
        Some(Action::Profile) => {
            let _ = app.run("ui.ubplot.profile", Value::Null);
        }
        Some(Action::Pick(key)) => super::pick_window(app, ctx, key),
        Some(Action::Approve(params)) => match app.run("ubplot.approve", params) {
            Ok(_) => state.error = None,
            Err(error) => state.error = Some(error),
        },
        Some(Action::Publish) => publish(app, &mut state, open),
        None => {}
    }
    if *open {
        ctx.data_mut(|data| data.insert_temp(id, state));
    }
}

fn source(ui: &mut egui::Ui, request: &ScanRequest, action: &mut Option<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::tl!("Source"));
        for (mode, label) in [
            (ScanMode::Block, crate::tl!("Block")),
            (ScanMode::Window, crate::tl!("Window")),
            (ScanMode::Layout, crate::tl!("Layout")),
            (ScanMode::Kcs, crate::tl!("KCS plot areas (layer KCS_Plotter)")),
        ] {
            let selected = request.mode == mode;
            // Blocks, windows and layouts are picked in the drawing; KCS markers are found without picking.
            let radio = ui.add_enabled(selected || mode == ScanMode::Kcs, egui::RadioButton::new(selected, label));
            if radio.clicked() && !selected {
                let kcs = ScanRequest {
                    mode: ScanMode::Kcs,
                    paper: request.paper.clone(),
                    scale: request.scale.clone(),
                    device: request.device.clone(),
                    ..Default::default()
                };
                *action = Some(Action::Edit(Box::new(kcs)));
            }
        }
    });
}

fn thumbnails(app: &mut CadApp, ui: &mut egui::Ui, plan: &SheetPlan, state: &mut State, compact: bool) {
    let columns = ((ui.available_width() / 176.0).floor() as usize).clamp(1, 16);
    let rows = plan.sheets.len().div_ceil(columns);
    let mut budget = 2_usize;
    // At high zoom, wheel input over previews must reach the exceptions in the parent body.
    let source = egui::scroll_area::ScrollSource { mouse_wheel: !compact, ..Default::default() };
    egui::ScrollArea::vertical()
        .id_salt("ubplot_thumbnails")
        .max_height((ui.ctx().content_rect().height() * 0.2).clamp(48.0, 140.0))
        .scroll_source(source)
        .show_rows(ui, 188.0, rows, |ui, range| {
            let visible_orders: BTreeSet<_> =
                plan.sheets.iter().skip(range.start * columns).take(range.len() * columns).map(|sheet| sheet.order).collect();
            // Retaining viewport rows bounds GPU memory even when every sheet has been visited.
            state.textures.retain(|(_, order), _| visible_orders.contains(order));
            for row in range {
                ui.horizontal(|ui| {
                    for sheet in plan.sheets.iter().skip(row * columns).take(columns) {
                        ui.vertical(|ui| {
                            ui.set_min_height(188.0);
                            let (rect, _) = ui.allocate_exact_size(vec2(160.0, 160.0), egui::Sense::hover());
                            if ui.is_rect_visible(rect) {
                                let key = (plan.job_id.clone(), sheet.order);
                                if budget > 0
                                    && let Entry::Vacant(entry) = state.textures.entry(key.clone())
                                {
                                    budget -= 1;
                                    entry.insert(thumbnail(app, ui.ctx(), sheet));
                                }
                                match state.textures.get(&key) {
                                    Some(Ok(texture)) => {
                                        let image_rect = egui::Rect::from_center_size(rect.center(), texture.size_vec2());
                                        egui::Image::new((texture.id(), texture.size_vec2())).paint_at(ui, image_rect);
                                    }
                                    Some(Err(_)) => {
                                        ui.put(rect, egui::Label::new(crate::tl!("Preview unavailable")));
                                    }
                                    None => {
                                        ui.put(rect, egui::Label::new(crate::tl!("Loading preview…")));
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }
                            ui.label(crate::tf!("Preview {order}", order = sheet.order));
                        });
                    }
                });
            }
        });
}

fn thumbnail(app: &mut CadApp, ctx: &egui::Context, sheet: &Sheet) -> Result<TextureHandle, String> {
    let params = match &sheet.source {
        SheetSource::Layout { name } => json!({"layout": name, "width": 160}),
        SheetSource::Window { outline, .. } => json!({"layout": "Model", "outline": outline, "width": 160}),
        SheetSource::Block { outline, block_handle, .. } => {
            let handle = Handle::parse_hex(block_handle).ok_or_else(|| crate::tl!("Preview unavailable").to_string())?;
            let space = app
                .session
                .doc()
                .map_err(|error| error.to_string())?
                .space_of(handle)
                .ok_or_else(|| crate::tl!("Preview unavailable").to_string())?;
            json!({"layout": space.tab_name(), "outline": outline, "width": 160})
        }
    };
    let result = app.run("preview", params)?;
    let png = result
        .get("data")
        .and_then(Value::as_str)
        .and_then(cadcraft_engine::cmd::file::base64_decode)
        .ok_or_else(|| crate::tl!("Preview unavailable").to_string())?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|error| error.to_string())?.thumbnail(160, 160).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ctx.load_texture("ubplot_thumbnail", egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), egui::TextureOptions::LINEAR))
}

fn publish_reason(plan: &SheetPlan, split: bool) -> Option<&'static str> {
    if plan.sheets.is_empty() {
        Some(crate::tl!("No sheets to publish"))
    } else if !split
        && (plan.issues.iter().any(|issue| issue.severity == Severity::Block)
            || plan.sheets.iter().any(|sheet| verdict(sheet) == Severity::Block || sheet.status == SheetStatus::Blocked))
    {
        Some(crate::tl!("Fix or exclude blocked sheets before publishing a merged PDF"))
    } else if super::exceptions::has_unapproved_warnings(plan) {
        Some(crate::tl!("Approve all warnings before publishing"))
    } else {
        None
    }
}

fn publish(app: &mut CadApp, state: &mut State, open: &mut bool) {
    let title = app.session.state().map(|state| state.title.clone()).unwrap_or_default();
    let stem = std::path::Path::new(&title).file_stem().and_then(|stem| stem.to_str()).filter(|stem| !stem.is_empty()).unwrap_or("Drawing");
    let suggested = format!("{stem}.pdf");
    let path = match &app.services.pick_save {
        Some(pick) => {
            let Some(path) = pick(&suggested) else { return };
            path
        }
        None => suggested,
    };
    let output = if state.split { "split" } else { "merged" };
    state.failed.clear();
    state.error = None;
    let background = cfg!(not(target_arch = "wasm32")) || app.job_pump.available();
    let params = json!({"output": output, "path": path, "background": background});
    match app.run("ubplot.publish", params) {
        Ok(result) => {
            if let Some(id) = result.get("jobId").and_then(Value::as_str) {
                let message = if app.job_pump.interrupted.contains(id) {
                    crate::tf!("Print job {jobId} already exists; use Resume or Discard in Plot Jobs", jobId = id)
                } else if result.get("status").and_then(Value::as_str) == Some("QUEUED") {
                    crate::tf!("Print job {jobId} queued", jobId = id)
                } else {
                    let status = result.get("status").cloned().and_then(|value| serde_json::from_value(value).ok());
                    match status {
                        Some(status) => crate::tf!("Print job {jobId}: {status}", jobId = id, status = super::worker::status_label(status, false)),
                        None => crate::tf!("Print job {jobId}", jobId = id),
                    }
                };
                app.session.echo(message);
                let _ = app.run("ui.ubplot.jobs", Value::Null);
                *open = false;
                return;
            }
            let count = result.get("files").and_then(Value::as_array).map_or(0, Vec::len);
            app.session.echo(crate::tf!("Published {k} file(s)", k = count));
            if let Some(failed) = result.get("failed").and_then(Value::as_array) {
                state.failed = failed.iter().filter_map(|sheet| sheet.get("sheetKey").and_then(Value::as_str).map(str::to_string)).collect();
            }
            *open = !state.failed.is_empty();
        }
        Err(error) => {
            state.error = Some(error);
            if let Some(last) = &app.session.ubplot.last {
                state.failed = last
                    .plan
                    .sheets
                    .iter()
                    .filter(|sheet| matches!(sheet.status, SheetStatus::Failed | SheetStatus::Blocked))
                    .map(sheet_key)
                    .collect();
            }
        }
    }
}
