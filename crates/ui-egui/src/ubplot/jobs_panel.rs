use cadcraft_engine::sheetplan::queue::{JobStatus, Progress};
use cadcraft_engine::sheetplan::{Severity, SheetStatus};
use serde_json::{Value, json};

use crate::CadApp;

pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let resolved = cadcraft_engine::cmd::registry::resolve_command(id).unwrap_or(id);
    if resolved == "ubplot.jobs"
        && matches!(params.get("action").and_then(Value::as_str), Some("resume" | "remove"))
        && params.get("jobId").and_then(Value::as_str).is_some_and(|id| app.job_pump.owns(id))
    {
        return Some(Err(crate::tl!("Print job is already queued or running").into()));
    }
    #[cfg(target_arch = "wasm32")]
    if resolved == "ubplot.jobs" {
        let action = params.get("action").and_then(Value::as_str).unwrap_or("list");
        if let Some(id) = params.get("jobId").and_then(Value::as_str) {
            if action == "resume" {
                return Some(app.job_pump.resume(&mut app.session, id).map(|()| json!({"manifest": app.job_pump.manifests.get(id)})));
            }
            if action == "remove" {
                return Some(app.job_pump.discard_interrupted(&mut app.session, id).map(|()| json!({"jobId": id, "removed": true})));
            }
        }
    }
    if id != "ui.ubplot.jobs" {
        return None;
    }
    Some(command(app, params))
}

fn command(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let action = params.get("action").and_then(Value::as_str).unwrap_or("open");
    if action == "open" {
        app.job_pump.open = true;
        return Ok(Value::Null);
    }
    let id = params.get("jobId").and_then(Value::as_str).ok_or_else(|| crate::tl!("Print job id is required").to_string())?;
    match action {
        "resume" => app.job_pump.resume(&mut app.session, id)?,
        "discard" => app.job_pump.discard_interrupted(&mut app.session, id)?,
        #[cfg(target_arch = "wasm32")]
        "folder" => return Err(crate::tl!("Print output is unavailable").into()),
        #[cfg(not(target_arch = "wasm32"))]
        "folder" => {
            let name = params.get("file").and_then(Value::as_str).ok_or_else(|| crate::tl!("Print output is unavailable").to_string())?;
            let file = app
                .job_pump
                .manifests
                .get(id)
                .and_then(|manifest| manifest.files.iter().find(|file| file.name == name))
                .ok_or_else(|| crate::tl!("Print output is unavailable").to_string())?;
            let parent = std::path::Path::new(&file.delivered_to).parent().ok_or_else(|| crate::tl!("Print output is unavailable").to_string())?;
            #[cfg(target_os = "windows")]
            let opener = "explorer";
            #[cfg(target_os = "macos")]
            let opener = "open";
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let opener = "xdg-open";
            let parent = parent.to_path_buf();
            // Waiting off the UI thread reaps the opener without stalling drawing input.
            std::thread::Builder::new()
                .name("cadcraft-open-folder".into())
                .spawn(move || {
                    if let Err(error) = std::process::Command::new(opener).arg(parent).status() {
                        log::warn!("Could not open print output folder: {error}");
                    }
                })
                .map_err(|error| error.to_string())?;
        }
        _ => return Err(crate::tl!("Unknown print job action").into()),
    }
    Ok(Value::Null)
}

