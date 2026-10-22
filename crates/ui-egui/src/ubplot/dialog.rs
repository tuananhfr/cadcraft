use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use cadcraft_doc::Handle;
use cadcraft_engine::sheetplan::{ScanRequest, Severity, Sheet, SheetPlan, SheetSource, SheetStatus, ids::sheet_key};
use egui::{TextureHandle, vec2};
use serde_json::{Value, json};

use crate::CadApp;

#[derive(Clone, Default)]
struct State {
    doc_uid: u64,
    job_id: String,
    show_all: bool,
    split: bool,
    textures: BTreeMap<(String, u32), Result<TextureHandle, String>>,
    failed: Vec<String>,
    error: Option<String>,
}

enum Action {
    Rescan,
    Edit(Box<ScanRequest>),
    Pick(String),
    Publish,
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
        state.failed.clear();
        state.error = None;
    }
    let mut action = None;
    egui::Window::new(crate::tl!("Quick Plot"))
        .id(egui::Id::new(super::DIALOG))
        .open(open)
        .collapsible(false)
        .default_width(820.0)
        .default_height(640.0)
        .max_height((ctx.content_rect().height() - 48.0).max(240.0))
        .vscroll(true)
        .show(ctx, |ui| {
            ui.heading(crate::tl!("Scan"));
            let blocked = plan.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Blocked).count();
            let warnings =
                plan.issues.iter().chain(plan.sheets.iter().flat_map(|sheet| &sheet.issues)).filter(|issue| issue.severity == Severity::Warn).count();
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::tf!("{n} sheets", n = plan.sheets.len()));
                ui.label(crate::tf!("· {b} blocked · {w} warnings", b = blocked, w = warnings));
                if ui.button(crate::tl!("Rescan")).clicked() {
                    action = Some(Action::Rescan);
                }
            });
            ui.separator();
            ui.heading(crate::tl!("Preview"));
            thumbnails(app, ui, &plan, &mut state);
            ui.separator();
            ui.heading(crate::tl!("Exceptions"));
            ui.checkbox(&mut state.show_all, crate::tl!("Show all sheets"));
            exceptions(ui, &plan, &request, state.show_all, &mut action);
            ui.separator();
            ui.heading(crate::tl!("Publish"));
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut state.split, false, crate::tl!("Merged PDF"));
                ui.radio_value(&mut state.split, true, crate::tl!("One PDF per sheet"));
                if ui.add_enabled(!plan.sheets.is_empty(), egui::Button::new(crate::tl!("Publish"))).clicked() {
                    action = Some(Action::Publish);
                }
            });
            if let Some(error) = &state.error {
                ui.colored_label(crate::theme::Tokens::get().warn, error);
            }
            if !state.failed.is_empty() {
                ui.label(crate::tl!("Failed sheets"));
                egui::ScrollArea::vertical().id_salt("ubplot_failed").max_height(80.0).show(ui, |ui| {
                    for key in &state.failed {
                        ui.label(key);
                    }
                });
            }
        });
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
        Some(Action::Pick(key)) => super::pick_window(app, ctx, key),
        Some(Action::Publish) => publish(app, &mut state, open),
        None => {}
    }
    if *open {
        ctx.data_mut(|data| data.insert_temp(id, state));
    }
}

fn thumbnails(app: &mut CadApp, ui: &mut egui::Ui, plan: &SheetPlan, state: &mut State) {
    let columns = ((ui.available_width() / 176.0).floor() as usize).clamp(1, 16);
    let rows = plan.sheets.len().div_ceil(columns);
    let mut budget = 2_usize;
    egui::ScrollArea::vertical().id_salt("ubplot_thumbnails").max_height(190.0).show_rows(ui, 188.0, rows, |ui, range| {
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

fn exceptions(ui: &mut egui::Ui, plan: &SheetPlan, request: &ScanRequest, show_all: bool, action: &mut Option<Action>) {
    let mut groups: BTreeMap<&str, Vec<&Sheet>> = BTreeMap::new();
    for sheet in &plan.sheets {
        let codes: BTreeSet<_> =
            sheet.issues.iter().chain(&plan.issues).filter(|issue| issue.severity >= Severity::Warn).map(|issue| issue.code.as_str()).collect();
        if show_all && codes.is_empty() {
            groups.entry("").or_default().push(sheet);
        }
        for code in codes {
            groups.entry(code).or_default().push(sheet);
        }
    }
    egui::ScrollArea::vertical().id_salt("ubplot_exceptions").max_height(200.0).show(ui, |ui| {
        for (code, sheets) in groups {
            ui.push_id(code, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if code.is_empty() {
                        ui.strong(crate::tl!("All sheets"));
                    } else {
                        ui.strong(crate::tf!("{code} · {n} sheets", code = code, n = sheets.len())).on_hover_text(issue_label(code));
                    }
                    if ui.button(crate::tl!("Exclude all")).clicked() {
                        let mut next = request.clone();
                        for sheet in &sheets {
                            let key = sheet_key(sheet);
                            if !next.exclude.contains(&key) {
                                next.exclude.push(key);
                            }
                        }
                        *action = Some(Action::Edit(Box::new(next)));
                    }
                    ui.menu_button(crate::tl!("Set paper for all"), |ui| {
                        for (paper, label) in [
                            ("A0", crate::tl!("A0")),
                            ("A1", crate::tl!("A1")),
                            ("A2", crate::tl!("A2")),
                            ("A3", crate::tl!("A3")),
                            ("A4", crate::tl!("A4")),
                            ("A5", crate::tl!("A5")),
                        ] {
                            if ui.selectable_label(false, label).clicked() {
                                let mut next = request.clone();
                                for sheet in &sheets {
                                    next.overrides.entry(sheet_key(sheet)).or_default().paper = Some(paper.into());
                                }
                                *action = Some(Action::Edit(Box::new(next)));
                                ui.close();
                            }
                        }
                    });
                });
                for sheet in sheets {
                    ui.push_id(sheet.order, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::tf!("{sheet} · {order}", sheet = sheet.sheet_id, order = sheet.order));
                            if ui.button(crate::tl!("Pick window <")).clicked() {
                                *action = Some(Action::Pick(sheet_key(sheet)));
                            }
                        });
                    });
                }
            });
        }
    });
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
    match app.run("ubplot.publish", json!({"output": output, "path": path})) {
        Ok(result) => {
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

fn issue_label(code: &str) -> &str {
    match code {
        "FRAME_AMBIGUOUS" => crate::tl!("FRAME_AMBIGUOUS"),
        "FRAME_NOT_FOUND" => crate::tl!("FRAME_NOT_FOUND"),
        "FRAME_MINSERT" => crate::tl!("FRAME_MINSERT"),
        "PAPER_TOO_LARGE" => crate::tl!("PAPER_TOO_LARGE"),
        "RENDER_FAILED" => crate::tl!("RENDER_FAILED"),
        "FRAME_DUPLICATE" => crate::tl!("FRAME_DUPLICATE"),
        "FRAME_OVERLAP" => crate::tl!("FRAME_OVERLAP"),
        "FRAME_NESTED" => crate::tl!("FRAME_NESTED"),
        "FRAME_MIRRORED" => crate::tl!("FRAME_MIRRORED"),
        "FRAME_NONUNIFORM" => crate::tl!("FRAME_NONUNIFORM"),
        other => other,
    }
}
