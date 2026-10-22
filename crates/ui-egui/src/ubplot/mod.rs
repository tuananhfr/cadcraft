mod dialog;
mod exceptions;
pub mod jobs_panel;
pub(crate) mod overlay;
mod split;
pub mod worker;

use cadcraft_engine::sheetplan::{ScanRequest, ids::sheet_key};
use serde_json::{Value, json};

use crate::CadApp;

pub const DIALOG: &str = "ubplot";
pub(crate) use dialog::show as dialog;

#[derive(Clone, Default)]
struct WindowPick {
    doc_uid: u64,
    job_id: String,
    sheet_key: String,
}

const PICKING: &str = "ubplot_picking_window";

pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(result) = jobs_panel::route(app, id, params) {
        return Some(result);
    }
    if !matches!(id, "ubplot" | "ubp") || !params.is_null() {
        return None;
    }
    let result = app.session.start("ubplot").map(|()| Value::Null).map_err(|error| error.to_string());
    if let Err(error) = &result {
        app.session.echo(error.clone());
    }
    Some(result)
}

pub(crate) fn resume(app: &mut CadApp, ctx: &egui::Context) {
    if app.session.running.is_some() {
        return;
    }
    let uid = app.session.state().ok().map(|state| state.uid);
    if let Some(pick) = ctx.data_mut(|data| data.remove_temp::<WindowPick>(egui::Id::new(PICKING))) {
        let window = app.session.plot_window.take();
        let last = app.session.ubplot.last.as_ref().filter(|last| Some(last.doc_uid) == uid);
        if let Some(last) = last
            && last.doc_uid == pick.doc_uid
            && last.plan.job_id == pick.job_id
            && last.plan.sheets.iter().any(|sheet| sheet_key(sheet) == pick.sheet_key)
        {
            if let Some(window) = window {
                let mut request = last.request.clone();
                request.overrides.entry(pick.sheet_key).or_default().window = Some([[window.min.x, window.min.y], [window.max.x, window.max.y]]);
                let _ = scan(app, &request);
            }
            app.ui.dialog = Some(DIALOG.into());
        }
    }
    if app.session.ubplot.review {
        app.session.ubplot.review = false;
        if app.session.ubplot.last.as_ref().is_some_and(|last| Some(last.doc_uid) == uid) {
            app.ui.dialog = Some(DIALOG.into());
        }
    }
}

fn scan(app: &mut CadApp, request: &ScanRequest) -> Result<Value, String> {
    let params = serde_json::to_value(request).map_err(|error| error.to_string())?;
    app.run("ubplot.scan", params)
}

fn pick_window(app: &mut CadApp, ctx: &egui::Context, key: String) {
    let Some(last) = &app.session.ubplot.last else { return };
    let pick = WindowPick { doc_uid: last.doc_uid, job_id: last.plan.job_id.clone(), sheet_key: key };
    app.session.plot_window = None;
    app.start("plotwindow");
    if app.session.running.as_ref().is_some_and(|running| running.id == "plotwindow") {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(PICKING), pick));
        app.ui.dialog = None;
    }
}

fn rescan(app: &mut CadApp) -> Result<Value, String> {
    app.run("ubplot.scan", json!({"mode": "last"}))
}