pub(crate) fn show(app: &mut CadApp, ctx: &egui::Context) {
    if !app.job_pump.open {
        return;
    }
    let mut open = true;
    let mut action = None;
    egui::Window::new(crate::tl!("Plot Jobs"))
        .id(egui::Id::new("ubplot_jobs_panel"))
        .open(&mut open)
        .default_width(360.0)
        .default_pos(egui::pos2((ctx.content_rect().right() - 400.0).max(16.0), 100.0))
        .max_height((ctx.content_rect().height() - 160.0).max(120.0))
        .show(ctx, |ui| {
            if !app.job_pump.available() {
                ui.label(crate::tl!("Background printing is not available in this build"));
            }
            if app.job_pump.manifests.is_empty() && app.job_pump.errors.is_empty() {
                ui.label(crate::tl!("No print jobs"));
            }
            egui::ScrollArea::vertical().id_salt("ubplot_jobs_scroll").show(ui, |ui| {
                for (id, manifest) in &app.job_pump.manifests {
                    ui.push_id(id, |ui| {
                        ui.label(crate::tf!("Print job {jobId}", jobId = id));
                        ui.label(super::worker::status_label(manifest.status));
                        let progress =
                            app.job_pump.progress.as_ref().filter(|(active, _)| active == id).map(|(_, progress)| progress.clone()).unwrap_or_else(
                                || Progress {
                                    done: u32::try_from(manifest.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Done).count())
                                        .unwrap_or(0),
                                    failed: 0,
                                    total: u32::try_from(manifest.sheets.len()).unwrap_or(0),
                                    files: u32::try_from(manifest.files.len()).unwrap_or(0),
                                    current: None,
                                },
                            );
                        let fraction = if progress.total == 0 { 0.0 } else { progress.done as f32 / progress.total as f32 };
                        ui.add(egui::ProgressBar::new(fraction).text(crate::tf!(
                            "{done}/{total} sheets",
                            done = progress.done,
                            total = progress.total
                        )));
                        if let Some(current) = &progress.current {
                            ui.label(crate::tf!("Current sheet: {sheet}", sheet = current));
                        }
                        if app.job_pump.interrupted.contains(id) {
                            ui.colored_label(crate::theme::Tokens::get().warn, crate::tf!("Print job {jobId} was interrupted", jobId = id));
                            ui.horizontal(|ui| {
                                if ui.button(crate::tl!("Resume")).clicked() {
                                    action = Some(("ui.ubplot.jobs", json!({"action": "resume", "jobId": id})));
                                }
                                if ui.button(crate::tl!("Discard")).clicked() {
                                    action = Some(("ui.ubplot.jobs", json!({"action": "discard", "jobId": id})));
                                }
                            });
                        } else if app.job_pump.owns(id) || matches!(manifest.status, JobStatus::Queued | JobStatus::Running) {
                            if ui.button(crate::tl!("Cancel")).clicked() {
                                action = Some(("ubplot.jobs", json!({"action": "cancel", "jobId": id})));
                            }
                        } else if matches!(manifest.status, JobStatus::Cancelled | JobStatus::Failed) && ui.button(crate::tl!("Resume")).clicked() {
                            action = Some(("ui.ubplot.jobs", json!({"action": "resume", "jobId": id})));
                        }
                        for file in &manifest.files {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(&file.name);
                                #[cfg(not(target_arch = "wasm32"))]
                                if ui.button(crate::tl!("Open folder")).clicked() {
                                    action = Some(("ui.ubplot.jobs", json!({"action": "folder", "jobId": id, "file": file.name})));
                                }
                            });
                        }
                        for issue in manifest
                            .issues
                            .iter()
                            .chain(manifest.sheets.iter().flat_map(|sheet| &sheet.issues))
                            .chain(manifest.sources.iter().flat_map(|source| &source.issues))
                            .filter(|issue| issue.severity >= Severity::Warn)
                        {
                            ui.colored_label(crate::theme::Tokens::get().warn, format!("{}: {}", issue.code, issue.message));
                        }
                    });
                    ui.separator();
                }
                for error in app.job_pump.errors.values() {
                    ui.colored_label(crate::theme::Tokens::get().warn, error);
                }
            });
        });
    app.job_pump.open = open;
    if let Some((command, params)) = action {
        match app.run(command, params.clone()) {
            Ok(_) => {
                if command == "ubplot.jobs"
                    && let Some(id) = params.get("jobId").and_then(Value::as_str)
                {
                    #[cfg(not(target_arch = "wasm32"))]
                    app.job_pump.refresh(id);
                    #[cfg(target_arch = "wasm32")]
                    app.job_pump.refresh(&mut app.session, id);
                }
            }
            Err(error) => app.set_status(error),
        }
        ctx.request_repaint();
    }
}
